//! The browser planner's API, driven on the host (plan point 28).
//!
//! `Planner` is what a page uses to split a query between a remote source and
//! the engine in the tab. The loop is three steps — plan, send, finish — and it
//! is all JSON, so it can be run here without a browser: a second `Engine`
//! stands in for the remote source, and the answer must match what one engine
//! alone would have said.

use opengrid_wasm::{Engine, Planner};

const ORDERS_CSV: &str = include_str!("../../opengrid-conformance/data/orders.csv");
const ORDERS_SCHEMA: &str = include_str!("../../opengrid-conformance/data/orders.schema.json");

/// Everything false but the named capabilities.
fn capabilities(can: &[&str]) -> String {
    let flags: Vec<String> = [
        "filter",
        "sort",
        "group",
        "aggregate",
        "paging",
        "pivot",
        "calculated_fields",
        "streaming",
    ]
    .iter()
    .map(|name| format!("\"{name}\":{}", can.contains(name)))
    .collect();
    format!("{{{}}}", flags.join(","))
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .expect("the dataset loads");
    engine
}

/// The whole loop: plan, let the "remote" answer, finish here.
fn hybrid(planner: &Planner, remote: &Engine, query: &str, mode: &str) -> (String, String) {
    let plan: serde_json::Value = serde_json::from_str(
        &planner
            .plan_json(query, mode)
            .unwrap_or_else(|_| panic!("planning {query} in mode {mode:?}")),
    )
    .expect("the plan is JSON");

    let partial = remote
        .execute_result(&plan["source"].to_string())
        .expect("the remote answers its half");
    let partial_json = opengrid_datasource::wire::result_to_json(&partial);

    let describe = plan["describe"].as_str().expect("a description").to_owned();
    if plan["client"].is_null() {
        return (partial_json, describe);
    }
    let finished = planner
        .finish_result(&plan["client"].to_string(), &partial_json)
        .expect("the client half runs");
    (
        opengrid_datasource::wire::result_to_json(&finished),
        describe,
    )
}

const QUERY: &str = r#"{"source":"orders","select":["id","country"],
    "filter":{"field":"country","op":"eq","value":"DE"},
    "sort":[{"field":"id","direction":"asc"}],"offset":2,"limit":5}"#;

/// However the work is divided, the answer is the same answer.
#[test]
fn every_split_gives_the_same_answer() {
    let remote = engine();
    let alone = opengrid_datasource::wire::result_to_json(
        &remote.execute_result(QUERY).expect("one engine answers"),
    );

    let splits = [
        (capabilities(&["filter", "sort", "paging"]), "auto"),
        (capabilities(&["filter", "sort"]), "auto"),
        (capabilities(&["filter"]), "auto"),
        (capabilities(&[]), "auto"),
        // The mode overrides what the capabilities would have allowed.
        (capabilities(&["filter", "sort", "paging"]), "local"),
        (capabilities(&["filter", "sort", "paging"]), "remote"),
        (capabilities(&["filter", "sort", "paging"]), "hybrid"),
    ];

    for (caps, mode) in splits {
        let planner = Planner::build(ORDERS_SCHEMA, &caps, "auto").expect("a planner");
        let (answer, describe) = hybrid(&planner, &remote, QUERY, mode);
        assert_eq!(answer, alone, "mode {mode:?} with {caps}: {describe}");
    }
}

/// The plan is readable — this is the string a developer tool shows.
#[test]
fn the_plan_says_who_does_what() {
    let remote = engine();
    let planner =
        Planner::build(ORDERS_SCHEMA, &capabilities(&["filter"]), "auto").expect("planner");

    let (_, describe) = hybrid(&planner, &remote, QUERY, "");
    assert_eq!(describe, "source: filter | client: sort · page");

    let planner = Planner::build(
        ORDERS_SCHEMA,
        &capabilities(&["filter", "sort", "paging"]),
        "auto",
    )
    .expect("planner");
    let (_, describe) = hybrid(&planner, &remote, QUERY, "");
    assert_eq!(describe, "source: filter · sort · page | client: —");

    // `local` is the other extreme, on the very same source.
    let (_, describe) = hybrid(&planner, &remote, QUERY, "local");
    assert_eq!(describe, "source: scan | client: filter · sort · page");
}

/// `remote` refuses rather than quietly finishing the work in the tab.
#[test]
fn remote_mode_refuses_what_the_source_cannot_do() {
    let planner =
        Planner::build(ORDERS_SCHEMA, &capabilities(&["filter"]), "auto").expect("planner");
    assert!(
        planner.plan_json(QUERY, "remote").is_err(),
        "this source cannot sort, and mode=remote must say so"
    );
}

/// A misspelt mode is an error, not a silent fallback to another one.
#[test]
fn an_unknown_mode_is_refused() {
    assert!(Planner::build(ORDERS_SCHEMA, &capabilities(&[]), "hybrd").is_err());
    let planner = Planner::build(ORDERS_SCHEMA, &capabilities(&[]), "auto").expect("planner");
    assert!(planner.plan_json(QUERY, "server").is_err());
}

/// A grouped query survives the split too: the source hands over the raw
/// columns the client needs, not the columns the query selected.
#[test]
fn a_grouped_query_splits_and_still_adds_up() {
    let remote = engine();
    let query = r#"{"source":"orders","select":["country","total"],
        "group":["country"],
        "aggregate":[{"field":"amount","fn":"sum","as":"total"}],
        "sort":[{"field":"total","direction":"desc"}]}"#;
    let alone = opengrid_datasource::wire::result_to_json(
        &remote.execute_result(query).expect("one engine answers"),
    );

    let planner = Planner::build(ORDERS_SCHEMA, &capabilities(&["filter", "paging"]), "auto")
        .expect("planner");
    let plan: serde_json::Value =
        serde_json::from_str(&planner.plan_json(query, "").expect("a plan")).expect("JSON");
    assert_eq!(
        plan["source"]["select"],
        serde_json::json!(["country", "amount"]),
        "the group key and the column the sum reads"
    );

    let (answer, describe) = hybrid(&planner, &remote, query, "");
    assert_eq!(describe, "source: scan | client: group · aggregate · sort");
    assert_eq!(answer, alone);
}
