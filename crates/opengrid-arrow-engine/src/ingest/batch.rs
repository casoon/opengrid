//! Building Arrow batches from typed rows.

use std::sync::Arc;

use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::Schema as ArrowSchema;
use opengrid_types::{DataType, Field, Schema, TIMESTAMP_TIMEZONE, Value};

/// Builds one batch from rows that belong to `schema`.
///
/// The values went through [`super::cell`] before, so a mismatch here means the
/// caller is broken — it is reported as an error rather than a panic, because
/// ingest is the path untrusted input takes.
pub(crate) fn build(
    schema: &Schema,
    arrow: &ArrowSchema,
    rows: &[Vec<Value>],
) -> Result<RecordBatch, String> {
    let arrays = schema
        .fields()
        .iter()
        .enumerate()
        .map(|(column, field)| array(field, column, rows))
        .collect::<Result<Vec<ArrayRef>, String>>()?;
    RecordBatch::try_new(Arc::new(arrow.clone()), arrays).map_err(|error| error.to_string())
}

/// The values of one column, top to bottom.
fn column(column: usize, rows: &[Vec<Value>]) -> impl Iterator<Item = &Value> {
    rows.iter().map(move |row| &row[column])
}

fn array(field: &Field, index: usize, rows: &[Vec<Value>]) -> Result<ArrayRef, String> {
    match field.data_type {
        DataType::Bool => {
            let mut builder = BooleanBuilder::with_capacity(rows.len());
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Bool(value) => builder.append_value(*value),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Int64 => {
            let mut builder = Int64Builder::with_capacity(rows.len());
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Int64(value) => builder.append_value(*value),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        // A NaN is appended as a value, not as a null (rule S7).
        DataType::Float64 => {
            let mut builder = Float64Builder::with_capacity(rows.len());
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Float64(value) => builder.append_value(*value),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Decimal { precision, scale } => {
            let mut builder = Decimal128Builder::with_capacity(rows.len())
                .with_precision_and_scale(precision, scale as i8)
                .map_err(|error| error.to_string())?;
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Decimal(value) => builder.append_value(value.value()),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Utf8 => {
            let mut builder = StringBuilder::with_capacity(rows.len(), 0);
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Utf8(value) => builder.append_value(value),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Date => {
            let mut builder = Date32Builder::with_capacity(rows.len());
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Date(value) => builder.append_value(value.days_since_epoch()),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
        DataType::Timestamp => {
            let mut builder = TimestampMicrosecondBuilder::with_capacity(rows.len())
                .with_timezone(TIMESTAMP_TIMEZONE);
            for value in column(index, rows) {
                match value {
                    Value::Null => builder.append_null(),
                    Value::Timestamp(value) => builder.append_value(value.micros()),
                    other => return Err(mismatch(field, other)),
                }
            }
            Ok(Arc::new(builder.finish()))
        }
    }
}

/// The value does not belong to the column it sits in.
fn mismatch(field: &Field, value: &Value) -> String {
    format!(
        "column {} expects {}, found {value:?}",
        field.name.as_str(),
        field.data_type
    )
}
