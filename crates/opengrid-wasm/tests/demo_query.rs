//! The demo's query, pinned on the host.
//!
//! Point 10's demo page (`examples/engine-demo/index.html`) defaults to a query
//! derived from the example in `plan/spezifikation/02-query-modell.md`. The same
//! query runs in the browser through the WASM build
//! (`conformance_in_browser.rs`); this host test is the reference it is compared
//! against and the reason the demo's output is not left to a look at the page —
//! it pins the exact table.
//!
//! It is part of `just check`, so a change to the engine, the fixture or the
//! query that changes the demo's output fails here first.
//!
//! **Why the demo query differs from `02-query-modell.md`.** The example there is
//! schema-agnostic and written for a column whose type is not fixed. Over the
//! conformance dataset two things change, both forced by the dataset's schema:
//!
//! * `amount` is `Decimal(12, 2)`, and decimals travel as JSON **strings** — the
//!   literal is `"100"`, not `100` (a JSON integer is a type mismatch, not a
//!   decimal).
//! * No `DE` row has an amount above 100 — the largest is 50.00. With the
//!   original threshold the query is correct but answers zero rows, which is a
//!   poor demo. The demo default therefore lowers it to 40, which selects the
//!   three rows below.
//!
//! Both queries are pinned: [`demo_default_query_returns_the_pinned_table`] for
//! what the page shows, [`spec_example_validates_but_matches_no_row`] so the
//! original example and the reason for the deviation stay visible.

use opengrid_conformance::Table;
use opengrid_types::{Decimal, Value};
use opengrid_wasm::Engine;

const ORDERS_CSV: &str = include_str!("../../opengrid-conformance/data/orders.csv");
const ORDERS_SCHEMA: &str = include_str!("../../opengrid-conformance/data/orders.schema.json");

/// The demo page's default query, shared with the browser test as a file so the
/// two cannot drift apart.
const DEMO_QUERY: &str = include_str!("queries/demo_default.json");

/// The example of `02-query-modell.md`, with the decimal literal written the way
/// the type system requires.
const SPEC_EXAMPLE: &str = include_str!("queries/spec_example.json");

/// Loads the conformance dataset into a fresh engine.
fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .expect("the dataset loads");
    engine
}

/// Runs a query and returns its result as the suite's row-oriented table.
fn run(engine: &Engine, query: &str) -> (Table, u64) {
    let result = engine.execute_result(query).expect("the query runs");
    (Table::from(&result), result.total_count)
}

fn decimal(value: i128) -> Value {
    Value::Decimal(Decimal::new(value, 2))
}

/// What the demo page shows: `DE`, `amount > 40`, grouped by customer, summed and
/// sorted descending — the three matching rows.
///
/// The sums are `Decimal(38, 2)` (S12: `sum(Decimal(p, s))` widens to precision
/// 38 at the input scale), hence [`decimal`] with scale 2.
#[test]
fn demo_default_query_returns_the_pinned_table() {
    let (table, total_count) = run(&engine(), DEMO_QUERY);

    let names: Vec<&str> = table.columns.iter().map(|name| name.as_str()).collect();
    assert_eq!(names, ["customer", "revenue"]);
    assert_eq!(total_count, 3, "three DE customers exceed an amount of 40");

    let expected = vec![
        vec![Value::Utf8("Epsilon".to_owned()), decimal(5000)],
        vec![Value::Utf8("Beta".to_owned()), decimal(4600)],
        vec![Value::Utf8("Alpha".to_owned()), decimal(4500)],
    ];
    assert_eq!(table.rows, expected);
}

/// The original example: valid, but the fixture has no `DE` amount above 100, so
/// it answers no row. Pinned so the deviation in the demo stays deliberate.
#[test]
fn spec_example_validates_but_matches_no_row() {
    let (table, total_count) = run(&engine(), SPEC_EXAMPLE);

    let names: Vec<&str> = table.columns.iter().map(|name| name.as_str()).collect();
    assert_eq!(names, ["customer", "revenue"]);
    assert_eq!(total_count, 0);
    assert!(table.rows.is_empty());
}
