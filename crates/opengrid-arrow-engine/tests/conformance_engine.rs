//! Point 07/08 closure: the **whole** conformance suite against the local engine,
//! driven through the `DataSource` trait it implements (point 09).
//!
//! Since point 08 this runner skips nothing: grouping and aggregation are part of
//! the engine, and the run counts what it answered against what the suite holds.

mod common;

use std::path::PathBuf;

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::execute::execute;
use opengrid_conformance::{RowOrder, Table, block_on, check_dir, compare};
use opengrid_datasource::DataSource;
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::{Schema, Value};

/// The cases live next to the suite that owns them.
fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance/cases")
}

/// The local engine as a [`DataSource`], over the dataset.
///
/// Point 09 binds data and engine: the source holds the batches and answers with
/// the Arrow-free result, so no adapter lives in the test any more.
fn source() -> LocalDataSource {
    LocalDataSource::new(common::csv_batches()).expect("the dataset has batches")
}

/// Every case of the suite, answered by the engine — nothing skipped.
#[test]
fn the_local_engine_answers_the_whole_suite() {
    let schema = common::schema();
    let source = source();
    let checked = check_dir(&cases_dir(), &schema).expect("the cases load");

    let mut ran = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for case in &checked {
        let order = if case.case.ordered {
            RowOrder::Ordered
        } else {
            RowOrder::Unordered
        };
        match block_on(source.execute(case.query.clone())) {
            Ok(result) => match compare(&case.expected, &Table::from(&result), order) {
                Ok(()) => ran += 1,
                Err(mismatch) => failed.push(format!("{}: {mismatch}", case.case.id)),
            },
            Err(error) => failed.push(format!("{}: {error}", case.case.id)),
        }
    }

    println!(
        "conformance (local engine): {ran} of {} cases answered, {} failed",
        checked.len(),
        failed.len()
    );

    assert!(
        failed.is_empty(),
        "{} of {} cases failed:\n{}",
        failed.len(),
        checked.len(),
        failed.join("\n")
    );
    assert_eq!(
        ran,
        checked.len(),
        "the whole suite runs against the local engine — no case may be skipped"
    );
    assert!(ran >= 40, "the suite is smaller than expected: {ran}");
}

/// `total_count` is the result size, not the page — that is what the grid shows
/// next to the page number.
#[test]
fn total_count_counts_the_result_not_the_page() {
    let schema = common::schema();
    let batches = common::csv_batches();
    let query = validate(
        r#"{"source":"orders","select":["id"],
            "filter":{"field":"country","op":"eq","value":"DE"},
            "sort":[{"field":"id"}],"limit":3}"#,
        &schema,
    );

    let result = execute(&batches, &query).expect("the engine runs the query");
    assert_eq!(result.batches.len(), 1);
    assert_eq!(
        result.batches[0].num_rows(),
        3,
        "the page is asked for 3 rows"
    );

    // Counted from the fixture, not from the engine's own answer.
    let de = common::decode(&batches)
        .iter()
        .filter(|row| row[2] == Value::Utf8("DE".to_owned()))
        .count() as u64;
    assert!(de > 3, "the fixture has more than one page of DE rows");
    assert_eq!(result.total_count, de);
}

/// Paging and sorting act on the aggregate result, and `total_count` counts the
/// groups — not the input rows.
#[test]
fn paging_and_sorting_act_on_the_aggregate() {
    let schema = common::schema();
    let batches = common::csv_batches();
    let query = validate(
        r#"{"source":"orders","select":["country","revenue"],
            "group":["country"],
            "aggregate":[{"field":"amount","fn":"sum","as":"revenue"}],
            "sort":[{"field":"revenue","direction":"desc"}],"limit":2}"#,
        &schema,
    );

    let result = execute(&batches, &query).expect("the engine runs the query");
    assert_eq!(result.batches[0].num_rows(), 2, "the page is two groups");
    assert!(
        result.total_count > 2,
        "there are more groups than one page"
    );

    // Sorted by revenue, descending: the first page carries the two largest.
    let rows = common::decode(&result.batches);
    let first = match rows[0][1] {
        Value::Decimal(value) => value.value(),
        ref other => panic!("{other:?}"),
    };
    let second = match rows[1][1] {
        Value::Decimal(value) => value.value(),
        ref other => panic!("{other:?}"),
    };
    assert!(first >= second, "{first} should not come after {second}");
}

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("the query parses");
    query
        .validate(schema, &Limits::default())
        .expect("the query validates")
}
