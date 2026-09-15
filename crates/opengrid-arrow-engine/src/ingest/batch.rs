//! Arrow batches and typed rows, in both directions.
//!
//! [`build`] is the ingest direction (rows → batch), [`decode`] the output
//! direction (batch → values) that the datasource adapter uses. Both walk the
//! same seven types, so the mapping stays in one file.

use std::sync::Arc;

use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    TimestampMicrosecondBuilder,
};
use arrow_array::types::{
    ArrowPrimitiveType, Date32Type, Decimal128Type, Float64Type, Int64Type,
    TimestampMicrosecondType,
};
use arrow_array::{Array, ArrayRef, BooleanArray, PrimitiveArray, RecordBatch, StringArray};
use arrow_schema::Schema as ArrowSchema;
use opengrid_types::{
    DataType, Date, Decimal, Field, Schema, TIMESTAMP_TIMEZONE, Timestamp, Value,
};

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

/// The inverse of [`build`]: the typed values of one batch, one `Vec` per column.
///
/// The column-oriented form of the wire format (E6) and of the Arrow-free
/// `QueryResult` (E14) — this is the only place the executor's batches turn back
/// into `opengrid_types::Value`s. A column whose Arrow type the query model does
/// not know is an error, not a guess.
pub(crate) fn decode(batch: &RecordBatch) -> Result<Vec<Vec<Value>>, String> {
    let arrows = batch.schema();
    let rows = batch.num_rows();
    let mut columns = Vec::with_capacity(batch.num_columns());
    for (index, field) in arrows.fields().iter().enumerate() {
        let data_type = DataType::from_arrow(field.data_type()).ok_or_else(|| {
            format!(
                "column {} has the unknown type {}",
                field.name(),
                field.data_type()
            )
        })?;
        let array = batch.column(index);
        let mut values = Vec::with_capacity(rows);
        for row in 0..rows {
            values.push(value_of(array, row, data_type)?);
        }
        columns.push(values);
    }
    Ok(columns)
}

/// One cell, read through the type table.
fn value_of(array: &ArrayRef, row: usize, data_type: DataType) -> Result<Value, String> {
    if array.is_null(row) {
        return Ok(Value::Null);
    }
    Ok(match data_type {
        DataType::Bool => Value::Bool(boolean(array)?.value(row)),
        DataType::Int64 => Value::Int64(primitive::<Int64Type>(array)?.value(row)),
        // A NaN travels as a NaN, not as a null (rule S7).
        DataType::Float64 => Value::Float64(primitive::<Float64Type>(array)?.value(row)),
        DataType::Decimal { scale, .. } => Value::Decimal(Decimal::new(
            primitive::<Decimal128Type>(array)?.value(row),
            scale,
        )),
        DataType::Utf8 => Value::Utf8(string(array)?.value(row).to_owned()),
        DataType::Date => Value::Date(Date::from_days_since_epoch(
            primitive::<Date32Type>(array)?.value(row),
        )),
        DataType::Timestamp => Value::Timestamp(Timestamp::from_micros(
            primitive::<TimestampMicrosecondType>(array)?.value(row),
        )),
    })
}

/// The array behind a column of the expected physical type.
fn primitive<T: ArrowPrimitiveType>(array: &ArrayRef) -> Result<&PrimitiveArray<T>, String> {
    array
        .as_any()
        .downcast_ref::<PrimitiveArray<T>>()
        .ok_or_else(|| {
            format!(
                "column is {}, but the type table expects {}",
                array.data_type(),
                T::DATA_TYPE
            )
        })
}

/// The array behind a Utf8 column.
fn string(array: &ArrayRef) -> Result<&StringArray, String> {
    array.as_any().downcast_ref::<StringArray>().ok_or_else(|| {
        format!(
            "column is {}, but the type table expects Utf8",
            array.data_type()
        )
    })
}

/// The array behind a Bool column, which is not a primitive array in arrow-rs.
fn boolean(array: &ArrayRef) -> Result<&BooleanArray, String> {
    array
        .as_any()
        .downcast_ref::<BooleanArray>()
        .ok_or_else(|| {
            format!(
                "column is {}, but the type table expects Boolean",
                array.data_type()
            )
        })
}
