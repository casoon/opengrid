//! Running the grouping sets and stitching the matrix together (plan point 52).

use std::collections::HashMap;

use opengrid_datasource::{DataSource, DataSourceError, QueryResult};
use opengrid_query::{Aggregate, AggregateFn};
use opengrid_types::{Field, FieldName, Schema, Value};

use crate::query::{PivotError, ValidatedPivotQuery};
use crate::result::{PivotColumn, PivotResult};
use crate::{Key, same_key};

/// A pivot that failed somewhere between the source and the matrix.
#[derive(Debug)]
pub enum ExecuteError {
    /// The source could not answer one of the grouping sets.
    Source(DataSourceError),
    /// The pivot itself does not work — a limit, usually.
    Pivot(PivotError),
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::Source(error) => write!(f, "{error}"),
            ExecuteError::Pivot(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ExecuteError {}

impl From<DataSourceError> for ExecuteError {
    fn from(error: DataSourceError) -> Self {
        ExecuteError::Source(error)
    }
}

impl From<PivotError> for ExecuteError {
    fn from(error: PivotError) -> Self {
        ExecuteError::Pivot(error)
    }
}

/// The pivot over a source: ask each level, then arrange the answers.
pub struct PivotEngine;

/// Runs `pivot` against `source`.
///
/// One query per level (`n+1` for `n` row dimensions), each already sorted by
/// its own keys. Nothing here compares two values to decide an order.
pub async fn execute<S: DataSource>(
    source: &S,
    pivot: &ValidatedPivotQuery,
) -> Result<PivotResult, ExecuteError> {
    let mut levels = Vec::with_capacity(pivot.sets.len());
    for set in &pivot.sets {
        levels.push(source.execute(set.clone()).await?);
    }
    assemble(pivot, &levels).map_err(ExecuteError::Pivot)
}

impl PivotEngine {
    /// The same thing as [`execute`], as a method, for callers that prefer one.
    pub async fn run<S: DataSource>(
        source: &S,
        pivot: &ValidatedPivotQuery,
    ) -> Result<PivotResult, ExecuteError> {
        execute(source, pivot).await
    }
}

/// One level's answer, indexed the way the assembly reads it.
struct Level<'a> {
    result: &'a QueryResult,
    /// `(row key, column key)` → the row's index in `result`.
    cells: HashMap<(Key, Key), usize>,
}

impl<'a> Level<'a> {
    fn of(depth: usize, columns: usize, result: &'a QueryResult) -> Self {
        let mut cells = HashMap::with_capacity(result.row_count());
        for row in 0..result.row_count() {
            let keys: Vec<Value> = (0..depth + columns)
                .map(|index| result.columns[index][row].clone())
                .collect();
            cells.insert((same_key(&keys[..depth]), same_key(&keys[depth..])), row);
        }
        Level { result, cells }
    }

