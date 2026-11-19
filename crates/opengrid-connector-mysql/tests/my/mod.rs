//! What the MySQL tests share: a connection, the fixture tables, the data.
//!
//! The tests need a MySQL 8 and **skip themselves** when there is none, so
//! `just check` stays runnable on a machine without one. Set
//! `OPENGRID_TEST_MYSQL` to a URL (`mysql://root:pw@127.0.0.1:3306/opengrid`).

#![allow(dead_code)]

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, Params, Value as My};
use opengrid_conformance::block_on;
use opengrid_datasource::{QueryResult, SendDataSource};
use opengrid_types::{DataType, Schema, Value};

/// The column type the fixture table creates for an opengrid type.
///
/// Written out here and not borrowed from the dialect, so the two have to
/// meet in the database. A string column gets a deliberately *different*
/// collation, accent- and case-insensitive: the compiler's
/// `utf8mb4_0900_bin` on every comparison is what makes the answers right
/// (rule S4).
fn mysql_type(data_type: DataType) -> String {
    match data_type {
        DataType::Bool => "BOOLEAN".to_owned(),
        DataType::Int64 => "BIGINT".to_owned(),
        DataType::Float64 => "DOUBLE".to_owned(),
        DataType::Decimal { precision, scale } => format!("DECIMAL({precision},{scale})"),
        DataType::Utf8 => "TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci".to_owned(),
        DataType::Date => "DATE".to_owned(),
        DataType::Timestamp => "TIMESTAMP(6) NULL".to_owned(),
        DataType::ListUtf8 | DataType::ListInt64 => "JSON".to_owned(),
    }
}

/// The URL, or `None` when this machine has no database for the tests.
pub fn url() -> Option<String> {
    match std::env::var("OPENGRID_TEST_MYSQL") {
        Ok(url) => Some(url),
        Err(_) => {
            eprintln!("skipping the MySQL tests: set OPENGRID_TEST_MYSQL to a mysql:// URL");
            None
        }
    }
}

/// A connection with the session the connector's has: UTC, utf8mb4.
pub async fn connect(url: &str) -> Conn {
    let opts = OptsBuilder::from_opts(Opts::from_url(url).expect("a URL"))
        .setup(vec!["SET time_zone = '+00:00', NAMES utf8mb4"]);
    Conn::new(opts).await.expect("a connection to MySQL")
}

pub fn suite_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("opengrid-conformance")
}

pub fn schema_of(name: &str) -> Schema {
    opengrid_conformance::load_schema(&suite_dir().join("data").join(name))
        .expect("the suite's schema")
}

/// The data set `csv` of the suite, read through the local engine, so both
/// sides provably start from the same rows.
pub fn rows_of(schema: &Schema, csv: &str) -> QueryResult {
    use opengrid_engine::datasource::LocalDataSource;
    use opengrid_engine::ingest::{CsvOptions, load_csv};
    use opengrid_query::{Limits, Query};

    let schema = &schema.stored();
    let csv = std::fs::read(suite_dir().join("data").join(csv)).expect("the data");
    let source =
        LocalDataSource::new(load_csv(&csv, schema, CsvOptions::default()).expect("ingest"));
    let select: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| format!("\"{}\"", field.name.as_str()))
        .collect();
    let query: Query = opengrid_json::from_str(&format!(
        r#"{{"source":"t","select":[{}]}}"#,
        select.join(",")
    ))
    .expect("a query over every column");
    let validated = query.validate(schema, &Limits::default()).expect("valid");
    block_on(SendDataSource::execute(&source, validated)).expect("the engine answers")
}

/// Creates the table `table` from the suite's `csv` under `schema` (stored
/// columns only: a derived one is MySQL's to compute) and fills it.
///
/// A NaN cannot go into a `DOUBLE`: it goes in as NULL, and the cases that
/// read it are named in the tests.
pub async fn create(connection: &mut Conn, schema: &Schema, table: &str, csv: &str) {
    let schema = &schema.stored();
    connection
        .query_drop(format!("DROP TABLE IF EXISTS `{table}`"))
        .await
        .expect("drop a leftover table");
    let columns: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| format!("`{}` {}", field.name.as_str(), mysql_type(field.data_type)))
        .collect();
    connection
        .query_drop(format!("CREATE TABLE `{table}` ({})", columns.join(", ")))
        .await
        .expect("create table");

    let rows = rows_of(schema, csv);
    let names: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| format!("`{}`", field.name.as_str()))
        .collect();
    let insert = format!(
        "INSERT INTO `{table}` ({}) VALUES ({})",
        names.join(", "),
        vec!["?"; names.len()].join(", ")
    );
    for row in 0..rows.row_count() {
        let values: Vec<My> = rows
            .columns
            .iter()
            .map(|column| match &column[row] {
                Value::Null => My::NULL,
                Value::Bool(flag) => My::Int(i64::from(*flag)),
                Value::Int64(number) => My::Int(*number),
                Value::Float64(number) if number.is_nan() => My::NULL,
                Value::Float64(number) => My::Double(*number),
                Value::Decimal(decimal) => My::from(decimal.to_string()),
                Value::Utf8(text) => My::from(text.as_str()),
                Value::Date(date) => My::from(date.to_string()),
                Value::Timestamp(timestamp) => My::from(
                    timestamp
                        .to_string()
                        .trim_end_matches('Z')
                        .replacen('T', " ", 1),
                ),
                list @ Value::List(_) => My::from(opengrid_json::ToJson::to_json(list).to_string()),
            })
            .collect();
        connection
            .exec_drop(&insert, Params::Positional(values))
            .await
            .unwrap_or_else(|error| panic!("insert row {row}: {error}"));
    }
}

pub async fn drop_table(connection: &mut Conn, table: &str) {
    connection
        .query_drop(format!("DROP TABLE IF EXISTS `{table}`"))
        .await
        .expect("drop table");
}
