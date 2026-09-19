//! The `DataSource` adapter over the local engine (point 09).
//!
//! The suite runner next door proves the answers are *right*; this file pins down
//! the adapter's own contract — the schema it reports, the column-oriented result
//! (E14), the empty page and the source without data.

mod common;

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::execute::execute;
use opengrid_conformance::{RowOrder, Table, block_on, compare};
use opengrid_datasource::{DataSource, DataSourceCapabilities, DataSourceError};
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::{Schema, Value};

/// A source without a single batch has no schema and nothing to answer with — an
/// empty *table* is one batch without rows, which is not the same thing.
#[test]
fn a_source_without_batches_is_an_error() {
    assert_eq!(
        LocalDataSource::new(Vec::new()).err(),
        Some(DataSourceError::NoData)
    );
}

/// The schema comes from the data: ingest built it against exactly this schema.
///
/// **Materialized**, because a derived column stops being derived once ingest has
/// computed it (point 54): the batch really holds those values, and the source
/// reports what it holds. Where they came from is the loader's business.
#[test]
fn the_schema_comes_from_the_batches() {
    let source = LocalDataSource::new(common::csv_batches()).expect("the dataset has batches");
    let reported = block_on(source.schema()).expect("a source with data has a schema");

    assert_eq!(reported, common::schema().materialized());
    assert!(!reported.has_derived());
    assert!(
        common::schema().has_derived(),
        "the declared schema is the one with the derivations"
    );
}

/// Everything runs in WASM, so the local source claims every capability
/// (point 09, step 3).
#[test]
fn every_capability_is_reported() {
    let source = LocalDataSource::new(common::csv_batches()).expect("the dataset has batches");
    assert_eq!(source.capabilities(), DataSourceCapabilities::ALL);
}

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("the query parses");
    query
        .validate(schema, &Limits::default())
        .expect("the query validates")
}

/// The filter value the tests below use, spelled once.
fn de() -> Value {
    Value::Utf8("DE".to_owned())
}

/// The page the grid asks for: column-oriented (E14), in schema order, aligned —
/// and cell for cell what the engine produced in Arrow.
///
/// The expectation is built by the test helper (`common::decode`, which reads the
/// Arrow arrays itself), so this compares two independent readers of the same
/// batch: a wrong scale, a lost microsecond or a shifted column shows up here.
#[test]
fn the_result_is_column_oriented_and_carries_the_engine_values() {
    let schema = common::schema();
    let batches = common::csv_batches();
    let query = validate(
        r#"{"source":"orders","select":["id","country","amount"],
            "filter":{"field":"country","op":"eq","value":"DE"},
            "sort":[{"field":"id"}],"limit":4}"#,
        &schema,
    );

    let direct = execute(&batches, &query).expect("the engine runs the query");
    let source = LocalDataSource::new(batches).expect("the dataset has batches");
    let result = block_on(source.execute(query)).expect("the source runs the query");

    let names: Vec<&str> = result
        .schema
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(names, ["id", "country", "amount"], "schema order");
    assert_eq!(result.columns.len(), 3, "one column per output field");
    assert_eq!(result.row_count(), 4, "the page is four rows");
    for column in &result.columns {
        assert_eq!(column.len(), result.row_count(), "columns stay aligned");
    }

    let table = Table::from(&result);
    assert_eq!(
        table.rows,
        common::decode(&direct.batches),
        "the adapter decodes what the engine produced"
    );
    assert_eq!(result.total_count, direct.total_count);

    // The types travel with the values: `id` stays an integer, `amount` a decimal
    // (a widened or narrowed column would have been caught above already).
    assert!(matches!(result.columns[0][0], Value::Int64(_)));
    assert!(matches!(result.columns[2][0], Value::Decimal(_)));
    assert!(
        result.columns[1].iter().all(|value| *value == de()),
        "the filter held"
    );
}

/// A page past the last row is empty, not an error, and keeps its columns: the
/// grid draws the header from them, and `total_count` still counts the result.
#[test]
fn a_page_beyond_the_end_keeps_its_columns() {
    let schema = common::schema();
    let source = LocalDataSource::new(common::csv_batches()).expect("the dataset has batches");
    let query = validate(
        r#"{"source":"orders","select":["id","country"],
            "filter":{"field":"country","op":"eq","value":"DE"},
            "sort":[{"field":"id"}],"offset":1000}"#,
        &schema,
    );

    let result = block_on(source.execute(query)).expect("the source runs the query");
    assert_eq!(result.row_count(), 0, "no row that far out");
    assert_eq!(result.columns.len(), 2, "the columns stay");
    assert!(result.columns.iter().all(Vec::is_empty));
    assert!(result.total_count > 0, "the filter still matched rows");
}

/// The coercion path E14 names: a [`QueryResult`] becomes data again.
///
/// This is what makes hybrid execution possible (plan point 28) — the source's
/// partial answer has to go back into the engine without losing a value on the
/// way. Every column type of the dataset travels, NULLs and the non-finite
/// floats of E13 included.
#[test]
fn a_result_goes_back_into_the_engine_unchanged() {
    let schema = common::schema();
    let source = LocalDataSource::new(common::csv_batches()).expect("the dataset has batches");
    let every_column: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| format!("\"{}\"", field.name.as_str()))
        .collect();
    let read_all = validate(
        &format!(
            r#"{{"source":"orders","select":[{}]}}"#,
            every_column.join(",")
        ),
        &schema,
    );

    let first = block_on(source.execute(read_all.clone())).expect("the source answers");
    let again = LocalDataSource::from_result(&first).expect("the result is data again");
    let second = block_on(again.execute(read_all)).expect("the round trip answers");

    assert_eq!(second.schema, first.schema);
    assert_eq!(second.total_count, first.total_count);
    // Through the suite's comparison, not `assert_eq!`: NaN is not equal to
    // itself, and the dataset has one (E13).
    compare(
        &Table::from(&first),
        &Table::from(&second),
        RowOrder::Ordered,
    )
    .expect("every value survived the round trip");
    assert!(
        first
            .columns
            .iter()
            .flatten()
            .any(|value| *value == Value::Null),
        "the dataset has NULLs — otherwise this proves less than it looks"
    );
}

/// An empty answer keeps its columns on the way back, so the steps that follow
/// still know what they are working on.
#[test]
fn an_empty_result_still_carries_its_schema() {
    let schema = common::schema();
    let source = LocalDataSource::new(common::csv_batches()).expect("the dataset has batches");
    let query = validate(
        r#"{"source":"orders","select":["id","country"],
            "filter":{"field":"country","op":"eq","value":"ZZ"}}"#,
        &schema,
    );

    let empty = block_on(source.execute(query)).expect("the source answers");
    assert_eq!(empty.row_count(), 0);

    let again = LocalDataSource::from_result(&empty).expect("an empty result is still data");
    assert_eq!(block_on(DataSource::schema(&again)).unwrap(), empty.schema);
}
