//! Property tests — plan point 08, step 5.
//!
//! The conformance suite pins the rules of the query model against one data set.
//! These tests pin the invariants that have to hold for **any** data set: the
//! group sums add up to the global sum, the group counts add up to the row
//! count, and every group carries exactly the sum of its own rows. The data is
//! generated, read through ingest, and the expectation is computed here in
//! plain Rust — not by the engine under test.

mod common;

use std::collections::BTreeMap;

use opengrid_arrow_engine::execute::execute;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};
use proptest::prelude::*;

/// A key is NULL, or one of four strings — the empty string among them, so the
/// tests also cover rule S14 (an empty string is not a NULL).
const KEYS: [&str; 4] = ["", "a", "b", "c"];

fn field(name: &str) -> FieldName {
    FieldName::new(name).expect("a valid identifier")
}

/// The generated data: a key and an amount, each possibly NULL.
fn rows() -> impl Strategy<Value = Vec<(Option<usize>, Option<i64>)>> {
    prop::collection::vec(
        (
            prop::option::of(0..KEYS.len()),
            prop::option::of(-500i64..=500),
        ),
        0..120,
    )
}

/// The CSV the engine reads, with `\N` for NULL.
///
/// With `decimal` the amount is written as a two-place number and the generated
/// value counts *cents*, so the expectation stays exact.
fn csv_of(rows: &[(Option<usize>, Option<i64>)], decimal: bool) -> String {
    let mut text = String::from("grp,amount\n");
    for (key, amount) in rows {
        let key = match key {
            None => "\\N".to_owned(),
            Some(index) => KEYS[*index].to_owned(),
        };
        let amount = match amount {
            None => "\\N".to_owned(),
            Some(value) if decimal => format!(
                "{}{}.{:02}",
                if *value < 0 { "-" } else { "" },
                value.abs() / 100,
                value.abs() % 100
            ),
            Some(value) => value.to_string(),
        };
        text.push_str(&key);
        text.push(',');
        text.push_str(&amount);
        text.push('\n');
    }
    text
}

fn schema(data_type: DataType) -> Schema {
    Schema::new(vec![
        Field::new(field("grp"), DataType::Utf8),
        Field::new(field("amount"), data_type),
    ])
}

/// `group by grp`, `sum(amount)`, `count(amount)` and `count(*)`.
const QUERY: &str = r#"{"source":"orders","select":["grp","total","n","rows"],
    "group":["grp"],
    "aggregate":[{"field":"amount","fn":"sum","as":"total"},
                 {"field":"amount","fn":"count","as":"n"},
                 {"fn":"count","as":"rows"}]}"#;

/// Runs the query against generated data and returns the result rows.
fn answered(
    rows: &[(Option<usize>, Option<i64>)],
    data_type: DataType,
    decimal: bool,
) -> Vec<Vec<Value>> {
    let schema = schema(data_type);
    let batches = load_csv(
        csv_of(rows, decimal).as_bytes(),
        &schema,
        CsvOptions::default(),
    )
    .expect("the generated data loads");
    let query: Query = serde_json::from_str(QUERY).expect("the query parses");
    let query = query
        .validate(&schema, &Limits::default())
        .expect("the query validates");
    let result = execute(&batches, &query).expect("the engine answers");
    assert_eq!(result.schema.fields().len(), 4);
    common::decode(&result.batches)
}

/// The key a group is grouped by, as the expectation sees it.
fn expected_key(key: Option<usize>) -> Option<String> {
    key.map(|index| KEYS[index].to_owned())
}

/// `(sum, count(amount), count(*))` per key, computed in plain Rust.
type Expected = BTreeMap<Option<String>, (i128, i64, i64)>;

fn expectation(rows: &[(Option<usize>, Option<i64>)]) -> Expected {
    let mut expected: Expected = BTreeMap::new();
    for (key, amount) in rows {
        let entry = expected.entry(expected_key(*key)).or_insert((0, 0, 0));
        if let Some(amount) = amount {
            entry.0 += *amount as i128;
            entry.1 += 1;
        }
        entry.2 += 1;
    }
    expected
}

