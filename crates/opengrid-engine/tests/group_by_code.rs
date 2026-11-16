//! Grouping by text columns that are dictionaries finds the group of a row by its codes (issue
//! #465). What a query answers must not depend on that: the same rows grouped through the codes, as
//! a flat column through hashed keys, and as the plain values group, in the same order. The
//! expected answers are worked out here from plain values, not by the engine, and the flat twin of
//! a table is made by `Table::slice`, which gathers into flat buffers.

use std::collections::BTreeMap;

use opengrid_columns::{Strs, Values};
use opengrid_engine::Table;
use opengrid_engine::execute::execute;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

fn schema() -> Schema {
    let text = |name: &str| Field::new(FieldName::new(name).unwrap(), DataType::Utf8);
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        text("a"),
        text("b"),
        text("c"),
        Field::new(FieldName::new("n").unwrap(), DataType::Int64),
    ])
}

/// One row: the three texts and a number, each possibly NULL.
type Row = (Option<String>, Option<String>, Option<String>, Option<i64>);

/// Texts with ties and the awkward ones: the empty text, a NUL inside, a text that starts another.
fn texts(distinct: usize) -> Vec<String> {
    let mut texts = vec![
        "DE".to_owned(),
        String::new(),
        "a\0b".to_owned(),
        "a".to_owned(),
        "a\0".to_owned(),
        "日本語".to_owned(),
    ];
    for i in 0..distinct.saturating_sub(texts.len()) {
        texts.push(format!("text {i}"));
    }
    texts.truncate(distinct);
    texts
}

fn rows(count: usize, distinct: usize) -> Vec<Row> {
    let texts = texts(distinct);
    let pick = |row: usize, step: usize, nulls: usize| {
        if row % nulls == 3 {
            None
        } else {
            Some(texts[(row * step + row / 7) % texts.len()].clone())
        }
    };
    (0..count)
        .map(|row| {
            (
                pick(row, 3, 11),
                pick(row, 5, 17),
                pick(row, 7, 19),
                if row % 9 == 2 {
                    None
                } else {
                    Some((row % 5) as i64 - 2)
                },
            )
        })
        .collect()
}

fn table(rows: &[Row]) -> Table {
    let text = |get: fn(&Row) -> &Option<String>| -> Vec<Value> {
        rows.iter()
            .map(|row| get(row).clone().map_or(Value::Null, Value::Utf8))
            .collect()
    };
    let ids: Vec<Value> = (0..rows.len() as i64).map(Value::Int64).collect();
    let numbers: Vec<Value> = rows
        .iter()
        .map(|row| row.3.map_or(Value::Null, Value::Int64))
        .collect();
    Table::from_values(
        &schema(),
        &[
            ids,
            text(|row| &row.0),
            text(|row| &row.1),
            text(|row| &row.2),
            numbers,
        ],
    )
    .unwrap()
}

fn flat(table: &Table) -> Table {
    table.slice(0, table.num_rows())
}

