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

use opengrid_conformance::{block_on, check_tree_case, load_schema, tree_cases, tree_dataset};
use opengrid_datasource::{DataSource, wire};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_json::{FromJson, Json};
use opengrid_query::{Limits, Query};

fn suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance")
}

/// The engine's answer to one case, read back from the wire form.
fn answer(case: &Json) -> Result<opengrid_datasource::QueryResult, String> {
    let query = Query::from_json(&case["query"]).map_err(|e| e.to_string())?;
    let (csv, schema_path) = tree_dataset(query.source.as_str());
    let schema = load_schema(&schema_path).expect("schema");
    let table = load_csv(&std::fs::read(csv).unwrap(), &schema, CsvOptions::default())
        .expect("the CSV loads");
    let validated = query
        .validate(&schema, &Limits::default())
        .map_err(|e| e.to_string())?;
    let result =
        block_on(LocalDataSource::new(table).execute(validated)).map_err(|e| e.to_string())?;
    // Through the wire and back: what a reader of the answer gets.
    Ok(wire::result_from_json(&wire::result_to_json(&result)).expect("reads back"))
}

/// Every tree case, answered by the engine and read back from the wire.
#[test]
fn the_local_engine_answers_every_tree_case() {
    let cases = tree_cases();
    assert!(cases.len() >= 9, "the suite has its cases");
    let failures: Vec<String> = cases
        .iter()
        .flat_map(|(id, case)| {
            check_tree_case(case, answer(case))
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