    /// The measures of one cell, or NULL/0 where the group does not exist.
    ///
    /// Rule P4 applied through S11: a cell nobody's rows fall into is an empty
    /// set, and `count` of an empty set is 0 while the others are NULL.
    fn measures(&self, row: &Key, column: &Key, values: &[Aggregate]) -> Vec<Value> {
        // The measures are the last columns of every grouping set: the select is
        // group keys first, aliases after.
        let first = self.result.schema.len() - values.len();
        match self.cells.get(&(row.clone(), column.clone())) {
            Some(index) => (0..values.len())
                .map(|offset| self.result.columns[first + offset][*index].clone())
                .collect(),
            None => values.iter().map(empty_cell).collect(),
        }
    }
}

/// What a cell holds when no row falls into it (P4 through S11).
///
/// `count` of an empty set is 0; `sum`, `avg`, `min` and `max` of an empty set
/// are NULL. The distinction is not decoration — a pivot that wrote NULL into
/// every empty count cell would show gaps where the answer is "none".
fn empty_cell(value: &Aggregate) -> Value {
    match value.function {
        AggregateFn::Count => Value::Int64(0),
        _ => Value::Null,
    }
}

/// Turns the levels into the matrix.
///
/// Public because there is more than one way to *get* the levels and only one
/// way to arrange them: the generic path runs `n+1` queries, and the PostgreSQL
/// pushdown (plan point 31) runs one `GROUPING SETS` statement and splits the
/// answer back into the same levels. Two assemblers would mean the differential
/// test compared two reshapers instead of the database.
pub fn assemble(
    pivot: &ValidatedPivotQuery,
    results: &[QueryResult],
) -> Result<PivotResult, PivotError> {
    let depth = pivot.rows.len();
    let across = pivot.columns.len();
    let measures = pivot.values.len();

    // `sets` is deepest first, so `results[0]` is the detail level and the last
    // one is the grand total — which doubles as the catalogue of columns.
    let detail = &results[0];
    let catalogue = results.last().expect("a pivot has at least one level");

    // The columns, in the order the source sorted them (S3/S4).
    let mut columns = Vec::new();
    for row in 0..catalogue.row_count() {
        let path: Vec<Value> = (0..across)
            .map(|index| catalogue.columns[index][row].clone())
            .collect();
        for value in &pivot.values {
            columns.push(PivotColumn {
                path: path.clone(),
                measure: value.alias.clone(),
            });
        }
    }
    if columns.len() > pivot.limits.max_columns {
        return Err(PivotError::TooManyColumns {
            found: columns.len(),
            maximum: pivot.limits.max_columns,
        });
    }
    let column_keys: Vec<Key> = (0..catalogue.row_count())
        .map(|row| {
            same_key(
                &(0..across)
                    .map(|index| catalogue.columns[index][row].clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    let levels: Vec<Level> = results
        .iter()
        .enumerate()
        .map(|(index, result)| Level::of(depth - index, across, result))
        .collect();

    // Walk the detail level in its own order and emit a subtotal whenever a
    // prefix ends. That is rule P6 — the subtotal follows the rows it sums —
    // and it falls out of the source's ordering instead of being sorted here.
    let mut paths: Vec<(Vec<Value>, u16)> = Vec::new();
    let mut previous: Vec<Value> = Vec::new();
    for row in 0..detail.row_count() {
        let path: Vec<Value> = (0..depth)
            .map(|index| detail.columns[index][row].clone())
            .collect();
        if path == previous {
            continue;
        }
        // Close every prefix the new path does not share, deepest first.
        let shared = path
            .iter()
            .zip(&previous)
            .take_while(|(left, right)| left == right)
            .count();
        if !previous.is_empty() {
            for level in ((shared + 1)..depth).rev() {
                paths.push((previous[..level].to_vec(), level as u16));
            }
        }
        paths.push((path.clone(), depth as u16));
        previous = path;
    }
    if !previous.is_empty() {
        for level in (1..depth).rev() {
            paths.push((previous[..level].to_vec(), level as u16));
        }
    }
    // The grand total is the level that groups by nothing but the columns.
    paths.push((Vec::new(), 0));

    if paths.len() > pivot.limits.max_rows {
        return Err(PivotError::TooManyRows {
            found: paths.len(),
            maximum: pivot.limits.max_rows,
        });
    }

    // The output schema: the row dimensions as they are, then one column per
    // generated leaf. A dimension column is nullable here even when the source
    // column is not — a subtotal has no value at the levels below it.
    let mut fields: Vec<Field> = pivot
        .rows
        .iter()
        .enumerate()
        .map(|(index, name)| Field {
            name: name.clone(),
            data_type: detail.schema.fields()[index].data_type,
            nullable: true,
            from: None,
        })
        .collect();
    for (index, column) in columns.iter().enumerate() {
        let measure = index % measures.max(1);
        let source = &detail.schema.fields()[depth + across + measure];
        fields.push(Field {
            name: FieldName::new(format!("{}_{index}", column.measure))
                .expect("a measure alias plus an index is an identifier"),
            data_type: source.data_type,
            nullable: true,
            from: None,
        });
    }

    let mut cells: Vec<Vec<Value>> = vec![Vec::with_capacity(paths.len()); fields.len()];
    let mut row_levels = Vec::with_capacity(paths.len());
    for (path, level) in &paths {
        row_levels.push(*level);
        for (index, column) in cells.iter_mut().take(depth).enumerate() {
            // Below the row's level the dimensions have no value — `row_levels`
            // is what says so, not this NULL (P2).
            column.push(path.get(index).cloned().unwrap_or(Value::Null));
        }
        let level_of = &levels[depth - usize::from(*level)];
        let row_key = same_key(path);
        for (index, column_key) in column_keys.iter().enumerate() {
            for (offset, value) in level_of
                .measures(&row_key, column_key, &pivot.values)
                .into_iter()
                .enumerate()
            {
                cells[depth + index * measures + offset].push(value);
            }
        }
    }

    let total = paths.len() as u64;
    Ok(PivotResult {
        data: QueryResult::new(Schema::new(fields), cells, total),
        row_levels,
        columns,
    })
}
