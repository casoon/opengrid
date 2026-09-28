//! Grouping and aggregation, rules S10–S12.
//!
//! Plan point 08. [`run`] takes the filtered table and answers with the result
//! table in the columns of `query.output_schema` — group keys and aggregate
//! aliases, in the order validation fixed.
//!
//! **Group keys** are *order-preserving* byte sequences, one per group column,
//! concatenated per row. Equal keys mean equal values (rule S10: NULL forms its
//! own group; rule S14: the empty string is a value, not a NULL), and the byte
//! order is the order the rules ask for (`binary`/codepoint for strings,
//! S4/S13; `NaN` at the top and `-0.0` = `0.0` for floats, S7). That one
//! mechanism carries both the grouping `HashMap` and the comparison for
//! `min`/`max`. It is not the sort's encoding in `opengrid-columns`: grouping
//! folds `-0.0` into `0.0` and every NaN into one, as equality does (S7), where
//! the sort keeps the total order.
//!
//! **Accumulators** ignore NULL (rule S11) and produce exactly the types of the
//! result table (rule S12): `count` → Int64, `sum(Int64)` → Int64 with a
//! checked overflow, `sum(Decimal(p, s))` → Decimal(38, s), `sum(Float64)` →
//! Float64, `avg` → Float64, `min`/`max` → the input type. `min`/`max` keep the
//! *row* of the extreme value and take the column from there, so no value has
//! to be re-encoded. An aggregate without `group` answers with exactly one row,
//! also for an empty input (rule S11).

use std::collections::HashMap;

use opengrid_columns::{Column, ColumnBuilder, Table, Values};
use opengrid_query::{AggregateFn, ValidatedQuery};
use opengrid_types::{DataType, Decimal, FieldName, Value};

use super::ExecuteError;

