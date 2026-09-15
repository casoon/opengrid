//! Point 07 closure: the conformance suite against the **local engine**.
//!
//! The suite's cases run through `opengrid_conformance::Engine`, the docking
//! point every engine implements (the local one here, PostgreSQL in point 26).
//! The runner reports what it ran and what it skipped: point 08 owns grouping
//! and aggregation, so those cases are counted and named, never silently
//! dropped.

mod common;

use std::path::PathBuf;

use arrow_array::RecordBatch;
use opengrid_arrow_engine::execute::{ExecuteError, execute};
use opengrid_conformance::{Engine, EngineError, RowOrder, Table, check_dir, compare};
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::{Schema, Value};

/// The cases live next to the suite that owns them.
fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance/cases")
}

/// The local engine as an [`Engine`]: the data is the dataset, the query comes
/// in validated. A thin adapter, because the trait of point 09 (`opengrid-datasource`)
/// is what will bind data and engine properly — until then the engine crate must
/// not depend on the test suite to satisfy it.
struct LocalEngine {
    batches: Vec<RecordBatch>,
}

impl Engine for LocalEngine {
    fn run(&self, _schema: &Schema, query: &ValidatedQuery) -> Result<Table, EngineError> {
        let result = execute(&self.batches, query)
            .map_err(|error| EngineError(format!("local engine: {error}")))?;
        Ok(Table::new(
            result
                .schema
                .fields()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            common::decode(&result.batches),
        ))
    }
}

/// Every case that needs no grouping, answered by the engine.
#[test]
fn the_local_engine_answers_every_case_without_grouping() {
    let schema = common::schema();
    let engine = LocalEngine {
        batches: common::csv_batches(),
    };
    let checked = check_dir(&cases_dir(), &schema).expect("the cases load");

    let mut ran = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    for case in &checked {
        if !case.query.group.is_empty() || !case.query.aggregate.is_empty() {
            skipped.push(case.case.id.clone());
            continue;
        }
        let result = engine
            .run(&schema, &case.query)
            .unwrap_or_else(|error| panic!("{}: {error}", case.case.id));
        let order = if case.case.ordered {
            RowOrder::Ordered
        } else {
            RowOrder::Unordered
        };
        compare(&case.expected, &result, order)
            .unwrap_or_else(|mismatch| panic!("{}: {mismatch}", case.case.id));
        ran += 1;
    }

    println!(
        "conformance (local engine): {ran} of {} cases ran, {} skipped — group/aggregate, point 08: {skipped:?}",
        checked.len(),
        skipped.len()
    );

    assert_eq!(
        ran + skipped.len(),
        checked.len(),
        "every case has to be either run or reported as skipped"
    );
    assert!(
        ran >= 30,
        "the executor has to carry the suite; only {ran} cases ran"
    );
    assert_eq!(
        skipped.len(),
        12,
        "the grouping cases moved (S10 grouping, S11 aggregates, S12 result types) — \
         adjust this count deliberately: {skipped:?}"
    );
}

/// The engine refuses a grouping query instead of answering half of it.
#[test]
fn a_grouping_query_is_refused_not_half_answered() {
    let schema = common::schema();
    let batches = common::csv_batches();
    let query = validate(
        r#"{"source":"orders","select":["country"],"group":["country"]}"#,
        &schema,
    );

    match execute(&batches, &query) {
        Err(ExecuteError::Unsupported { feature }) => {
            assert!(feature.contains("point 08"), "{feature}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// `total_count` is the filter result, not the page — that is what the grid
/// shows next to the page number.
#[test]
fn total_count_counts_the_filter_result_not_the_page() {
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

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("the query parses");
    query
        .validate(schema, &Limits::default())
        .expect("the query validates")
}
