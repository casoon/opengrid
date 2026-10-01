//! The tree's conformance cases (E38, rules T1–T6), answered by the local
//! engine — `crates/opengrid-conformance/tree-cases/` over the datasets
//! `tree.csv` (an org chart with two roots, an orphan and four levels) and
//! `tree-cycle.csv`.
//!
//! Each answer is also written in the wire form and read back, so the tree
//! part a level carries (`children`, `match`, `matches`, `orphans`) is checked
//! where a browser or a server would read it.

mod common;

use std::path::PathBuf;

use opengrid_conformance::{block_on, load_schema};
use opengrid_datasource::{DataSource, wire};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_json::{FromJson, Json, ToJson};
use opengrid_query::{Limits, Query};

fn suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance")
}

/// The source a case names, from the dataset of that name (`tree_cycle` is
/// `tree-cycle.csv`).
fn source(name: &str) -> (LocalDataSource, opengrid_types::Schema) {
    let file = name.replace('_', "-");
    let schema = load_schema(&suite().join(format!("data/{file}.schema.json"))).expect("schema");
    let table = load_csv(
        &common::data(&format!("{file}.csv")),
        &schema,
        CsvOptions::default(),
    )
    .expect("the CSV loads");
    (LocalDataSource::new(table), schema)
}

fn cases() -> Vec<(String, Json)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(suite().join("tree-cases"))
        .expect("the tree cases")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read");
            let case = Json::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            (case["id"].as_str().expect("an id").to_owned(), case)
        })
        .collect()
}

/// What one case got wrong, or nothing.
fn run(case: &Json) -> Vec<String> {
    let query = Query::from_json(&case["query"]).expect("the case's query reads");
    let (source, schema) = source(query.source.as_str());
    let validated = query
        .validate(&schema, &Limits::default())
        .expect("the case's query is valid");
    let answer = block_on(source.execute(validated));

    if let Some(error) = case.get("expected_error") {
        let Err(error_text) = answer.map(|_| ()).map_err(|e| e.to_string()) else {
            return vec!["expected an error, got an answer".to_owned()];
        };
        return error["contains"]
            .as_array()
            .expect("contains")
            .iter()
            .filter_map(Json::as_str)
            .filter(|word| !error_text.contains(word))
            .map(|word| format!("the error {error_text:?} does not say {word:?}"))
            .collect();
    }

    let result = match answer {
        Ok(result) => result,
        Err(error) => return vec![format!("failed: {error}")],
    };
    // Through the wire and back: what a reader of the answer gets.
    let result = wire::result_from_json(&wire::result_to_json(&result)).expect("reads back");
    let expected = &case["expected"];
    let mut problems = Vec::new();

    let columns: Vec<String> = result
        .schema
        .fields()
        .iter()
        .map(|f| f.name.to_string())
        .collect();
    let want_columns: Vec<String> = expected["columns"]
        .as_array()
        .expect("columns")
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    if columns != want_columns {
        problems.push(format!(
            "columns: expected {want_columns:?}, got {columns:?}"
        ));
    }
    let rows: Vec<Vec<Json>> = (0..result.row_count())
        .map(|row| {
            result
                .columns
                .iter()
                .map(|column| column[row].to_json())
                .collect()
        })
        .collect();
    let want_rows: Vec<Vec<Json>> = expected["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row.as_array().unwrap().clone())
        .collect();
    if rows != want_rows {
        problems.push(format!("rows: expected {want_rows:?}, got {rows:?}"));
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
    let numbers = |key: &str| -> Vec<u64> {
        want[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap())
            .collect()
    };
    let flags: Vec<bool> = want["match"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b.as_bool().unwrap())
        .collect();
    for (name, ok) in [
        ("children", tree.children == numbers("children")),
        ("match", tree.matched == flags),
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
    problems
}

/// Every tree case, answered by the engine and read back from the wire.
#[test]
fn the_local_engine_answers_every_tree_case() {
    let cases = cases();
    assert!(cases.len() >= 9, "the suite has its cases");
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(id, case)| {
            run(case)
                .into_iter()
                .map(move |problem| format!("{id}: {problem}"))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// T1: a key twice is an error that names it, not a tree with a guess in it.
#[test]
fn a_key_twice_is_an_error() {
    let schema = load_schema(&suite().join("data/tree-cycle.schema.json")).expect("schema");
    let table = load_csv(
        b"id,parent_id,name\n1,\\N,Root\n2,1,A\n2,1,B\n",
        &schema,
        CsvOptions::default(),
    )
    .expect("loads");
    let query = Query::from_json(
        &Json::parse(r#"{"source":"t","select":["id"],"tree":{"parent":"parent_id"}}"#).unwrap(),
    )
    .unwrap()
    .validate(&schema, &Limits::default())
    .unwrap();
    let error = block_on(LocalDataSource::new(table).execute(query))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("2") && error.contains("more than once"),
        "{error}"
    );
}

/// What a level of a large tree costs (plan point 121, 12-qualitaet.md):
/// 100 000 nodes, ten children each, four levels below the roots. Run with
/// `cargo test --release -p opengrid-engine --test conformance_tree -- --ignored --nocapture`.
#[test]
#[ignore]
fn a_large_tree_answers_a_level_quickly() {
    use std::time::Instant;
    let schema = load_schema(&suite().join("data/tree-cycle.schema.json")).expect("schema");
    let mut csv = String::from("id,parent_id,name\n");
    for id in 1..=100_000u64 {
        let parent = if id <= 10 {
            "\\N".to_owned()
        } else {
            ((id - 1) / 10).to_string()
        };
        csv.push_str(&format!("{id},{parent},node{}\n", id % 977));
    }
    let table = load_csv(csv.as_bytes(), &schema, CsvOptions::default()).expect("loads");
    let source = LocalDataSource::new(table);
    for (label, json) in [
        (
            "roots",
            r#"{"source":"t","select":["id","name"],"tree":{"parent":"parent_id"},"sort":[{"field":"name"}],"limit":40}"#,
        ),
        (
            "children of 5",
            r#"{"source":"t","select":["id","name"],"tree":{"parent":"parent_id","under":5},"sort":[{"field":"name"}],"limit":40}"#,
        ),
        (
            "filtered roots",
            r#"{"source":"t","select":["id","name"],"tree":{"parent":"parent_id"},"filter":{"field":"name","op":"eq","value":"node7"},"sort":[{"field":"name"}],"limit":40}"#,
        ),
    ] {
        let query = Query::from_json(&Json::parse(json).unwrap())
            .unwrap()
            .validate(&schema, &Limits::default())
            .unwrap();
        let start = Instant::now();
        let runs = 10;
        for _ in 0..runs {
            block_on(source.execute(query.clone())).expect("answers");
        }
        println!(
            "{label}: {:.1} ms",
            start.elapsed().as_secs_f64() * 1000.0 / runs as f64
        );
    }
}
