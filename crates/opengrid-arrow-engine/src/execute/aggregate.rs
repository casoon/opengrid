//! Grouping and aggregation, rules S10–S12.
//!
//! Plan point 08. [`run`] takes the filtered batch and answers with the result
//! batch in the columns of `query.output_schema` — group keys and aggregate
//! aliases, in the order validation fixed.
//!
//! **Group keys** are *order-preserving* byte sequences, one per group column,
//! concatenated per row. Equal keys mean equal values (rule S10: NULL forms its
//! own group; rule S14: the empty string is a value, not a NULL), and the byte
//! order is the order the rules ask for (`binary`/codepoint for strings,
//! S4/S13; `NaN` at the top and `-0.0` = `0.0` for floats, S7). That one
//! mechanism carries both the grouping `HashMap` and the comparison for
//! `min`/`max`. The plan named `arrow-row::RowConverter` for this step — the
//! crate is not available in this environment (no network), so the encoding is
//! written out here: same technique, no new dependency, see
//! plan/spezifikation/14-entscheidungen.md.
//!
//! **Accumulators** ignore NULL (rule S11) and produce exactly the types of the
//! result table (rule S12): `count` → Int64, `sum(Int64)` → Int64 with a
//! checked overflow, `sum(Decimal(p, s))` → Decimal(38, s), `sum(Float64)` →
//! Float64, `avg` → Float64, `min`/`max` → the input type. `min`/`max` keep the
//! *row* of the extreme value and take the column from there, so no value has
//! to be re-encoded. An aggregate without `group` answers with exactly one row,
//! also for an empty input (rule S11).

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::builder::{Decimal128Builder, Float64Builder, Int64Builder};
use arrow_array::types::{
    ArrowPrimitiveType, Date32Type, Decimal128Type, Float64Type, Int64Type,
    TimestampMicrosecondType,
};
use arrow_array::{Array, ArrayRef, BooleanArray, PrimitiveArray, RecordBatch, UInt32Array};
use arrow_schema::DataType as ArrowDataType;
use arrow_select::take::take;
use opengrid_query::{AggregateFn, ValidatedQuery};
use opengrid_types::{DataType, FieldName};

use super::ExecuteError;

/// Runs `group` + `aggregate` and returns the result batch.
pub(crate) fn run(
    batch: &RecordBatch,
    query: &ValidatedQuery,
) -> Result<RecordBatch, ExecuteError> {
    let groups = GroupColumns::resolve(batch, &query.group)?;
    let aggregates = Aggregates::resolve(batch, query)?;

    let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut state: Vec<Group> = Vec::new();
    for row in 0..batch.num_rows() {
        let key = groups.key(row)?;
        let group = match index.get(&key) {
            Some(group) => *group,
            None => {
                let group = state.len();
                state.push(Group::new(row, &aggregates));
                index.insert(key, group);
                group
            }
        };
        for (position, accumulator) in state[group].accumulated.iter_mut().enumerate() {
            accumulator.absorb(&aggregates, position, row)?;
        }
    }

    // Rule S11: an aggregate without `group` answers with one row, also when the
    // input is empty (`count` = 0, everything else NULL). With group keys an
    // empty input has no group to answer — no group, no row.
    if state.is_empty() && groups.is_empty() {
        state.push(Group::new(0, &aggregates));
    }

    build(query, &groups, &aggregates, &state)
}

/// One result group: the input row its keys are shown from, plus one
/// accumulator per aggregate.
struct Group {
    row: usize,
    accumulated: Vec<Accumulator>,
}

impl Group {
    fn new(row: usize, aggregates: &Aggregates) -> Self {
        Group {
            row,
            accumulated: aggregates
                .functions
                .iter()
                .enumerate()
                .map(|(position, function)| {
                    Accumulator::of(*function, aggregates.types[position].as_ref())
                })
                .collect(),
        }
    }
}

/// The input columns the query groups by.
struct GroupColumns {
    columns: Vec<ArrayRef>,
    types: Vec<DataType>,
    names: Vec<FieldName>,
}

