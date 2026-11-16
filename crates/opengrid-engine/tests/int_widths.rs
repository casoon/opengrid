//! An `int64` column stores its values as wide as they need (issue #447). What a query answers
//! must not depend on that: the same integers at every width — a byte, two, four, eight — filter,
//! sort, aggregate and group as the integers do. The expected answers are worked out here from the
//! plain `Vec<Option<i64>>`, not by the engine.

use std::collections::BTreeMap;

use opengrid_engine::Table;
use opengrid_engine::execute::execute;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

fn schema() -> Schema {
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("n").unwrap(), DataType::Int64),
    ])
}

/// Values at and around every width: bytes, shorts, ints, longs, NULLs among them, ties.
fn numbers() -> Vec<Option<i64>> {
    let mut values = vec![
        Some(0),
        Some(5),
        None,
        Some(-7),
        Some(127),
        Some(-128),
        Some(128),
        Some(-129),
        Some(5),
        Some(32_767),
        Some(32_768),
        Some(-32_768),
        Some(-32_769),
        None,
        Some(40_000),
        Some(i64::from(i32::MAX)),
        Some(i64::from(i32::MIN)),
        Some(i64::from(i32::MAX) + 1),
        Some(i64::from(i32::MIN) - 1),
        Some(1 << 40),
        Some(-(1 << 40)),
        Some(-7),
        Some(0),
    ];
    // More of the same, so that a page is a small part of the table too.
    for i in 0..400 {
        values.push(Some((i % 17) - 8));
    }
    values
}

fn table(numbers: &[Option<i64>]) -> Table {
    let rows: Vec<Value> = (0..numbers.len() as i64).map(Value::Int64).collect();
    let cells: Vec<Value> = numbers
        .iter()
        .map(|number| number.map_or(Value::Null, Value::Int64))
        .collect();
    Table::from_values(&schema(), &[rows, cells]).unwrap()
}

fn run(table: &Table, json: &str) -> Vec<Vec<Value>> {
    let query: Query = opengrid_json::from_str(json).expect("a well-formed query");
    let query = query
        .validate(&schema(), &Limits::default())
        .expect("a valid query");
    let result = execute(table, &query).expect("the engine answers");
    (0..result.table.num_rows())
        .map(|row| {
            (0..result.table.schema().len())
                .map(|column| result.table.column_at(column).value(row))
                .collect()
        })
        .collect()
}

fn ids(rows: &[Vec<Value>]) -> Vec<i64> {
    rows.iter()
        .map(|row| match row[0] {
            Value::Int64(id) => id,
            ref other => panic!("an id, got {other:?}"),
        })
        .collect()
}

#[test]
fn the_columns_are_as_wide_as_their_values() {
    let narrow = table(&[Some(1), Some(-2), None, Some(100)]);
    let wide = table(&[Some(1), Some(-2), None, Some(1 << 40)]);
    // The ids 0..3 and the numbers take a byte each in the first, eight in the second's numbers.
    assert_eq!(narrow.byte_size(), 4 + 4 + 8);
    assert_eq!(wide.byte_size(), 4 + 4 * 8 + 8);
}

#[test]
fn filters_answer_as_the_integers_do() {
    let numbers = numbers();
    let table = table(&numbers);
    let probes = [
        0,
        5,
        -7,
        127,
        -128,
        128,
        -129,
        32_767,
        32_768,
        -32_769,
        40_000,
        i64::from(i32::MAX) + 1,
        1 << 40,
        3,
        -1 << 41,
    ];
    for probe in probes {
        for (op, keep) in [
            ("eq", (|n: i64, p: i64| n == p) as fn(i64, i64) -> bool),
            ("ne", |n, p| n != p),
            ("lt", |n, p| n < p),
            ("lte", |n, p| n <= p),
            ("gt", |n, p| n > p),
            ("gte", |n, p| n >= p),
        ] {
            let got = ids(&run(
                &table,
                &format!(
                    r#"{{"source":"t","select":["id"],"filter":{{"field":"n","op":"{op}","value":{probe}}},"sort":[{{"field":"id"}}]}}"#
                ),
            ));
            let want: Vec<i64> = numbers
                .iter()
                .enumerate()
                .filter(|(_, number)| number.is_some_and(|number| keep(number, probe)))
                .map(|(row, _)| row as i64)
                .collect();
            assert_eq!(got, want, "n {op} {probe}");
        }
    }
    // A list of members.
    let members = [5, -7, 128, 40_000, 1 << 40, -33];
    let got = ids(&run(
        &table,
        &format!(
            r#"{{"source":"t","select":["id"],"filter":{{"field":"n","op":"in","value":{members:?}}},"sort":[{{"field":"id"}}]}}"#
        ),
    ));
    let want: Vec<i64> = numbers
        .iter()
        .enumerate()
        .filter(|(_, number)| number.is_some_and(|number| members.contains(&number)))
        .map(|(row, _)| row as i64)
        .collect();
    assert_eq!(got, want);
}

