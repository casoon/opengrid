//! The local query executor: filter, grouping and aggregation, sort, paging and
//! projection.
//!
//! Plan points 07 and 08, the pipeline of plan/spezifikation/04-local-engine.md:
//! filter → group/aggregate → sort → offset/limit → projection. Grouping and
//! aggregation live in the `aggregate` module.
//!
//! Two field namespaces meet here, and the contract is explicit about which is
//! which: `filter` names *input* columns, `sort` names *output* columns
//! (plan/spezifikation/02-query-modell.md). For a query without grouping the
//! output columns are the selected input columns, so both resolve in the same
//! batch. The projection therefore runs *before* the sort (point 44): it only
//! picks column references, and the sort then copies the page of the columns
//! the query returns rather than every row of every input column. The result is
//! the one the documented order describes.
//!
//! Rules S1 (three-valued filter logic), S2 (`in`), S3/S4/S6/S7 (sorting and
//! paging), S13/S14 (string comparison, empty string) are implemented in
//! [`filter`] and [`order`], each with the rule in its doc comment.

mod aggregate;
mod filter;
mod order;

use opengrid_columns::Table;
use opengrid_query::ValidatedQuery;
use opengrid_types::{DataType, Schema};

/// What the executor returns.
///
/// The shape is what the grid needs (`total_count` drives `aria-rowcount` and
/// "showing 1–50 of 312") and what the wire protocol of point 23 carries, so
/// the server of point 24 can answer with the same type.
#[derive(Clone, Debug)]
pub struct QueryResult {
    /// The result rows, in the columns of the query's output schema.
    pub table: Table,
    /// Rows that matched the filter, **before** `offset`/`limit`.
    pub total_count: u64,
}

/// What the executor refuses to do.
#[derive(Debug)]
pub enum ExecuteError {
    /// The query names a column the data does not carry.
    MissingField { field: String },
    /// A column carries a type the query was not validated against.
    TypeMismatch {
        field: String,
        expected: DataType,
        found: DataType,
    },
    /// A cell or literal does not fit its column.
    Value { field: String, message: String },
    /// The data is too large for a kernel.
    TooLarge { message: String },
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::MissingField { field } => {
                write!(f, "no column named {field:?}")
            }
            ExecuteError::TypeMismatch {
                field,
                expected,
                found,
            } => write!(
                f,
                "column {field:?} is {found}, but the query expects {expected}"
            ),
            ExecuteError::Value { field, message } => {
                write!(f, "column {field:?}: {message}")
            }
            ExecuteError::TooLarge { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for ExecuteError {}

/// Runs a validated query against a table.
///
/// The output is a single table; chunking the result is the caller's business
/// (the grid asks for it through `limit`/`offset`, the export path through
/// [`crate::datasource::LocalPieces`]).
pub fn execute(table: &Table, query: &ValidatedQuery) -> Result<QueryResult, ExecuteError> {
    let filtered;
    let mut table = table;
    if let Some(filter) = &query.filter {
        let mask = filter::evaluate(filter, table)?;
        let keep: Vec<u32> = mask
            .iter()
            .enumerate()
            .filter(|(_, hit)| **hit == Some(true))
            .map(|(row, _)| row as u32)
            .collect();
        filtered = table.take(&keep);
        table = &filtered;
    }

    // Rules S10–S12: grouping and aggregation, when the query asks for them.
    // They run *before* the sort, so `sort` can name an aggregate alias.
    let grouped;
    if !query.group.is_empty() || !query.aggregate.is_empty() {
        grouped = aggregate::run(table, query)?;
        table = &grouped;
    }

    // Before paging: this is the number the grid shows next to the page.
    let total_count = table.num_rows() as u64;

    // Projection first: it only picks columns, and whatever comes after it then
    // copies the columns the query returns, not all of them. The sort may do
    // that, because `sort` names *output* columns.
    let projected = project(table, &query.output_schema)?;
    let offset = usize::try_from(query.offset.unwrap_or(0)).unwrap_or(usize::MAX);
    let limit = query
        .limit
        .map(|limit| usize::try_from(limit).unwrap_or(usize::MAX));
    let table = if query.sort.is_empty() {
        // An `offset` beyond the last row is an empty page, not an error: the
        // grid asks for page N after the data shrank, and it should see
        // nothing rather than a failure. Rule S6 lets validation reject an
        // `offset` without `sort`, so this is a `limit` alone in practice.
        projected.slice(offset, limit.unwrap_or(usize::MAX))
    } else {
        order::sort_page(&projected, &query.sort, offset, limit)?
    };

    Ok(QueryResult { table, total_count })
}

/// Keeps the output columns, in the order the query declares them.
///
/// The order comes from the validated query's `output_schema`, not from
/// `select`: with aggregation (point 08) `select` no longer lists every output
/// column, and the schema is the authority both sides already agreed on.
fn project(table: &Table, output: &Schema) -> Result<Table, ExecuteError> {
    for field in output.fields() {
        let found = column(table, field.name.as_str())?.data_type();
        if found != field.data_type {
            return Err(ExecuteError::TypeMismatch {
                field: field.name.to_string(),
                expected: field.data_type,
                found,
            });
        }
    }
    table.select(output).map_err(|message| ExecuteError::Value {
        field: String::new(),
        message,
    })
}

/// The column of an input field, or the error naming it.
pub(crate) fn column<'a>(
    table: &'a Table,
    field: &str,
) -> Result<&'a opengrid_columns::Column, ExecuteError> {
    table
        .column(field)
        .ok_or_else(|| ExecuteError::MissingField {
            field: field.to_owned(),
        })
}
