#![cfg(target_arch = "wasm32")]
//! The conformance suite, answered by the WASM build in a real browser.
//!
//! Point 10's DoD asks for at least three conformance cases run in the browser.
//! This is the same suite the native runner uses — same case files, same
//! `check_case` validation, same `compare` (decimal exact, floats with
//! tolerance, `NaN == NaN`) — only the engine under test is the one compiled to
//! WebAssembly and driven through the `DataSource` trait.
//!
//! The query and the expectation come from the committed case JSON, not from a
//! copy: a case that changes changes this test. The dataset and its schema are
//! the conformance fixtures, embedded at compile time because a browser test has
//! no filesystem.
//!
//! Run with `just wasm-test` (headless Chrome). On the host target the file
//! compiles to nothing — `wasm-bindgen` cannot call a browser there — so
//! `just check` stays green and `just wasm-test` is the gate that runs it.
//! The demo's own query is pinned on the host side in `tests/demo_query.rs`;
//! here it is checked for the properties the demo page displays.

use opengrid_conformance::{Case, RowOrder, Table, check_case, compare};
use opengrid_types::Value;
use opengrid_wasm::Engine;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const ORDERS_CSV: &str = include_str!("../../opengrid-conformance/data/orders.csv");
const ORDERS_SCHEMA: &str = include_str!("../../opengrid-conformance/data/orders.schema.json");

/// The demo page's default query — the example of
/// `plan/spezifikation/02-query-modell.md`, adapted to the conformance dataset
/// (decimal literal as a string, threshold lowered so rows match). The host test
/// `demo_query.rs` pins the exact table; the file is shared so the two cannot
/// drift apart.
const DEMO_QUERY: &str = include_str!("queries/demo_default.json");

/// The original example of `02-query-modell.md` — valid, but no `DE` row in the
/// fixture exceeds 100, so it answers zero rows.
const SPEC_EXAMPLE: &str = include_str!("queries/spec_example.json");

/// Loads the conformance dataset into a fresh engine.
fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .expect("the dataset loads");
    engine
}

/// Runs one committed case and fails with the suite's own diagnosis.
fn check(case_json: &str) {
    let case: Case = serde_json::from_str(case_json).expect("the case parses");
    let schema = opengrid_wasm::schema_json::from_json(ORDERS_SCHEMA).expect("the schema parses");
    let checked = check_case(case, &schema).expect("the case validates against the schema");

    let query_json = serde_json::to_string(&checked.case.query).expect("the query serializes");
    let result = engine()
        .execute_result(&query_json)
        .expect("the query runs against the WASM engine");

    let order = if checked.case.ordered {
        RowOrder::Ordered
    } else {
        RowOrder::Unordered
    };
    if let Err(message) = compare(&checked.expected, &Table::from(&result), order) {
        panic!("{}: {message}", checked.case.id);
    }
}

/// S1 — the `or` of two ids, ordered by `id`.
#[wasm_bindgen_test]
fn s1_or_of_two_ids() {
    check(include_str!(
        "../../opengrid-conformance/cases/s1-or-of-two-ids.json"
    ));
}

/// S5 — `contains` is case-sensitive.
#[wasm_bindgen_test]
fn s5_contains_is_case_sensitive() {
    check(include_str!(
        "../../opengrid-conformance/cases/s5-contains-is-case-sensitive.json"
    ));
}

/// S10 — the empty string is a value and forms its own group.
#[wasm_bindgen_test]
fn s10_empty_string_is_not_the_null_group() {
    check(include_str!(
        "../../opengrid-conformance/cases/s10-empty-string-is-not-the-null-group.json"
    ));
}

/// S11 — an aggregate over a NULL-only set still answers one row, and it is
/// `Null`. This one goes through the aggregate path in the browser, which the
/// three cases above do not reach except in `s10`.
#[wasm_bindgen_test]
fn s11_aggregates_on_all_null_set() {
    check(include_str!(
        "../../opengrid-conformance/cases/s11-aggregates-on-all-null-set.json"
    ));
}

/// The demo path end to end: the default query, through the WASM build.
///
/// The pinned numbers live in `tests/demo_query.rs` (host test, part of
/// `just check`); this asserts what the demo page shows on top of them — the
/// output columns, the row count and the descending order of `revenue`.
#[wasm_bindgen_test]
fn demo_default_query_is_sorted_by_revenue_descending() {
    let result = engine()
        .execute_result(DEMO_QUERY)
        .expect("the demo query runs");

    let table = Table::from(&result);
    let names: Vec<&str> = table.columns.iter().map(|name| name.as_str()).collect();
    assert_eq!(names, ["customer", "revenue"]);
    assert_eq!(result.total_count, table.rows.len() as u64);
    assert!(
        table.rows.len() > 1,
        "the demo selects several rows to sort"
    );

    // `revenue` is `sum(Decimal(12, 2))`, i.e. Decimal(38, 2) — the values are
    // decimals, and the wire notation asks for a string in JSON.
    let revenues: Vec<i128> = table
        .rows
        .iter()
        .map(|row| match &row[1] {
            Value::Decimal(decimal) => decimal.value(),
            other => panic!("expected a decimal revenue, got {other:?}"),
        })
        .collect();
    for pair in revenues.windows(2) {
        assert!(
            pair[0] >= pair[1],
            "revenue must not increase: {pair:?} (rows: {:?})",
            table.rows
        );
    }
}

/// The original example of `02-query-modell.md` runs in the browser too — it is
/// valid and, on this fixture, answers zero rows (see `demo_query.rs`).
#[wasm_bindgen_test]
fn spec_example_runs_and_matches_no_row() {
    let result = engine()
        .execute_result(SPEC_EXAMPLE)
        .expect("the spec example is valid");
    assert_eq!(result.total_count, 0);
    assert_eq!(result.row_count(), 0);
}