fn is_dictionary(table: &Table, column: usize) -> bool {
    matches!(
        table.column_at(column).values(),
        Values::Utf8(Strs::Dict { .. })
    )
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

/// Groups `rows` by `keys` (columns 0 to 3 of a [`Row`]; a NULL is a key of its own) in the order
/// the first row of each group appears, with the count and the sum of `n` (`None` over no number).
fn reference(rows: &[Row], keys: &[usize]) -> Vec<Vec<Value>> {
    type Cell = Option<String>;
    let key_of = |row: &Row| -> Vec<Cell> {
        keys.iter()
            .map(|key| match key {
                0 => row.0.clone(),
                1 => row.1.clone(),
                2 => row.2.clone(),
                _ => row.3.map(|n| n.to_string()),
            })
            .collect()
    };
    let mut order: Vec<Vec<Cell>> = Vec::new();
    let mut groups: BTreeMap<Vec<Cell>, (i64, Option<i64>)> = BTreeMap::new();
    for row in rows {
        let key = key_of(row);
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        let group = groups.entry(key).or_insert((0, None));
        group.0 += 1;
        if let Some(n) = row.3 {
            group.1 = Some(group.1.unwrap_or(0) + n);
        }
    }
    order
        .into_iter()
        .map(|key| {
            let (count, sum) = groups[&key];
            let mut out: Vec<Value> = key
                .iter()
                .zip(keys)
                .map(|(cell, key)| match (cell, key) {
                    (None, _) => Value::Null,
                    (Some(n), 3) => Value::Int64(n.parse().unwrap()),
                    (Some(text), _) => Value::Utf8(text.clone()),
                })
                .collect();
            out.push(Value::Int64(count));
            out.push(sum.map_or(Value::Null, Value::Int64));
            out
        })
        .collect()
}

fn grouped(table: &Table, fields: &[&str]) -> Vec<Vec<Value>> {
    let list = fields
        .iter()
        .map(|field| format!("\"{field}\""))
        .collect::<Vec<_>>()
        .join(",");
    run(
        table,
        &format!(
            r#"{{"source":"t","select":[{list}],"group":[{list}],"aggregate":[{{"fn":"count","as":"cnt"}},{{"fn":"sum","field":"n","as":"total"}}]}}"#
        ),
    )
}

/// The groups of `fields` through the codes, through hashed keys and by the plain values.
fn check(rows: &[Row], fields: &[&str], keys: &[usize]) {
    let dictionary = table(rows);
    let twin = flat(&dictionary);
    let want = reference(rows, keys);
    let got = grouped(&dictionary, fields);
    assert_eq!(got, want, "grouped by {fields:?}");
    assert_eq!(
        grouped(&twin, fields),
        want,
        "the flat twin, grouped by {fields:?}"
    );
}

#[test]
fn the_few_texts_are_dictionaries_and_their_twin_is_flat() {
    let table = table(&rows(500, 12));
    assert!([1, 2, 3].iter().all(|at| is_dictionary(&table, *at)));
    let twin = flat(&table);
    assert!([1, 2, 3].iter().all(|at| !is_dictionary(&twin, *at)));
}

#[test]
fn one_dictionary_key_groups_as_the_plain_values_do() {
    let rows = rows(800, 12);
    check(&rows, &["a"], &[0]);
    check(&rows, &["b"], &[1]);
}

#[test]
fn two_and_three_dictionary_keys_group_as_the_plain_values_do() {
    let rows = rows(900, 12);
    check(&rows, &["a", "b"], &[0, 1]);
    check(&rows, &["b", "a"], &[1, 0]);
    check(&rows, &["a", "b", "c"], &[0, 1, 2]);
}

#[test]
fn a_dictionary_key_beside_a_number_groups_as_the_plain_values_do() {
    let rows = rows(900, 12);
    check(&rows, &["a", "n"], &[0, 3]);
    check(&rows, &["n", "a"], &[3, 0]);
    check(&rows, &["n"], &[3]);
}

#[test]
fn null_and_the_empty_text_are_groups_of_their_own() {
    let rows = rows(800, 12);
    let got = grouped(&table(&rows), &["a"]);
    assert!(got.iter().any(|row| row[0] == Value::Null));
    assert!(got.iter().any(|row| row[0] == Value::Utf8(String::new())));
    // A text with a NUL in it is not the text before the NUL.
    assert!(
        got.iter()
            .any(|row| row[0] == Value::Utf8("a\0b".to_owned()))
    );
    assert!(got.iter().any(|row| row[0] == Value::Utf8("a".to_owned())));
}

#[test]
fn more_combinations_than_the_code_table_holds_group_as_the_plain_values_do() {
    // 70 texts in each of three columns: 71 slots each, 357,911 together, past the table.
    let rows = rows(3000, 70);
    assert!(is_dictionary(&table(&rows), 1));
    check(&rows, &["a", "b", "c"], &[0, 1, 2]);
    // And two of them, which fit.
    check(&rows, &["a", "b"], &[0, 1]);
}

#[test]
fn an_empty_input_has_no_group() {
    let rows = rows(60, 4);
    let got = run(
        &table(&rows),
        r#"{"source":"t","select":["a"],"filter":{"field":"id","op":"gt","value":100000},"group":["a"],"aggregate":[{"fn":"count","as":"cnt"}]}"#,
    );
    assert!(got.is_empty());
}
