//! The `DataSource` adapter over the local engine (point 09).
//!
//! The suite runner next door proves the answers are *right*; this file pins down
//! the adapter's own contract — the schema it reports, the column-oriented result
//! (E14), the empty page and the source without data.

mod common;

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::execute::execute;
use opengrid_conformance::{Table, block_on};
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
#[test]
fn the_schema_comes_from_the_batches() {
    let source = LocalDataSource::new(common::csv_batches()).expect("the dataset has batches");
    assert_eq!(
        block_on(source.schema()).expect("a source with data has a schema"),
        common::schema()
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
