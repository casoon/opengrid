//! A `decimal` column of up to 18 digits stores its coefficients as `i64`, a wider one as `i128`
//! (issue #450). What a query answers must not depend on that: the same coefficients, at the ends
//! of 18 and 19 digits, filter, sort, aggregate and group as the plain numbers do. The expected
//! answers are worked out here from the plain `Vec<Option<i128>>`, not by the engine.

use opengrid_engine::Table;
use opengrid_engine::execute::execute;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Decimal, Field, FieldName, Schema, Value};

fn schema(precision: u8) -> Schema {
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(
            FieldName::new("d").unwrap(),
            DataType::Decimal {
                precision,
                scale: 2,
            },
        ),
    ])
}

/// Coefficients around zero and at the ends of `digits` digits, NULLs and ties among them.
fn numbers(digits: u32) -> Vec<Option<i128>> {
    let top = 10i128.pow(digits) - 1;
    let mut values = vec![
        Some(0),
        Some(1234),
        None,
        Some(-1),
        Some(top),
        Some(-top),
        Some(top - 1),
        Some(1234),
        Some(i128::from(i64::MAX).min(top)),
        Some(i128::from(i64::MIN).max(-top)),
        None,
        Some(0),
    ];
    for i in 0..300 {
        values.push(Some(i128::from(i % 13) - 6));
    }
    values
}

fn table(precision: u8, numbers: &[Option<i128>]) -> Table {
    let ids: Vec<Value> = (0..numbers.len() as i64).map(Value::Int64).collect();
    let cells: Vec<Value> = numbers
        .iter()
        .map(|number| number.map_or(Value::Null, |n| Value::Decimal(Decimal::new(n, 2))))
        .collect();
    Table::from_values(&schema(precision), &[ids, cells]).unwrap()
}

fn run(precision: u8, table: &Table, json: &str) -> Vec<Vec<Value>> {
    let query: Query = opengrid_json::from_str(json).expect("a well-formed query");
    let query = query
        .validate(&schema(precision), &Limits::default())
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

/// The text of a coefficient at scale 2: `-1234` is `-12.34`.
fn text(value: i128) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let magnitude = value.unsigned_abs();
    format!("{sign}{}.{:02}", magnitude / 100, magnitude % 100)
}

/// Both sides of the 18-digit line: the narrow storage and the wide one.
const CASES: [(u8, u32); 3] = [(12, 12), (18, 18), (19, 19)];

#[test]
fn the_storage_follows_the_precision() {
    for (precision, narrow) in [(12, true), (18, true), (19, false), (38, false)] {
        let table = table(precision, &[Some(1), None, Some(-3)]);
        let bytes = table.column_at(1).byte_size();
        // Three coefficients and the bitmap word of the NULL.
        let want = if narrow { 3 * 8 } else { 3 * 16 } + 8;
        assert_eq!(bytes, want, "precision {precision}");
    }
}

#[test]
fn filters_answer_as_the_numbers_do() {
    for (precision, digits) in CASES {
        let numbers = numbers(digits);
        let table = table(precision, &numbers);
        let top = 10i128.pow(digits) - 1;
        for probe in [0, 1234, -1, 3, top, -top, top - 1] {
            for (op, keep) in [
                ("eq", (|n: i128, p: i128| n == p) as fn(i128, i128) -> bool),
                ("ne", |n, p| n != p),
                ("lt", |n, p| n < p),
                ("lte", |n, p| n <= p),
                ("gt", |n, p| n > p),
                ("gte", |n, p| n >= p),
            ] {
                let got = ids(&run(
                    precision,
                    &table,
                    &format!(
                        r#"{{"source":"t","select":["id"],"filter":{{"field":"d","op":"{op}","value":"{}"}},"sort":[{{"field":"id"}}]}}"#,
                        text(probe)
                    ),
                ));
                let want: Vec<i64> = numbers
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.is_some_and(|n| keep(n, probe)))
                    .map(|(at, _)| at as i64)
                    .collect();
                assert_eq!(got, want, "precision {precision}: d {op} {probe}");
            }
        }
        let members = [0, 1234, top, -top];
        let list: Vec<String> = members
            .iter()
            .map(|m| format!("\"{}\"", text(*m)))
            .collect();
        let got = ids(&run(
            precision,
            &table,
            &format!(
                r#"{{"source":"t","select":["id"],"filter":{{"field":"d","op":"in","value":[{}]}},"sort":[{{"field":"id"}}]}}"#,
                list.join(",")
            ),
        ));
        let want: Vec<i64> = numbers
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_some_and(|n| members.contains(&n)))
            .map(|(at, _)| at as i64)
            .collect();
        assert_eq!(got, want, "precision {precision}: d in [...]");
    }
}