impl GroupColumns {
    fn resolve(batch: &RecordBatch, group: &[FieldName]) -> Result<Self, ExecuteError> {
        let mut columns = Vec::with_capacity(group.len());
        let mut types = Vec::with_capacity(group.len());
        for field in group {
            let index = index_of(batch, field)?;
            columns.push(batch.column(index).clone());
            types.push(super::data_type_of(batch, index)?);
        }
        Ok(GroupColumns {
            columns,
            types,
            names: group.to_vec(),
        })
    }

    fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The key of one row: its encoded cells, in group order.
    fn key(&self, row: usize) -> Result<Vec<u8>, ExecuteError> {
        let mut key = Vec::new();
        for ((column, data_type), name) in self.columns.iter().zip(&self.types).zip(&self.names) {
            encode(column, data_type, row, name, &mut key)?;
        }
        Ok(key)
    }

    /// The key columns of the result: one row per group, taken from the group's
    /// representative input row. A NULL group keeps its NULL (rule S10).
    fn take(&self, state: &[Group]) -> Result<Vec<ArrayRef>, ExecuteError> {
        let indices = UInt32Array::from_iter_values(state.iter().map(|group| group.row as u32));
        self.columns
            .iter()
            .map(|column| Ok(take(column.as_ref(), &indices, None)?))
            .collect()
    }
}

/// The aggregates of the query, bound to their input column.
struct Aggregates {
    /// The input column; `count(*)` has none.
    columns: Vec<Option<ArrayRef>>,
    types: Vec<Option<DataType>>,
    functions: Vec<AggregateFn>,
    aliases: Vec<FieldName>,
}

impl Aggregates {
    fn resolve(batch: &RecordBatch, query: &ValidatedQuery) -> Result<Self, ExecuteError> {
        let mut columns = Vec::with_capacity(query.aggregate.len());
        let mut types = Vec::with_capacity(query.aggregate.len());
        let mut functions = Vec::with_capacity(query.aggregate.len());
        let mut aliases = Vec::with_capacity(query.aggregate.len());
        for aggregate in &query.aggregate {
            match &aggregate.field {
                Some(field) => {
                    let index = index_of(batch, field)?;
                    columns.push(Some(batch.column(index).clone()));
                    types.push(Some(super::data_type_of(batch, index)?));
                }
                None => {
                    columns.push(None);
                    types.push(None);
                }
            }
            functions.push(aggregate.function);
            aliases.push(aggregate.alias.clone());
        }
        Ok(Aggregates {
            columns,
            types,
            functions,
            aliases,
        })
    }

    /// Where the alias sits in the query's aggregate list.
    fn position_of(&self, alias: &FieldName) -> Result<usize, ExecuteError> {
        self.aliases
            .iter()
            .position(|candidate| candidate == alias)
            .ok_or_else(|| ExecuteError::MissingField {
                field: alias.to_string(),
            })
    }

