//! The conformance suite against a real PostgreSQL (plan point 26).
//!
//! This is the test risk R4 exists for: the same 48 cases, the same expectations,
//! answered once by the Arrow engine in the browser and once by a database, and
//! the two must agree exactly — NULL ordering, binary collation, exact decimals,
//! NaN, microseconds and the NFC/NFD pair included.
//!
//! # Getting a database
//!
//! The tests need one and **skip themselves** when there is none, so
//! `just check` stays runnable on a machine without PostgreSQL. Set
//! `OPENGRID_TEST_PG` to a libpq connection string, or have a local server that
//! the usual `PG*` environment variables reach.
//!
//! Every run creates its own table with a unique name and drops it afterwards,
//! so a shared database is safe and two runs do not collide.

mod pg;

use opengrid_conformance::{Checked, RowOrder, Table, block_on, compare};
use opengrid_datasource::{QueryResult, SendDataSource};
use opengrid_datasource_postgres::PostgresDataSource;

fn cases() -> Vec<Checked> {
    let schema = pg::schema();
    let mut checked = opengrid_conformance::check_dir(&pg::suite_dir().join("cases"), &schema)
        .expect("the conformance cases");
    checked.sort_by(|a, b| a.case.id.cmp(&b.case.id));
    checked
}

/// A result as the suite's comparison sees it.
fn as_table(result: &QueryResult) -> Table {
    let columns = result
        .schema
        .fields()
        .iter()
        .map(|field| field.name.clone())
        .collect();
    let rows = (0..result.row_count())
        .map(|row| result.columns.iter().map(|c| c[row].clone()).collect())
        .collect();
    Table::new(columns, rows)
}

/// **The point of this plan point:** 48/48, against a database.
#[tokio::test]
async fn postgresql_answers_every_conformance_case() {
    let Some((client, url)) = pg::connect().await else {
        return;
    };
    let schema = pg::schema();
    let table = pg::create_fixture(&client, &schema, "opengrid_conformance_cases").await;

    let source = PostgresDataSource::connect(&url, &table, schema).expect("a source");
    let mut failures = Vec::new();
    let mut ran = 0;

    for checked in cases() {
        let result = SendDataSource::execute(&source, checked.query.clone()).await;
        match result {
            Ok(result) => {
                let order = if checked.case.ordered {
                    RowOrder::Ordered
                } else {
                    RowOrder::Unordered
                };
                if let Err(difference) = compare(&checked.expected, &as_table(&result), order) {
                    failures.push(format!("{}: {difference}", checked.case.id));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", checked.case.id)),
        }
        ran += 1;
    }

    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");

    let version: String = client
        .query_one("select version()", &[])
        .await
        .map(|row| row.get(0))
        .unwrap_or_default();
    println!(
        "conformance against {}: {ran} cases, {} failures",
        version.split(" on ").next().unwrap_or("PostgreSQL"),
        failures.len()
    );
    assert_eq!(ran, 53, "the suite has 53 cases");
    assert!(
        failures.is_empty(),
        "{} of {ran} cases differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The differential test of risk R4: the same query, both engines, cell by cell.
///
/// Beyond the 48 cases, over generated queries — every operator against every
/// column type, which is where a compiler quietly disagrees with the engine.
#[tokio::test]
async fn both_engines_answer_the_same_thing() {
    use opengrid_query::{Limits, Query};

    let Some((client, url)) = pg::connect().await else {
        return;
    };
    let schema = pg::schema();
    let table = pg::create_fixture(&client, &schema, "opengrid_conformance_diff").await;
    let remote = PostgresDataSource::connect(&url, &table, schema.clone()).expect("a source");

    let local = pg::local_source(&schema);

    // One literal per column type that exists in the data set.
    let probes: &[(&str, &str)] = &[
        ("id", "25"),
        ("customer", "\"Alpha\""),
        ("country", "\"DE\""),
        ("amount", "\"10.00\""),
        ("qty", "5"),
        ("ratio", "1.5"),
        ("note", "\"\""),
        ("ordered_on", "\"2024-12-31\""),
        ("created_at", "\"2024-06-01T12:00:00.000000Z\""),
    ];
    let operators = ["eq", "ne", "lt", "lte", "gt", "gte"];

    let mut failures = Vec::new();
    let mut ran = 0;
    for (column, literal) in probes {
        for op in operators {
            let json = format!(
                r#"{{"source":"orders","select":["id"],
                     "filter":{{"field":"{column}","op":"{op}","value":{literal}}},
                     "sort":[{{"field":"id","direction":"asc"}}]}}"#
            );
            let query: Query = serde_json::from_str(&json).expect("a query");
            let Ok(validated) = query.validate(&schema, &Limits::default()) else {
                // Not every operator fits every type — the validator says so, and
                // that is a case the compiler never sees.
                continue;
            };
            ran += 1;

            let here = block_on(SendDataSource::execute(&local, validated.clone()))
                .expect("the local engine answers");
            let there = SendDataSource::execute(&remote, validated)
                .await
                .unwrap_or_else(|error| panic!("{column} {op}: {error}"));

            if let Err(difference) = compare(&as_table(&here), &as_table(&there), RowOrder::Ordered)
            {
                failures.push(format!("{column} {op}: {difference}"));
            }
            if here.total_count != there.total_count {
                failures.push(format!(
                    "{column} {op}: total_count {} here, {} there",
                    here.total_count, there.total_count
                ));
            }
        }
    }

    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");

    println!(
        "differential: {ran} comparisons, {} failures",
        failures.len()
    );
    assert!(ran >= 40, "expected a broad sweep, ran {ran}");
    assert!(
        failures.is_empty(),
        "{} of {ran} comparisons differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