#[test]
fn sorts_answer_as_the_numbers_do() {
    for (precision, digits) in CASES {
        let numbers = numbers(digits);
        let table = table(precision, &numbers);
        for desc in [false, true] {
            let got = ids(&run(
                precision,
                &table,
                &format!(
                    r#"{{"source":"t","select":["id","d"],"sort":[{{"field":"d","direction":"{}","nulls":"last"}},{{"field":"id"}}]}}"#,
                    if desc { "desc" } else { "asc" }
                ),
            ));
            let mut want: Vec<usize> = (0..numbers.len()).collect();
            // NULLs last in both directions; ties by id.
            want.sort_by(|a, b| match (numbers[*a], numbers[*b]) {
                (None, None) => a.cmp(b),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(x), Some(y)) => (if desc { y.cmp(&x) } else { x.cmp(&y) }).then(a.cmp(b)),
            });
            let want: Vec<i64> = want.into_iter().map(|at| at as i64).collect();
            assert_eq!(got, want, "precision {precision}, desc {desc}");
        }
    }
}

#[test]
fn aggregates_and_groups_answer_as_the_numbers_do() {
    for (precision, digits) in CASES {
        let numbers = numbers(digits);
        let table = table(precision, &numbers);
        let present: Vec<i128> = numbers.iter().flatten().copied().collect();
        let rows = run(
            precision,
            &table,
            r#"{"source":"t","select":[],"aggregate":[{"fn":"sum","field":"d","as":"s"},{"fn":"min","field":"d","as":"lo"},{"fn":"max","field":"d","as":"hi"},{"fn":"count","field":"d","as":"c"}]}"#,
        );
        let (Value::Decimal(sum), Value::Decimal(lo), Value::Decimal(hi), Value::Int64(count)) =
            (&rows[0][0], &rows[0][1], &rows[0][2], &rows[0][3])
        else {
            panic!("a sum, a min, a max and a count, got {:?}", rows[0]);
        };
        assert_eq!(
            sum.value(),
            present.iter().sum::<i128>(),
            "sum, {precision}"
        );
        assert_eq!(
            lo.value(),
            *present.iter().min().unwrap(),
            "min, {precision}"
        );
        assert_eq!(
            hi.value(),
            *present.iter().max().unwrap(),
            "max, {precision}"
        );
        assert_eq!(*count, present.len() as i64);

        // One group per distinct value and one for NULL.
        let groups = run(
            precision,
            &table,
            r#"{"source":"t","select":["d"],"group":["d"],"aggregate":[{"fn":"count","as":"c"}],"sort":[{"field":"d"}]}"#,
        );
        let mut want: std::collections::BTreeMap<i128, i64> = Default::default();
        for number in present {
            *want.entry(number).or_default() += 1;
        }
        let nulls = numbers.iter().filter(|n| n.is_none()).count() as i64;
        let mut want: Vec<Vec<Value>> = want
            .into_iter()
            .map(|(n, c)| vec![Value::Decimal(Decimal::new(n, 2)), Value::Int64(c)])
            .collect();
        want.push(vec![Value::Null, Value::Int64(nulls)]);
        assert_eq!(groups, want, "groups, {precision}");
    }
}
