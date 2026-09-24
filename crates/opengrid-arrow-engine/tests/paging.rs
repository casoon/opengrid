//! Paging through a sort with ties — plan point 44.
//!
//! The grid scrolls through a sort by `customer`, which is almost nothing but
//! ties, one window at a time. S6 leaves the order of ties undefined, but the
//! windows have to agree with each other: laid end to end they must be the full
//! sort, with no row twice and none missing. The engine stops sorting once it
//! has the rows a page needs, so this holds only because its order is total —
//! these tests are what would notice if it were not. Both paths of the sort are
//! covered: one key, and several (`execute/order.rs`).

mod common;

use std::ops::Range;

use opengrid_arrow_engine::execute::execute;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::{Schema, Value};

const ROWS: usize = 2_000;

/// The sorts under test, each with the result columns that form its key.
const SORTS: [(&str, Range<usize>); 2] = [
    (r#"[{"field":"customer"}]"#, 1..2),
    (
        r#"[{"field":"customer"},{"field":"country","direction":"desc"}]"#,
        1..3,
    ),
];

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("a well-formed query");
    query
        .validate(schema, &Limits::default())
        .expect("a valid query")
}

fn data() -> (Vec<arrow_array::RecordBatch>, Schema) {
    let schema = xtask::orders_schema();
    let csv = xtask::orders_csv(ROWS, 7);
    let batches = load_csv(csv.as_bytes(), &schema, CsvOptions::default()).expect("ingest");
    (batches, schema)
}

fn id(row: &[Value]) -> i64 {
    match row[0] {
        Value::Int64(id) => id,
        ref other => panic!("an id is an int64, got {other:?}"),
    }
}

/// The rows (id, customer, country) of a sort, from `offset`, at most `limit`.
fn sorted(
    batches: &[arrow_array::RecordBatch],
    schema: &Schema,
    sort: &str,
    offset: usize,
    limit: Option<usize>,
) -> Vec<Vec<Value>> {
    let limit = limit.map_or(String::new(), |limit| format!(r#","limit":{limit}"#));
    let query = validate(
        &format!(
            r#"{{"source":"orders","select":["id","customer","country"],
                "sort":{sort},"offset":{offset}{limit}}}"#
        ),
        schema,
    );
    let result = execute(batches, &query).expect("the engine answers");
    common::decode(&result.batches)
}

#[test]
fn windows_laid_end_to_end_are_the_full_sort() {
    let (batches, schema) = data();
    for (sort, _) in SORTS {
        let full: Vec<i64> = sorted(&batches, &schema, sort, 0, None)
            .iter()
            .map(|row| id(row))
            .collect();
        assert_eq!(full.len(), ROWS);

        // Window sizes that do not divide the row count, small ones and large.
        for window in [37, 40, 150, 700] {
            let mut stitched = Vec::with_capacity(ROWS);
            let mut offset = 0;
            while offset < ROWS {
                stitched.extend(
                    sorted(&batches, &schema, sort, offset, Some(window))
                        .iter()
                        .map(|row| id(row)),
                );
                offset += window;
            }
            assert_eq!(
                stitched, full,
                "{sort}: windows of {window} rows disagree with the full sort"
            );
        }
    }
}

#[test]
fn ties_keep_their_input_order() {
    // The generator writes ids in ascending order, so within equal keys the ids
    // must ascend: the input position is the last key.
    let (batches, schema) = data();
    for (sort, key) in SORTS {
        let rows = sorted(&batches, &schema, sort, 0, None);
        let mut ties = 0;
        for pair in rows.windows(2) {
            if pair[0][key.clone()] == pair[1][key.clone()] {
                ties += 1;
                assert!(
                    id(&pair[1]) > id(&pair[0]),
                    "{sort}: a tie out of input order: {} after {}",
                    id(&pair[1]),
                    id(&pair[0])
                );
            }
        }
        assert!(
            ties > ROWS / 2,
            "{sort}: the data has to be mostly ties ({ties})"
        );
    }
}