    /// The result column of one aggregate.
    fn column_of(
        &self,
        position: usize,
        output_type: DataType,
        alias: &FieldName,
        state: &[Group],
    ) -> Result<ArrayRef, ExecuteError> {
        let mut rows: Vec<Option<i64>> = Vec::new();
        let mut floats: Vec<Option<f64>> = Vec::new();
        let mut decimals: Vec<Option<i128>> = Vec::new();
        let mut taken: Vec<Option<u32>> = Vec::new();

        for group in state {
            match &group.accumulated[position] {
                Accumulator::Count(count) => rows.push(Some(*count)),
                Accumulator::SumInt64 { sum, seen } => rows.push(seen.then_some(*sum)),
                Accumulator::SumFloat64 { sum, seen } => floats.push(seen.then_some(*sum)),
                Accumulator::SumDecimal { sum, seen } => decimals.push(seen.then_some(*sum)),
                // Rule S11: over an empty or all-NULL set the average is NULL.
                Accumulator::Avg { sum, count } => {
                    floats.push((*count > 0).then(|| *sum / *count as f64))
                }
                Accumulator::Extreme { key, row, .. } => {
                    taken.push(key.as_ref().map(|_| *row as u32))
                }
            }
        }

        Ok(match (self.functions[position], output_type) {
            (AggregateFn::Count, _) => Arc::new(Int64Array(rows).finish()) as ArrayRef,
            (AggregateFn::Sum, DataType::Int64) => Arc::new(Int64Array(rows).finish()),
            (AggregateFn::Sum, DataType::Float64) => Arc::new(Float64Array(floats).finish()),
            (AggregateFn::Sum, DataType::Decimal { precision, scale }) => {
                let mut builder = Decimal128Builder::with_capacity(decimals.len())
                    .with_precision_and_scale(precision, scale as i8)
                    .map_err(|error| ExecuteError::Value {
                        field: alias.to_string(),
                        message: error.to_string(),
                    })?;
                for value in &decimals {
                    match value {
                        Some(value) => builder.append_value(*value),
                        None => builder.append_null(),
                    }
                }
                Arc::new(builder.finish())
            }
            (AggregateFn::Avg, _) => Arc::new(Float64Array(floats).finish()),
            (AggregateFn::Min | AggregateFn::Max, _) => {
                let source = self.columns[position]
                    .as_ref()
                    .expect("min and max are validated to have a field");
                if source.is_empty() {
                    // An empty input with no group: the extreme is NULL, and
                    // there is no row to take it from.
                    return Ok(arrow_array::array::new_null_array(
                        &ArrowDataType::from(output_type),
                        state.len(),
                    ));
                }
                take(source.as_ref(), &UInt32Array::from_iter(taken), None)?
            }
            (function, other) => {
                // Validation only admits the types of the result table (S12),
                // so reaching this is a defect — reported, never panicked.
                return Err(ExecuteError::Value {
                    field: alias.to_string(),
                    message: format!("{} has no result type for {other}", function.as_str()),
                });
            }
        })
    }
}

/// Builders for the three accumulator results, one value per group.
struct Int64Array(Vec<Option<i64>>);

impl Int64Array {
    fn finish(self) -> arrow_array::Int64Array {
        let mut builder = Int64Builder::with_capacity(self.0.len());
        for value in &self.0 {
            match value {
                Some(value) => builder.append_value(*value),
                None => builder.append_null(),
            }
        }
        builder.finish()
    }
}

struct Float64Array(Vec<Option<f64>>);

impl Float64Array {
    fn finish(self) -> arrow_array::Float64Array {
        let mut builder = Float64Builder::with_capacity(self.0.len());
        for value in &self.0 {
            match value {
                Some(value) => builder.append_value(*value),
                None => builder.append_null(),
            }
        }
        builder.finish()
    }
}

/// What one aggregate has seen so far.
enum Accumulator {
    /// `count(*)` counts rows, `count(field)` counts non-NULL values.
    Count(i64),
    SumInt64 {
        sum: i64,
        seen: bool,
    },
    /// `Decimal(38, s)`, the scale of the input column.
    SumDecimal {
        sum: i128,
        seen: bool,
    },
    SumFloat64 {
        sum: f64,
        seen: bool,
    },
    /// `avg` is a float, whatever the input was.
    Avg {
        sum: f64,
        count: i64,
    },
    /// `min`/`max`: the order-preserving key of the extreme value and the input
    /// row it sits in, so the result can be taken from the input column.
    Extreme {
        key: Option<Vec<u8>>,
        row: usize,
        max: bool,
    },
}

impl Accumulator {
    fn of(function: AggregateFn, input: Option<&DataType>) -> Self {
        match function {
            AggregateFn::Count => Accumulator::Count(0),
            // Rule S12: the sum keeps the input's kind — Int64 stays Int64 (with
            // a checked overflow), Decimal widens to 38 digits, Float64 stays.
            AggregateFn::Sum => match input {
                Some(DataType::Float64) => Accumulator::SumFloat64 {
                    sum: 0.0,
                    seen: false,
                },
                Some(DataType::Decimal { .. }) => Accumulator::SumDecimal {
                    sum: 0,
                    seen: false,
                },
                _ => Accumulator::SumInt64 {
                    sum: 0,
                    seen: false,
                },
            },
            AggregateFn::Avg => Accumulator::Avg { sum: 0.0, count: 0 },
            AggregateFn::Min | AggregateFn::Max => Accumulator::Extreme {
                key: None,
                row: 0,
                max: function == AggregateFn::Max,
            },
        }
    }

