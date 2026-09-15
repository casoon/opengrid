//! Helpers the engine's integration tests share.
//!
//! Test-only code, so it lives next to the tests instead of in the crate. Each
//! test binary compiles its own copy and uses only part of it — hence the
//! `allow`: the workspace denies warnings, and an unused helper in one binary is
//! not a defect.

#![allow(dead_code)]

use std::path::PathBuf;

use arrow_array::cast::{as_boolean_array, as_primitive_array, as_string_array};
use arrow_array::types::{
    Date32Type, Decimal128Type, Float64Type, Int64Type, TimestampMicrosecondType,
};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{DataType as ArrowDataType, TimeUnit};
use opengrid_arrow_engine::ingest::{CsvOptions, JsonOptions, load_csv, load_json};
use opengrid_conformance::load_schema;
use opengrid_types::{Date, Decimal, Schema, Timestamp, Value};

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

/// The dataset as CSV batches, read through ingest (point 06).
pub fn csv_batches_with(options: CsvOptions) -> Vec<RecordBatch> {
    load_csv(&data("orders.csv"), &schema(), options).expect("the CSV loads")
}

/// The same, with the default dialect.
pub fn csv_batches() -> Vec<RecordBatch> {
    csv_batches_with(CsvOptions::default())
}

/// The dataset as JSON batches — the twin of [`csv_batches`].
pub fn json_batches() -> Vec<RecordBatch> {
    load_json(&data("orders.json"), &schema(), JsonOptions::default()).expect("the JSON loads")
}

/// Arrow back to typed values, so a test can compare and assert with [`Value`].
pub fn decode(batches: &[RecordBatch]) -> Vec<Vec<Value>> {
    batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(move |row| {
                (0..batch.num_columns())
                    .map(|column| {
                        let data_type = batch.schema().field(column).data_type().clone();
                        value_at(&batch.column(column).clone(), row, &data_type)
                    })
                    .collect::<Vec<Value>>()
            })
        })
        .collect()
}

/// One cell, decoded by the column's Arrow type.
pub fn value_at(array: &ArrayRef, index: usize, data_type: &ArrowDataType) -> Value {
    if array.is_null(index) {
        return Value::Null;
    }
    match data_type {
        ArrowDataType::Boolean => Value::Bool(as_boolean_array(array).value(index)),
        ArrowDataType::Int64 => Value::Int64(as_primitive_array::<Int64Type>(array).value(index)),
        ArrowDataType::Float64 => {
            Value::Float64(as_primitive_array::<Float64Type>(array).value(index))
        }
        ArrowDataType::Decimal128(_, scale) => Value::Decimal(Decimal::new(
            as_primitive_array::<Decimal128Type>(array).value(index),
            *scale as u8,
        )),
        ArrowDataType::Utf8 => Value::Utf8(as_string_array(array).value(index).to_owned()),
        ArrowDataType::Date32 => Value::Date(Date::from_days_since_epoch(
            as_primitive_array::<Date32Type>(array).value(index),
        )),
        ArrowDataType::Timestamp(TimeUnit::Microsecond, _) => {
            Value::Timestamp(Timestamp::from_micros(
                as_primitive_array::<TimestampMicrosecondType>(array).value(index),
            ))
        }
        other => panic!("unsupported column type {other}"),
    }
}

/// The row with this `id` — the dataset's primary key, in file order.
pub fn row(rows: &[Vec<Value>], id: i64) -> &Vec<Value> {
    rows.iter()
        .find(|row| row[0] == Value::Int64(id))
        .unwrap_or_else(|| panic!("row with id {id}"))
}