/// Runs `group` + `aggregate` and returns the result table.
pub(crate) fn run(table: &Table, query: &ValidatedQuery) -> Result<Table, ExecuteError> {
    let groups = GroupColumns::resolve(table, &query.group)?;
    let aggregates = Aggregates::resolve(table, query)?;

    let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut state: Vec<Group> = Vec::new();
    let mut key = Vec::new();
    for row in 0..table.num_rows() {
        key.clear();
        groups.key(row, &mut key);
        let group = match index.get(&key) {
            Some(group) => *group,
            None => {
                let group = state.len();
                state.push(Group::new(row, &aggregates));
                index.insert(key.clone(), group);
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
    fn new(row: usize, aggregates: &Aggregates<'_>) -> Self {
        Group {
            row,
            accumulated: aggregates
                .functions
                .iter()
                .enumerate()
                .map(|(position, function)| {
                    Accumulator::of(
                        *function,
                        aggregates.columns[position].map(Column::data_type),
                    )
                })
                .collect(),
        }
    }
}

/// The input columns the query groups by.
struct GroupColumns<'a> {
    columns: Vec<&'a Column>,
}

impl<'a> GroupColumns<'a> {
    fn resolve(table: &'a Table, group: &[FieldName]) -> Result<Self, ExecuteError> {
        let columns = group
            .iter()
            .map(|field| super::column(table, field.as_str()))
            .collect::<Result<_, _>>()?;
        Ok(GroupColumns { columns })
    }

    fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// The key of one row: its encoded cells, in group order.
    fn key(&self, row: usize, key: &mut Vec<u8>) {
        for column in &self.columns {
            encode(column, row, key);
        }
    }

    /// The key columns of the result: one row per group, taken from the group's
    /// representative input row. A NULL group keeps its NULL (rule S10).
    fn take(&self, state: &[Group]) -> Vec<Column> {
        let rows: Vec<u32> = state.iter().map(|group| group.row as u32).collect();
        self.columns
            .iter()
            .map(|column| column.take(&rows))
            .collect()
    }
}

/// The aggregates of the query, bound to their input column.
struct Aggregates<'a> {
    /// The input column; `count(*)` has none.
    columns: Vec<Option<&'a Column>>,
    functions: Vec<AggregateFn>,
    aliases: Vec<FieldName>,
}

impl<'a> Aggregates<'a> {
    fn resolve(table: &'a Table, query: &ValidatedQuery) -> Result<Self, ExecuteError> {
        let mut columns = Vec::with_capacity(query.aggregate.len());
        let mut functions = Vec::with_capacity(query.aggregate.len());
        let mut aliases = Vec::with_capacity(query.aggregate.len());
        for aggregate in &query.aggregate {
            columns.push(match &aggregate.field {
                Some(field) => Some(super::column(table, field.as_str())?),
                None => None,
            });
            functions.push(aggregate.function);
            aliases.push(aggregate.alias.clone());
        }
        Ok(Aggregates {
            columns,
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
    ) -> Result<Column, ExecuteError> {
        let function = self.functions[position];
        if let AggregateFn::Min | AggregateFn::Max = function {
            let source = self.columns[position].expect("min and max are validated to have a field");
            let taken: Vec<Option<u32>> = state
                .iter()
                .map(|group| match &group.accumulated[position] {
                    Accumulator::Extreme { key, row, .. } => key.as_ref().map(|_| *row as u32),
                    _ => unreachable!("min and max accumulate extremes"),
                })
                .collect();
            // An empty input with no group: the extreme is NULL, and there is
            // no row to take it from — `take_optional` answers NULL then.
            return Ok(source.take_optional(&taken));
        }

        let mut builder = ColumnBuilder::new(output_type, state.len());
        for group in state {
            let value = match &group.accumulated[position] {
                Accumulator::Count(count) => Value::Int64(*count),
                Accumulator::SumInt64 { sum, seen } => option(*seen, || Value::Int64(*sum)),
                Accumulator::SumFloat64 { sum, seen } => option(*seen, || Value::Float64(*sum)),
                Accumulator::SumDecimal { sum, seen } => option(*seen, || {
                    let DataType::Decimal { scale, .. } = output_type else {
                        unreachable!("a decimal sum is a decimal column")
                    };
                    Value::Decimal(Decimal::new(*sum, scale))
                }),
                // Rule S11: over an empty or all-NULL set the average is NULL.
                Accumulator::Avg { sum, count } => {
                    option(*count > 0, || Value::Float64(*sum / *count as f64))
                }
                Accumulator::Extreme { .. } => unreachable!("handled above"),
            };
            // Validation only admits the types of the result table (S12), so a
            // mismatch here is a defect — reported, never panicked.
            builder
                .push(&value)
                .map_err(|message| ExecuteError::Value {
                    field: alias.to_string(),
                    message: format!("{} {message}", function.as_str()),
                })?;
        }
        Ok(builder.finish())
    }
}

fn option(seen: bool, value: impl FnOnce() -> Value) -> Value {
    if seen { value() } else { Value::Null }
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
    fn of(function: AggregateFn, input: Option<DataType>) -> Self {
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
        aggregates: &Aggregates<'_>,
        position: usize,
        row: usize,
    ) -> Result<(), ExecuteError> {
        let function = aggregates.functions[position];
        let alias = &aggregates.aliases[position];
        let Some(column) = aggregates.columns[position] else {
            // Only `count(*)` runs without a field, and it counts rows.
            if let Accumulator::Count(count) = self {
                *count += 1;
            }
            return Ok(());
        };
        if column.is_null(row) {
            return Ok(());
        }

        match (self, function, column.values()) {
            (Accumulator::Count(count), AggregateFn::Count, _) => *count += 1,
            (Accumulator::SumInt64 { sum, seen }, AggregateFn::Sum, Values::Int64(values)) => {
                *sum = sum
                    .checked_add(values[row])
                    .ok_or_else(|| overflow(alias))?;
                *seen = true;
            }
            (Accumulator::SumDecimal { sum, seen }, AggregateFn::Sum, Values::Decimal(values)) => {
                *sum = sum
                    .checked_add(values[row])
                    .ok_or_else(|| overflow(alias))?;
                *seen = true;
            }
            (Accumulator::SumFloat64 { sum, seen }, AggregateFn::Sum, Values::Float64(values)) => {
                *sum += values[row];
                *seen = true;
            }
            (Accumulator::Avg { sum, count }, AggregateFn::Avg, _) => {
                *sum += as_number(column, row, alias)?;
                *count += 1;
            }
            (
                Accumulator::Extreme {
                    key,
                    row: best,
                    max,
                },
                AggregateFn::Min | AggregateFn::Max,
                _,
            ) => {
                let mut candidate = Vec::new();
                encode(column, row, &mut candidate);
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
            (_, function, _) => {
                return Err(ExecuteError::Value {
                    field: alias.to_string(),
                    message: format!(
                        "cannot {} a {} column",
                        function.as_str(),
                        column.data_type()
                    ),
                });
            }
        }
        Ok(())
    }
}

/// The result table: the columns of `output_schema`, in that order.
fn build(
    query: &ValidatedQuery,
    groups: &GroupColumns<'_>,
    aggregates: &Aggregates<'_>,
    state: &[Group],
) -> Result<Table, ExecuteError> {
    let mut key_columns = groups.take(state).into_iter().map(Some).collect::<Vec<_>>();
    let mut columns = Vec::with_capacity(query.output_schema.len());

    for field in query.output_schema.fields() {
        if let Some(position) = query
            .group
            .iter()
            .position(|key| key.as_str() == field.name.as_str())
        {
            columns.push(
                key_columns[position]
                    .take()
                    .expect("every group key appears once in the output"),
            );
        } else {
            let position = aggregates.position_of(&field.name)?;
            columns.push(aggregates.column_of(position, field.data_type, &field.name, state)?);
        }
    }

    Table::new(&query.output_schema, columns).map_err(|message| ExecuteError::Value {
        field: String::new(),
        message,
    })
}

/// Writes the order-preserving key of one cell into `key`.
fn encode(column: &Column, row: usize, key: &mut Vec<u8>) {
    // A NULL is its own group (rule S10) and has no place in an order.
    if column.is_null(row) {
        key.extend_from_slice(&[0x00, 0x00]);
        return;
    }
    key.push(0x01);
    match column.values() {
        Values::Bool(values) => key.push(u8::from(values[row])),
        Values::Int64(values) | Values::Timestamp(values) => {
            key.extend_from_slice(&order_of_i64(values[row]));
        }
        Values::Float64(values) => key.extend_from_slice(&order_of_f64(values[row])),
        Values::Decimal(values) => key.extend_from_slice(&order_of_i128(values[row])),
        Values::Utf8 { .. } => {
            // Escaped, so the encoding stays prefix-free *and* keeps the byte
            // order (rules S4/S13: binary, no Unicode normalisation).
            for byte in column.str(row).as_bytes() {
                match byte {
                    0x00 => key.extend_from_slice(&[0x00, 0xFF]),
                    other => key.push(*other),
                }
            }
            key.extend_from_slice(&[0x00, 0x00]);
        }
        Values::Date(values) => key.extend_from_slice(&order_of_i32(values[row])),
    }
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

fn overflow(alias: &FieldName) -> ExecuteError {
    ExecuteError::Value {
        field: alias.to_string(),
        message: "the sum does not fit into the result type any more".to_owned(),
    }
}

/// `avg` over Int64, Decimal or Float64 is a float (rule S12).
fn as_number(column: &Column, row: usize, field: &FieldName) -> Result<f64, ExecuteError> {
    Ok(match (column.values(), column.data_type()) {
        (Values::Int64(values), _) => values[row] as f64,
        (Values::Float64(values), _) => values[row],
        (Values::Decimal(values), DataType::Decimal { scale, .. }) => {
            values[row] as f64 / 10f64.powi(scale as i32)
        }
        (_, other) => {
            return Err(ExecuteError::Value {
                field: field.to_string(),
                message: format!("cannot average {other}"),
            });
        }
    })
}