#[test]
fn sorts_answer_as_the_integers_do() {
    let numbers = numbers();
    let table = table(&numbers);
    for (direction, nulls) in [
        ("asc", "first"),
        ("asc", "last"),
        ("desc", "first"),
        ("desc", "last"),
    ] {
        // The whole table, the start of it, and a page deep in it.
        for (offset, limit) in [(0, 1000), (0, 9), (200, 25), (400, 100)] {
            let got = ids(&run(
                &table,
                &format!(
                    r#"{{"source":"t","select":["id","n"],"sort":[{{"field":"n","direction":"{direction}","nulls":"{nulls}"}}],"offset":{offset},"limit":{limit}}}"#
                ),
            ));
            let mut want: Vec<usize> = (0..numbers.len()).collect();
            want.sort_by(|a, b| {
                let order = match (numbers[*a], numbers[*b]) {
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, _) => {
                        return if nulls == "first" {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Greater
                        };
                    }
                    (_, None) => {
                        return if nulls == "first" {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Less
                        };
                    }
                    (Some(a), Some(b)) => a.cmp(&b),
                };
                (if direction == "desc" {
                    order.reverse()
                } else {
                    order
                })
                .then(a.cmp(b))
            });
            let want: Vec<i64> = want
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|row| row as i64)
                .collect();
            assert_eq!(
                got, want,
                "{direction}, nulls {nulls}, from {offset}, {limit} rows"
            );
        }
    }
}

#[test]
fn aggregates_and_groups_answer_as_the_integers_do() {
    let numbers = numbers();
    let table = table(&numbers);
    let present: Vec<i64> = numbers.iter().flatten().copied().collect();
    let rows = run(
        &table,
        r#"{"source":"t","select":[],"aggregate":[{"fn":"sum","field":"n","as":"s"},{"fn":"min","field":"n","as":"lo"},{"fn":"max","field":"n","as":"hi"},{"fn":"count","field":"n","as":"c"}]}"#,
    );
    assert_eq!(
        rows,
        vec![vec![
            Value::Int64(present.iter().sum()),
            Value::Int64(*present.iter().min().unwrap()),
            Value::Int64(*present.iter().max().unwrap()),
            Value::Int64(present.len() as i64),
        ]]
    );
    // Groups: one row per distinct value and one for NULL, with their counts.
    let groups = run(
        &table,
        r#"{"source":"t","select":["n"],"group":["n"],"aggregate":[{"fn":"count","as":"c"}],"sort":[{"field":"n"}]}"#,
    );
    let mut want: BTreeMap<Option<i64>, i64> = BTreeMap::new();
    for number in &numbers {
        *want.entry(*number).or_default() += 1;
    }
    let want: Vec<Vec<Value>> = want
        .into_iter()
        .map(|(number, count)| {
            vec![
                number.map_or(Value::Null, Value::Int64),
                Value::Int64(count),
            ]
        })
        .collect();
    // NULL is the last group in the default order of a key; the reference put `None` first.
    let mut want = want;
    want.rotate_left(1);
    assert_eq!(groups, want);
}

#[test]
fn the_ends_of_i64_sort_and_compare() {
    let numbers = vec![
        Some(i64::MAX),
        Some(i64::MIN),
        Some(0),
        None,
        Some(i64::MAX - 1),
        Some(i64::MIN + 1),
    ];
    let table = table(&numbers);
    let got = ids(&run(
        &table,
        r#"{"source":"t","select":["id","n"],"sort":[{"field":"n","direction":"desc","nulls":"last"}]}"#,
    ));
    assert_eq!(got, vec![0, 4, 2, 5, 1, 3]);
    let got = ids(&run(
        &table,
        &format!(
            r#"{{"source":"t","select":["id"],"filter":{{"field":"n","op":"eq","value":{}}},"sort":[{{"field":"id"}}]}}"#,
            i64::MIN
        ),
    ));
    assert_eq!(got, vec![1]);
}
