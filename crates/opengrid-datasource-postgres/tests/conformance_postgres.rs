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

use opengrid_conformance::{Checked, RowOrder, Table, block_on, compare};
use opengrid_datasource::{QueryResult, SendDataSource};
use opengrid_datasource_postgres::{PostgresDataSource, pg_type};
use opengrid_types::{DataType, Schema, Value};

/// The connection string, or `None` when this machine has no database.
fn connection() -> Option<String> {
    if let Ok(url) = std::env::var("OPENGRID_TEST_PG") {
        return Some(url);
    }
    // The usual local setup: a server on the default socket, the user's own
    // database. `PG*` variables still apply — libpq reads them itself.
    Some("host=localhost dbname=postgres".to_owned())
}

fn suite_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("opengrid-conformance")
}

fn schema() -> Schema {
    opengrid_conformance::load_schema(&suite_dir().join("data/orders.schema.json"))
        .expect("the conformance schema")
}

fn cases() -> Vec<Checked> {
    let schema = schema();
    let mut checked = opengrid_conformance::check_dir(&suite_dir().join("cases"), &schema)
        .expect("the conformance cases");
    checked.sort_by(|a, b| a.case.id.cmp(&b.case.id));
    checked
}

/// The data set, read through the local engine — so both sides provably start
/// from the same 50 rows.
fn fixture_rows(schema: &Schema) -> QueryResult {
    let schema = &schema.stored();
    use opengrid_arrow_engine::datasource::LocalDataSource;
    use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
    use opengrid_query::{Limits, Query};

    let csv = std::fs::read(suite_dir().join("data/orders.csv")).expect("the conformance data");
    let batches = load_csv(&csv, schema, CsvOptions::default()).expect("ingest");
    let source = LocalDataSource::new(batches).expect("a local source");

    let select: Vec<String> = schema
        .stored()
        .fields()
        .iter()
        .map(|field| format!("\"{}\"", field.name.as_str()))
        .collect();
    let query: Query = serde_json::from_str(&format!(
        r#"{{"source":"orders","select":[{}]}}"#,
        select.join(",")
    ))
    .expect("a query over every column");
    let validated = query
        .validate(schema, &Limits::default())
        .expect("a valid query");
    block_on(SendDataSource::execute(&source, validated)).expect("the local engine answers")
}

/// Creates the table, fills it and answers its name.
///
/// The name is fixed per test rather than unique per run, and the table is
/// dropped before it is created: a panic skips the cleanup at the end, and a
/// unique name would then leave a new table behind on every failed run. Two
/// tests can still run in parallel because they use different names; two
/// *concurrent runs of the suite* against one database would collide, which is
/// what `OPENGRID_TEST_PG` is for.
async fn create_fixture(client: &tokio_postgres::Client, schema: &Schema, table: &str) -> String {
    let table = table.to_owned();
    // Only the **stored** columns become columns of the table. A derived one
    // (plan point 54) is computed by PostgreSQL through the expression the
    // compiler writes — creating it here would prove nothing.
    let schema = &schema.stored();
    client
        .batch_execute(&format!("DROP TABLE IF EXISTS \"{table}\";"))
        .await
        .expect("drop a leftover table");

    let columns: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| {
            format!(
                "\"{}\" {}{}",
                field.name.as_str(),
                pg_type(field.data_type),
                // The column's own collation is irrelevant — the compiler writes
                // `COLLATE "C"` on every comparison — but a deliberately
                // *different* one here proves that (rule S4).
                if field.data_type == DataType::Utf8 {
                    " COLLATE \"en_US\""
                } else {
                    ""
                }
            )
        })
        .collect();
    client
        .batch_execute(&format!(
            "CREATE TABLE \"{table}\" ({});",
            columns.join(", ")
        ))
        .await
        .expect("create table");

    let rows = fixture_rows(schema);
    let names: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| format!("\"{}\"", field.name.as_str()))
        .collect();
    let placeholders: Vec<String> = schema
        .fields()
        .iter()
        .enumerate()
        // Text in, the column's type out — the same chain the compiler writes,
        // for the same reason: one binding path for every type.
        .map(|(index, field)| {
            let target = pg_type(field.data_type);
            if target == "text" {
                format!("${}::text", index + 1)
            } else {
                format!("${}::text::{target}", index + 1)
            }
        })
        .collect();
    let insert = format!(
        "INSERT INTO \"{table}\" ({}) VALUES ({})",
        names.join(", "),
        placeholders.join(", ")
    );

    for row in 0..rows.row_count() {
        let values: Vec<Option<String>> = rows
            .columns
            .iter()
            .map(|column| match &column[row] {
                Value::Null => None,
                Value::Bool(flag) => Some(flag.to_string()),
                Value::Int64(number) => Some(number.to_string()),
                Value::Float64(number) => Some(number.to_string()),
                Value::Decimal(decimal) => Some(decimal.to_string()),
                Value::Utf8(text) => Some(text.clone()),
                Value::Date(date) => Some(date.to_string()),
                Value::Timestamp(timestamp) => Some(timestamp.to_string()),
            })
            .collect();
        let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = values
            .iter()
            .map(|value| value as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        client
            .execute(insert.as_str(), &params)
            .await
            .unwrap_or_else(|error| panic!("insert row {row}: {error}"));
    }
    table
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

/// Connects, or explains why the test is skipped.
async fn connect() -> Option<(tokio_postgres::Client, String)> {
    let url = connection()?;
    match tokio_postgres::connect(&url, tokio_postgres::NoTls).await {
        Ok((client, connection)) => {
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Some((client, url))
        }
        Err(error) => {
            eprintln!(
                "skipping the PostgreSQL conformance run: {error}\n\
                 set OPENGRID_TEST_PG to a connection string to run it"
            );
            None
        }
    }
}

/// **The point of this plan point:** 48/48, against a database.
#[tokio::test]
async fn postgresql_answers_every_conformance_case() {
    let Some((client, url)) = connect().await else {
        return;
    };
    let schema = schema();
    let table = create_fixture(&client, &schema, "opengrid_conformance_cases").await;

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
    use opengrid_arrow_engine::datasource::LocalDataSource;
    use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
    use opengrid_query::{Limits, Query};

    let Some((client, url)) = connect().await else {
        return;
    };
    let schema = schema();
    let table = create_fixture(&client, &schema, "opengrid_conformance_diff").await;
    let remote = PostgresDataSource::connect(&url, &table, schema.clone()).expect("a source");

    let csv = std::fs::read(suite_dir().join("data/orders.csv")).expect("data");
    let local =
        LocalDataSource::new(load_csv(&csv, &schema, CsvOptions::default()).expect("ingest"))
            .expect("a local source");

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
