//! A sum that no longer fits its result type is an error, not a value the
//! engine cannot read back (rule S12, issue #213).

mod common;

use opengrid_conformance::block_on;
use opengrid_datasource::{DataSource, wire};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::load_json;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema};

fn sum_of(column: DataType, rows: &str) -> Result<opengrid_datasource::QueryResult, String> {
    let schema = Schema::new(vec![Field::new(FieldName::new("n").unwrap(), column)]);
    let table = load_json(rows.as_bytes(), &schema).expect("the rows load");
    let query: Query = opengrid_json::from_str(
        r#"{"source":"t","select":[],"aggregate":[{"fn":"sum","as":"total","field":"n"}]}"#,
    )
    .expect("a query");
    let validated = query
        .validate(&schema, &Limits::default())
        .map_err(|error| error.to_string())?;
    block_on(LocalDataSource::new(table).execute(validated)).map_err(|error| error.to_string())
}

#[test]
fn an_int64_sum_over_the_limit_is_an_error() {
    let rows = format!(r#"[{{"n":{}}},{{"n":1}}]"#, i64::MAX);
    let error = sum_of(DataType::Int64, &rows).unwrap_err();
    assert!(error.contains("does not fit"), "{error}");

    // One less fits, and is the exact value.
    let rows = format!(r#"[{{"n":{}}},{{"n":0}}]"#, i64::MAX - 1);
    let result = sum_of(DataType::Int64, &rows).expect("fits");
    assert_eq!(result.row_count(), 1);
}

/// `i128` holds 39 digits, the result type 38: a sum of two 38-digit values can
/// pass the first and break the second, and the answer then could not be read
/// back from either wire form.
#[test]
fn a_decimal_sum_of_39_digits_is_an_error_not_an_unreadable_value() {
    let decimal = DataType::decimal(38, 0).unwrap();
    // 6e37 + 6e37 = 1.2e38: 39 digits, and still below i128::MAX (1.7e38).
    let big = "60000000000000000000000000000000000000";
    let rows = format!(r#"[{{"n":"{big}"}},{{"n":"{big}"}}]"#);
    let error = sum_of(decimal, &rows).unwrap_err();
    assert!(error.contains("does not fit"), "{error}");

    // 4e37 + 4e37 = 8e37 has 38 digits and comes back through the wire.
    let four = "40000000000000000000000000000000000000";
    let rows = format!(r#"[{{"n":"{four}"}},{{"n":"{four}"}}]"#);
    let result = sum_of(decimal, &rows).expect("fits");
    let back = wire::result_from_json(&wire::result_to_json(&result)).expect("reads back");
    assert_eq!(back.row_count(), 1);
}