    /// Absorbs one row — NULL is not a value, it only counts for `count(*)`.
    fn absorb(
        &mut self,
        aggregates: &Aggregates,
        position: usize,
        row: usize,
    ) -> Result<(), ExecuteError> {
        let function = aggregates.functions[position];
        let alias = &aggregates.aliases[position];
        let Some(column) = &aggregates.columns[position] else {
            // Only `count(*)` runs without a field, and it counts rows.
            if let Accumulator::Count(count) = self {
                *count += 1;
            }
            return Ok(());
        };
        if column.is_null(row) {
            return Ok(());
        }
        let data_type = aggregates.types[position]
            .as_ref()
            .expect("a column carries a type");

        match (self, function) {
            (Accumulator::Count(count), AggregateFn::Count) => *count += 1,
            (Accumulator::SumInt64 { sum, seen }, AggregateFn::Sum) => {
                *sum = sum
                    .checked_add(as_primitive::<Int64Type>(column, alias)?.value(row))
                    .ok_or_else(|| overflow(alias))?;
                *seen = true;
            }
            (Accumulator::SumDecimal { sum, seen }, AggregateFn::Sum) => {
                *sum = sum
                    .checked_add(as_primitive::<Decimal128Type>(column, alias)?.value(row))
                    .ok_or_else(|| overflow(alias))?;
                *seen = true;
            }
            (Accumulator::SumFloat64 { sum, seen }, AggregateFn::Sum) => {
                *sum += as_primitive::<Float64Type>(column, alias)?.value(row);
                *seen = true;
            }
            (Accumulator::Avg { sum, count }, AggregateFn::Avg) => {
                *sum += as_number(column, data_type, row, alias)?;
                *count += 1;
            }
            (
                Accumulator::Extreme {
                    key,
                    row: best,
                    max,
                },
                AggregateFn::Min | AggregateFn::Max,
            ) => {
                let mut candidate = Vec::new();
                encode(column, data_type, row, alias, &mut candidate)?;
                let better = match key {
                    None => true,
                    Some(current) => {
                        if *max {
                            candidate > *current
                        } else {
                            candidate < *current
                        }
                    }
                };
                if better {
                    *key = Some(candidate);
                    *best = row;
                }
            }
            _ => unreachable!("the accumulator was built for its function"),
        }
        Ok(())
    }
}

/// The result batch: the columns of `output_schema`, in that order.
fn build(
    query: &ValidatedQuery,
    groups: &GroupColumns,
    aggregates: &Aggregates,
    state: &[Group],
) -> Result<RecordBatch, ExecuteError> {
    let key_columns = groups.take(state)?;
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(query.output_schema.fields().len());

    for field in query.output_schema.fields() {
        if let Some(position) = query
            .group
            .iter()
            .position(|key| key.as_str() == field.name.as_str())
        {
            arrays.push(key_columns[position].clone());
        } else {
            let position = aggregates.position_of(&field.name)?;
            arrays.push(aggregates.column_of(position, field.data_type, &field.name, state)?);
        }
    }

    let schema = Arc::new(arrow_schema::Schema::from(&query.output_schema));
    Ok(RecordBatch::try_new(schema, arrays)?)
}

