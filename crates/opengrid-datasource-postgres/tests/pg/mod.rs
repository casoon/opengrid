//! What the PostgreSQL tests share: a connection, the fixture table, the data
//! set and the local engine to compare against.
//!
//! Test-only code, so it lives next to the tests. Each test binary compiles its
//! own copy and uses part of it — hence the `allow`.

#![allow(dead_code)]

use opengrid_conformance::block_on;
use opengrid_datasource::{QueryResult, SendDataSource};
use opengrid_datasource_postgres::pg_type;
use opengrid_types::{DataType, Schema, Value};

/// The connection string, or `None` when this machine has no database.
pub fn connection() -> Option<String> {
    if let Ok(url) = std::env::var("OPENGRID_TEST_PG") {
        return Some(url);
    }
    // The usual local setup: a server on the default socket, the user's own
    // database. `PG*` variables still apply — libpq reads them itself.
    Some("host=localhost dbname=postgres".to_owned())
}

pub fn suite_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("opengrid-conformance")
}

pub fn schema() -> Schema {
    opengrid_conformance::load_schema(&suite_dir().join("data/orders.schema.json"))
        .expect("the conformance schema")
}

/// The data set, read through the local engine — so both sides provably start
/// from the same 50 rows.
pub fn fixture_rows(schema: &Schema) -> QueryResult {
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
pub async fn create_fixture(
    client: &tokio_postgres::Client,
    schema: &Schema,
    table: &str,
) -> String {
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

/// Connects, or explains why the test is skipped.
pub async fn connect() -> Option<(tokio_postgres::Client, String)> {
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

/// The same data set through the local engine — the other side of every
/// comparison in this crate.
pub fn local_source(schema: &Schema) -> opengrid_arrow_engine::datasource::LocalDataSource {
    use opengrid_arrow_engine::datasource::LocalDataSource;
    use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};

    let csv = std::fs::read(suite_dir().join("data/orders.csv")).expect("the conformance data");
    LocalDataSource::new(load_csv(&csv, schema, CsvOptions::default()).expect("ingest"))
        .expect("a local source")
}
