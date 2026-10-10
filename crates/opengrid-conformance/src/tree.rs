//! The tree's conformance cases (E38, rules T1–T7; plan points 121, 122, 125):
//! `tree-cases/*.json` over the datasets `tree.csv` and `tree-cycle.csv`.
//!
//! Shared by every runner — the engine, and each connector behind the server
//! — so a tree means the same thing wherever it is answered. A case names its
//! source (`tree`, `tree_cycle`); [`tree_dataset`] says which files hold it.

use std::path::PathBuf;

use opengrid_datasource::QueryResult;
use opengrid_json::{Json, ToJson};

use crate::suite_dir;

/// Every tree case, sorted by file name: `(id, case)`.
pub fn tree_cases() -> Vec<(String, Json)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(suite_dir().join("tree-cases"))
        .expect("the tree cases")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read a case");
            let case =
                Json::parse(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            (case["id"].as_str().expect("an id").to_owned(), case)
        })
        .collect()
}

/// The CSV and the schema of the dataset a case's `source` names
/// (`tree_cycle` is `tree-cycle.csv`).
pub fn tree_dataset(source: &str) -> (PathBuf, PathBuf) {
    let file = source.replace('_', "-");
    let data = suite_dir().join("data");
    (
        data.join(format!("{file}.csv")),
        data.join(format!("{file}.schema.json")),
    )
}

/// What `answer` got wrong for `case`, or nothing: the rows, the total, the
/// tree part — or, for a case that expects one, the error's words.
pub fn check_tree_case(case: &Json, answer: Result<QueryResult, String>) -> Vec<String> {
    if let Some(error) = case.get("expected_error") {
        let message = match answer {
            Ok(_) => return vec!["expected an error, got an answer".to_owned()],
            Err(message) => message,
        };
        return error["contains"]
            .as_array()
            .expect("contains")
            .iter()
            .filter_map(Json::as_str)
            .filter(|word| !message.contains(word))
            .map(|word| format!("the error {message:?} does not say {word:?}"))
            .collect();
    }
    let result = match answer {
        Ok(result) => result,
        Err(message) => return vec![format!("failed: {message}")],
    };
    let expected = &case["expected"];
    let mut problems = Vec::new();

    let columns: Vec<Json> = result
        .schema
        .fields()
        .iter()
        .map(|field| Json::from(field.name.as_str()))
        .collect();
    if Json::Array(columns.clone()) != expected["columns"] {
        problems.push(format!(
            "columns: expected {}, got {:?}",
            expected["columns"], columns
        ));
    }
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
    if Json::Array(rows.clone()) != expected["rows"] {
        problems.push(format!(
            "rows: expected {}, got {:?}",
            expected["rows"], rows
        ));
    }
    if let Some(total) = expected["total_count"].as_u64()
        && total != result.total_count
    {
        problems.push(format!(
            "total_count: expected {total}, got {}",
            result.total_count
        ));
    }
    let Some(tree) = &result.tree else {
        problems.push("the answer carries no tree part".to_owned());
        return problems;
    };
    let want = &expected["tree"];
    for (name, ok) in [
        ("children", tree.children.to_json() == want["children"]),
        ("match", tree.matched.to_json() == want["match"]),
        ("matches", Some(tree.matches) == want["matches"].as_u64()),
        ("orphans", Some(tree.orphans) == want["orphans"].as_u64()),
    ] {
        if !ok {
            problems.push(format!(
                "tree.{name}: expected {}, got {tree:?}",
                want[name]
            ));
        }
    }
    // T7: the subtree aggregates, by alias and in the query's order — and none
    // where none were asked for.
    let got: Vec<(String, Json)> = tree
        .aggregate_schema
        .fields()
        .iter()
        .zip(&tree.aggregates)
        .map(|(field, values)| (field.name.as_str().to_owned(), values.to_json()))
        .collect();
    let wanted: Vec<(String, Json)> = match want.get("aggregates") {
        None => Vec::new(),
        Some(Json::Object(object)) => object
            .iter()
            .map(|(alias, values)| (alias.clone(), values.clone()))
            .collect(),
        Some(other) => panic!("tree.aggregates in a case is an object, not {other}"),
    };
    if got != wanted {
        problems.push(format!("tree.aggregates: expected {wanted:?}, got {got:?}"));
    }
    problems
}