/// Writes the order-preserving key of one cell into `key`.
fn encode(
    column: &ArrayRef,
    data_type: &DataType,
    row: usize,
    field: &FieldName,
    key: &mut Vec<u8>,
) -> Result<(), ExecuteError> {
    // A NULL is its own group (rule S10) and has no place in an order.
    if column.is_null(row) {
        key.extend_from_slice(&[0x00, 0x00]);
        return Ok(());
    }
    key.push(0x01);
    match data_type {
        DataType::Bool => {
            let value = column
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| mismatch(column, field))?
                .value(row);
            key.push(u8::from(value));
        }
        DataType::Int64 => {
            key.extend_from_slice(&order_of_i64(
                as_primitive::<Int64Type>(column, field)?.value(row),
            ));
        }
        DataType::Float64 => {
            key.extend_from_slice(&order_of_f64(
                as_primitive::<Float64Type>(column, field)?.value(row),
            ));
        }
        DataType::Decimal { .. } => {
            key.extend_from_slice(&order_of_i128(
                as_primitive::<Decimal128Type>(column, field)?.value(row),
            ));
        }
        DataType::Utf8 => {
            // Escaped, so the encoding stays prefix-free *and* keeps the byte
            // order (rules S4/S13: binary, no Unicode normalisation).
            for byte in arrow_array::cast::as_string_array(column)
                .value(row)
                .as_bytes()
            {
                match byte {
                    0x00 => key.extend_from_slice(&[0x00, 0xFF]),
                    other => key.push(*other),
                }
            }
            key.extend_from_slice(&[0x00, 0x00]);
        }
        DataType::Date => {
            key.extend_from_slice(&order_of_i32(
                as_primitive::<Date32Type>(column, field)?.value(row),
            ));
        }
        DataType::Timestamp => {
            key.extend_from_slice(&order_of_i64(
                as_primitive::<TimestampMicrosecondType>(column, field)?.value(row),
            ));
        }
    }
    Ok(())
}

/// Big-endian with the sign bit flipped: unsigned byte order == number order.
fn order_of_i64(value: i64) -> [u8; 8] {
    (value ^ i64::MIN).to_be_bytes()
}

fn order_of_i32(value: i32) -> [u8; 4] {
    (value ^ i32::MIN).to_be_bytes()
}

fn order_of_i128(value: i128) -> [u8; 16] {
    (value ^ i128::MIN).to_be_bytes()
}

/// IEEE-754 bytes in total order with the two quirks of rule S7 folded in:
/// every NaN is the same NaN, `-0.0` is `0.0`. That makes the byte order agree
/// with the comparison rules and the group key agree with equality.
fn order_of_f64(value: f64) -> [u8; 8] {
    let bits = if value.is_nan() {
        f64::NAN.to_bits()
    } else if value == 0.0 {
        0.0f64.to_bits()
    } else {
        value.to_bits()
    };
    let ordered = if bits & (1 << 63) == 0 {
        bits ^ (1 << 63)
    } else {
        !bits
    };
    ordered.to_be_bytes()
}

fn index_of(batch: &RecordBatch, field: &FieldName) -> Result<usize, ExecuteError> {
    batch
        .schema()
        .index_of(field.as_str())
        .map_err(|_| ExecuteError::MissingField {
            field: field.to_string(),
        })
}

fn overflow(alias: &FieldName) -> ExecuteError {
    ExecuteError::Value {
        field: alias.to_string(),
        message: "the sum does not fit into the result type any more".to_owned(),
    }
}

fn mismatch(column: &ArrayRef, field: &FieldName) -> ExecuteError {
    ExecuteError::TypeMismatch {
        field: field.to_string(),
        expected: DataType::Utf8,
        found: column.data_type().clone(),
    }
}

/// A typed column, or a type error naming the field it came from.
fn as_primitive<'a, T: ArrowPrimitiveType>(
    column: &'a ArrayRef,
    field: &FieldName,
) -> Result<&'a PrimitiveArray<T>, ExecuteError> {
    column
        .as_any()
        .downcast_ref::<PrimitiveArray<T>>()
        .ok_or_else(|| ExecuteError::TypeMismatch {
            field: field.to_string(),
            expected: DataType::Utf8,
            found: column.data_type().clone(),
        })
}

/// `avg` over Int64, Decimal or Float64 is a float (rule S12).
fn as_number(
    column: &ArrayRef,
    data_type: &DataType,
    row: usize,
    field: &FieldName,
) -> Result<f64, ExecuteError> {
    Ok(match data_type {
        DataType::Int64 => as_primitive::<Int64Type>(column, field)?.value(row) as f64,
        DataType::Float64 => as_primitive::<Float64Type>(column, field)?.value(row),
        DataType::Decimal { scale, .. } => {
            let value = as_primitive::<Decimal128Type>(column, field)?.value(row);
            value as f64 / 10f64.powi(*scale as i32)
        }
        other => {
            return Err(ExecuteError::Value {
                field: field.to_string(),
                message: format!("cannot average {other}"),
            });
        }
    })
}
