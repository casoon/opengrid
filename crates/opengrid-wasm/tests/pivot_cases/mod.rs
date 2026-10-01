//! The pivot conformance cases, shared by the host test and the browser test
//! of the engine's `pivot` (issue #28). Each test binary uses part of it.

#![allow(dead_code)]

use opengrid_json::Json;
use opengrid_wasm::Engine;

pub const ORDERS_CSV: &str = include_str!("../../../opengrid-conformance/data/orders.csv");
pub const ORDERS_SCHEMA: &str =
    include_str!("../../../opengrid-conformance/data/orders.schema.json");

/// Every committed pivot case, embedded: a browser test has no filesystem.
pub const CASES: [&str; 11] = [
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p1-a-pivot-is-rows-crossed-with-columns.json"
    ),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p2-a-null-group-is-not-a-subtotal.json"
    ),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p4-an-empty-cell-is-null-but-a-count-is-zero.json"
    ),
    include_str!("../../../opengrid-conformance/pivot-cases/p5-a-subtotal-is-a-real-average.json"),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p6-a-subtotal-follows-the-rows-it-sums.json"
    ),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p3-two-column-dimensions-cross-in-order.json"
    ),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p3-two-column-dimensions-under-two-row-dimensions.json"
    ),
    include_str!("../../../opengrid-conformance/pivot-cases/p9-a-level-sorted-by-a-measure.json"),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p9-a-level-sorted-by-its-values-descending.json"
    ),
    include_str!(
        "../../../opengrid-conformance/pivot-cases/p9-a-measure-sorts-by-the-whole-row.json"
    ),
    include_str!("../../../opengrid-conformance/pivot-cases/p9-each-level-has-its-own-order.json"),
];

pub fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .expect("the dataset loads");
    engine
}

/// Runs one case through the engine's pivot and checks what the case pins:
/// the levels, the number of rows, and each generated column's measure and
/// path. The cells are checked value by value by the native pivot suite
/// (`opengrid-pivot/tests/conformance_pivot.rs`); here the same answer has to
/// come out of the engine the browser runs — as JSON and as bytes alike.
pub fn check(case_json: &str) {
    let case = Json::parse(case_json).expect("the case is JSON");
    let id = case["id"].as_str().expect("an id").to_owned();
    let pivot = case["pivot"].to_string();
    let engine = engine();

    let (result, rows) = engine
        .pivot_result(&pivot)
        .unwrap_or_else(|error| panic!("{id}: {error}"));
    let expected = &case["expected"];
    let levels: Vec<u16> = opengrid_json::FromJson::from_json(&expected["levels"]).unwrap();
    assert_eq!(result.row_levels, levels, "{id}: levels");
    assert_eq!(
        result.row_count(),
        expected["rows"].as_array().unwrap().len(),
        "{id}: rows"
    );
    let columns: Vec<(String, Vec<String>)> = result
        .columns
        .iter()
        .map(|column| {
            (
                column.measure.as_str().to_owned(),
                column.path.iter().map(opengrid_json::to_string).collect(),
            )
        })
        .collect();
    let expected_columns: Vec<(String, Vec<String>)> = expected["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|column| {
            (
                column["measure"].as_str().unwrap().to_owned(),
                column["path"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(Json::to_string)
                    .collect(),
            )
        })
        .collect();
    assert_eq!(columns, expected_columns, "{id}: columns");

    // The binary form is the same pivot, written as the JSON form writes it.
    let bytes = opengrid_pivot::pivot_to_bytes(&result, &rows);
    let (back, back_rows) = opengrid_pivot::pivot_from_bytes(&bytes).expect("the bytes read back");
    assert_eq!(
        opengrid_pivot::pivot_to_json(&back, &back_rows),
        opengrid_pivot::pivot_to_json(&result, &rows),
        "{id}: bytes"
    );
}
