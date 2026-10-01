//! The tree conformance cases (E38), shared by the host test and the browser
//! test of the engine (plan point 121). Each test binary uses part of it.

#![allow(dead_code)]

use opengrid_json::{Json, ToJson};
use opengrid_wasm::Engine;

const TREE_CSV: &str = include_str!("../../../opengrid-conformance/data/tree.csv");
const TREE_SCHEMA: &str = include_str!("../../../opengrid-conformance/data/tree.schema.json");
const CYCLE_CSV: &str = include_str!("../../../opengrid-conformance/data/tree-cycle.csv");
const CYCLE_SCHEMA: &str =
    include_str!("../../../opengrid-conformance/data/tree-cycle.schema.json");

/// Every committed tree case, embedded: a browser test has no filesystem.
pub const CASES: [&str; 9] = [
    include_str!(
        "../../../opengrid-conformance/tree-cases/t2-roots-are-null-parents-and-orphans.json"
    ),
    include_str!(
        "../../../opengrid-conformance/tree-cases/t3-a-cycle-is-an-error-that-names-it.json"
    ),
    include_str!("../../../opengrid-conformance/tree-cases/t4-a-leaf-has-no-children.json"),
    include_str!(
        "../../../opengrid-conformance/tree-cases/t4-a-level-is-the-children-of-a-node-with-their-counts.json"
    ),
    include_str!(
        "../../../opengrid-conformance/tree-cases/t5-a-match-comes-with-its-ancestors-as-context.json"
    ),
    include_str!(
        "../../../opengrid-conformance/tree-cases/t5-a-matching-parent-keeps-only-matching-children.json"
    ),
    include_str!(
        "../../../opengrid-conformance/tree-cases/t5-context-leads-down-to-the-match.json"
    ),
    include_str!("../../../opengrid-conformance/tree-cases/t6-a-sort-orders-siblings.json"),
    include_str!("../../../opengrid-conformance/tree-cases/t6-siblings-page-among-themselves.json"),
];

pub fn engine() -> Engine {
    let mut engine = Engine::new();
    engine
        .load_csv("tree", TREE_CSV.as_bytes(), TREE_SCHEMA)
        .expect("the tree loads");
    engine
        .load_csv("tree_cycle", CYCLE_CSV.as_bytes(), CYCLE_SCHEMA)
        .expect("the cycle loads");
    engine
}

/// Runs one case through the engine and checks all it pins: the rows, the
/// total, the tree part — or the error it expects.
pub fn check(case_json: &str) {
    let case = Json::parse(case_json).expect("the case is JSON");
    let id = case["id"].as_str().expect("an id").to_owned();
    let answer = engine().execute_result(&case["query"].to_string());

    if let Some(error) = case.get("expected_error") {
        let message = answer.expect_err(&id);
        for word in error["contains"].as_array().unwrap() {
            let word = word.as_str().unwrap();
            assert!(
                message.contains(word),
                "{id}: {message:?} does not say {word:?}"
            );
        }
        return;
    }
    let result = answer.unwrap_or_else(|error| panic!("{id}: {error}"));
    let expected = &case["expected"];
    let rows: Vec<Json> = (0..result.row_count())
        .map(|row| {
            Json::Array(
                result
                    .columns
                    .iter()
                    .map(|column| column[row].to_json())
                    .collect(),
            )
        })
        .collect();
    assert_eq!(&Json::Array(rows), &expected["rows"], "{id}: rows");
    if let Some(total) = expected["total_count"].as_u64() {
        assert_eq!(result.total_count, total, "{id}: total_count");
    }
    let tree = result.tree.unwrap_or_else(|| panic!("{id}: no tree part"));
    let want = &expected["tree"];
    assert_eq!(tree.children.to_json(), want["children"], "{id}: children");
    assert_eq!(tree.matched.to_json(), want["match"], "{id}: match");
    assert_eq!(
        Some(tree.matches),
        want["matches"].as_u64(),
        "{id}: matches"
    );
    assert_eq!(
        Some(tree.orphans),
        want["orphans"].as_u64(),
        "{id}: orphans"
    );
}