/// The result row as `(key, sum, count(amount), count(*))`.
fn result_row(row: &[Value], data_type: &DataType) -> (Option<String>, i128, i64, i64) {
    let key = match &row[0] {
        Value::Null => None,
        Value::Utf8(value) => Some(value.clone()),
        other => panic!("key {other:?}"),
    };
    let sum = match (&row[1], data_type) {
        (Value::Null, _) => 0,
        (Value::Int64(value), DataType::Int64) => *value as i128,
        (Value::Decimal(value), DataType::Decimal { .. }) => value.value(),
        (other, _) => panic!("sum {other:?}"),
    };
    let n = match &row[2] {
        Value::Int64(value) => *value,
        other => panic!("count {other:?}"),
    };
    let rows = match &row[3] {
        Value::Int64(value) => *value,
        other => panic!("count(*) {other:?}"),
    };
    (key, sum, n, rows)
}

proptest! {
    // Integration tests of a crate have no `src/main.rs` root for proptest's
    // failure files; the seeds are not persisted (the assertion output is what
    // matters here).
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Int64: `sum(Int64)` stays Int64 (rule S12), and the parts add up.
    #[test]
    fn the_group_sums_of_an_int64_column_add_up(rows in rows()) {
        let data_type = DataType::Int64;
        let answered = answered(&rows, data_type, false);
        let expected = expectation(&rows);

        prop_assert_eq!(answered.len(), expected.len(), "one row per group");
        let mut total = 0i128;
        let mut counted = 0i64;
        let mut all = 0i64;
        for row in &answered {
            let (key, sum, n, rows) = result_row(row, &data_type);
            let want = expected.get(&key).expect("the key was generated");
            prop_assert_eq!(sum, want.0, "sum of {:?}", key);
            prop_assert_eq!(n, want.1, "count(amount) of {:?}", key);
            prop_assert_eq!(rows, want.2, "count(*) of {:?}", key);
            total += sum;
            counted += n;
            all += rows;
        }

        let global: i128 = rows
            .iter()
            .filter_map(|(_, amount)| amount.map(i128::from))
            .sum();
        let non_null = rows.iter().filter(|(_, amount)| amount.is_some()).count() as i64;
        prop_assert_eq!(total, global, "the group sums add up to the global sum");
        prop_assert_eq!(counted, non_null, "the counts add up to the non-NULL values");
        prop_assert_eq!(all, rows.len() as i64, "count(*) adds up to the row count");
    }

    /// Decimal: `sum(Decimal(p, s))` widens to `Decimal(38, s)` and stays exact.
    #[test]
    fn the_group_sums_of_a_decimal_column_stay_exact(rows in rows()) {
        let data_type = DataType::decimal(DataType::MAX_DECIMAL_PRECISION, 2).unwrap();
        let answered = answered(&rows, data_type, true);
        let expected = expectation(&rows);

        prop_assert_eq!(answered.len(), expected.len(), "one row per group");
        let mut total = 0i128;
        let mut counted = 0i64;
        for row in &answered {
            let (key, sum, n, _) = result_row(row, &data_type);
            let want = expected.get(&key).expect("the key was generated");
            prop_assert_eq!(sum, want.0, "sum of {:?}", key);
            prop_assert_eq!(n, want.1, "count(amount) of {:?}", key);
            total += sum;
            counted += n;
        }

        let global: i128 = rows
            .iter()
            .filter_map(|(_, amount)| amount.map(i128::from))
            .sum();
        prop_assert_eq!(total, global, "the decimal sums add up exactly");
        prop_assert_eq!(
            counted,
            rows.iter().filter(|(_, amount)| amount.is_some()).count() as i64
        );
    }

    /// Rule S10: a NULL key is its own group, and it is not the empty string
    /// (rule S14) — whatever the data mixes.
    #[test]
    fn the_null_group_is_not_the_empty_string(rows in rows()) {
        let data_type = DataType::Int64;
        let answered = answered(&rows, data_type, false);
        let expected = expectation(&rows);

        let null_group = answered.iter().any(|row| row[0] == Value::Null);
        let empty_group = answered
            .iter()
            .any(|row| row[0] == Value::Utf8(String::new()));
        prop_assert_eq!(null_group, rows.iter().any(|(key, _)| key.is_none()));
        prop_assert_eq!(
            empty_group,
            rows.iter().any(|(key, _)| *key == Some(0)),
            "the empty string is a key of its own"
        );
        prop_assert_eq!(answered.len(), expected.len());
    }
}
