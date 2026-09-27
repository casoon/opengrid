//! Helpers the engine's integration tests share.
//!
//! Test-only code, so it lives next to the tests instead of in the crate. Each
//! test binary compiles its own copy and uses only part of it — hence the
//! `allow`: the workspace denies warnings, and an unused helper in one binary is
//! not a defect.

#![allow(dead_code)]

use std::path::PathBuf;

use opengrid_conformance::load_schema;
use opengrid_engine::Table;
use opengrid_engine::ingest::{CsvOptions, load_csv, load_json};
use opengrid_types::{Schema, Value};

/// The conformance dataset, next to the suite that owns it.
fn data_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../opengrid-conformance/data")
        .join(name)
}

/// One file of the dataset.
pub fn data(name: &str) -> Vec<u8> {
    let path = data_path(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The dataset schema, as the conformance suite declares it.
pub fn schema() -> Schema {
    load_schema(&data_path("orders.schema.json")).expect("the dataset schema")
}

/// The dataset as a CSV table, read through ingest (point 06).
pub fn csv_table_with(options: CsvOptions) -> Table {
    load_csv(&data("orders.csv"), &schema(), options).expect("the CSV loads")
}

/// The same, with the default dialect.
pub fn csv_table() -> Table {
    csv_table_with(CsvOptions::default())
}

/// The dataset as a JSON table — the twin of [`csv_table`].
pub fn json_table() -> Table {
    load_json(&data("orders.json"), &schema()).expect("the JSON loads")
}

/// A table back to typed values, row by row, so a test can compare and assert
/// with [`Value`].
pub fn decode(table: &Table) -> Vec<Vec<Value>> {
    (0..table.num_rows())
        .map(|row| {
            (0..table.schema().len())
                .map(|column| table.column_at(column).value(row))
                .collect()
        })
        .collect()
}

/// The row with this `id` — the dataset's primary key, in file order.
pub fn row(rows: &[Vec<Value>], id: i64) -> &Vec<Value> {
    rows.iter()
        .find(|row| row[0] == Value::Int64(id))
        .unwrap_or_else(|| panic!("row with id {id}"))
}
