//! **The question MVP D asks:** does a pivot come out the same from the engine
//! and from PostgreSQL? (plan point 31)
//!
//! The pivot engine sits on the `DataSource` trait, so PostgreSQL answers a
//! pivot without a single line of pivot-specific code: the `n+1` grouping sets
//! are ordinary queries, and point 26 already proved those agree. This file
//! checks the whole shape — cells, subtotals, levels and the generated columns —
//! against the same case files the local engine answers.
//!
//! Skips itself without a database, like every test in this crate.

mod pg;

use opengrid_conformance::block_on;
use opengrid_datasource_postgres::PostgresDataSource;
use opengrid_pivot::{PivotLimits, PivotQuery, PivotResult, execute};
use opengrid_query::Limits;
use opengrid_types::Value;

/// The pivot cases, as JSON, with the pivot left untyped until it is needed.
fn cases() -> Vec<(String, PivotQuery)> {
    let dir = pg::suite_dir().join("pivot-cases");
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("the pivot cases")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read case");
            let body: serde_json::Value = serde_json::from_str(&text).expect("JSON");
            let id = body["id"].as_str().expect("an id").to_owned();
            let pivot: PivotQuery = serde_json::from_value(body["pivot"].clone()).expect("a pivot");
            (id, pivot)
        })
        .collect()
}

/// Every cell, every level, every column — as text, so a difference reads.
fn render(result: &PivotResult) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(
        result
            .columns
            .iter()
            .map(|column| {
                format!(
                    "{}[{}]",
                    column.measure,
                    column
                        .path
                        .iter()
                        .map(|value| serde_json::to_string(value).expect("JSON"))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            })
            .collect::<Vec<_>>()
            .join(" | "),
    );
    for row in 0..result.row_count() {
        let cells: Vec<String> = result
            .data
            .columns
            .iter()
            .map(|column| match &column[row] {
                // S12: `avg` is a float, and two engines may differ in the last
                // bit. Everything else is compared exactly.
                Value::Float64(number) => format!("{number:.9}"),
                other => serde_json::to_string(other).expect("JSON"),
            })
            .collect();
        lines.push(format!("L{} {}", result.row_levels[row], cells.join(" | ")));
    }
    lines
}

#[tokio::test]
async fn a_pivot_is_the_same_from_the_engine_and_from_postgresql() {
    let Some((client, url)) = pg::connect().await else {
        return;
    };
    let schema = pg::schema();
    let table = pg::create_fixture(&client, &schema, "opengrid_pivot_cases").await;

    let remote = PostgresDataSource::connect(&url, &table, schema.clone()).expect("a source");
    let local = pg::local_source(&schema);

    let mut failures = Vec::new();
    let mut ran = 0;
    for (id, pivot) in cases() {
        let validated = pivot
            .validate(&schema, &PivotLimits::default(), &Limits::default())
            .unwrap_or_else(|error| panic!("{id}: {error}"));

        let here = block_on(execute(&local, &validated)).unwrap_or_else(|e| panic!("{id}: {e}"));
        let there = execute(&remote, &validated)
            .await
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        ran += 1;

        let (here, there) = (render(&here), render(&there));
        for (line, (a, b)) in here.iter().zip(&there).enumerate() {
            if a != b {
                failures.push(format!("{id}, line {line}:\n  engine: {a}\n  pg:     {b}"));
            }
        }
        if here.len() != there.len() {
            failures.push(format!(
                "{id}: {} rows here, {} there",
                here.len() - 1,
                there.len() - 1
            ));
        }
    }

    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");

    println!(
        "pivot against PostgreSQL: {ran} cases, {} failures",
        failures.len()
    );
    assert!(ran >= 5, "the pivot suite is smaller than expected: {ran}");
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// **The point of point 31:** the same pivot, one statement instead of `n+1`.
///
/// Generic path against PostgreSQL versus the `GROUPING SETS` pushdown against
/// the same table: the two must agree down to the level of every row, or the
/// pushdown is a different answer wearing the same name.
#[tokio::test]
async fn the_pushdown_answers_what_the_grouping_sets_answered() {
    let Some((client, url)) = pg::connect().await else {
        return;
    };
    let schema = pg::schema();
    let table = pg::create_fixture(&client, &schema, "opengrid_pivot_pushdown").await;
    let remote = PostgresDataSource::connect(&url, &table, schema.clone()).expect("a source");

    let mut failures = Vec::new();
    let mut ran = 0;
    for (id, pivot) in cases() {
        let validated = pivot
            .validate(&schema, &PivotLimits::default(), &Limits::default())
            .unwrap_or_else(|error| panic!("{id}: {error}"));

        let generic = execute(&remote, &validated)
            .await
            .unwrap_or_else(|error| panic!("{id}, generic: {error}"));
        let pushed = remote
            .execute_pivot(&validated)
            .await
            .unwrap_or_else(|error| panic!("{id}, pushdown: {error}"));
        ran += 1;

        let (generic, pushed) = (render(&generic), render(&pushed));
        for (line, (a, b)) in generic.iter().zip(&pushed).enumerate() {
            if a != b {
                failures.push(format!(
                    "{id}, line {line}:\n  n+1 queries: {a}\n  one statement: {b}"
                ));
            }
        }
        if generic.len() != pushed.len() {
            failures.push(format!(
                "{id}: {} rows one way, {} the other",
                generic.len() - 1,
                pushed.len() - 1
            ));
        }
    }

    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");

    println!("pivot pushdown: {ran} cases, {} failures", failures.len());
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The source says it can pivot now — which is what makes the planner pick it.
#[test]
fn postgresql_declares_that_it_can_pivot() {
    use opengrid_datasource::SendDataSource;
    let source =
        PostgresDataSource::connect("host=localhost dbname=postgres", "orders", pg::schema())
            .expect("a source needs no connection to declare itself");
    assert!(SendDataSource::capabilities(&source).pivot);
}
