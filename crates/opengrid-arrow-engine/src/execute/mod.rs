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

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{ArrowError, DataType as ArrowDataType};
use arrow_select::concat::concat_batches;
use arrow_select::filter::filter_record_batch;
use opengrid_query::ValidatedQuery;
use opengrid_types::{DataType, Schema};

/// What the executor returns.
///
/// The shape is what the grid needs (`total_count` drives `aria-rowcount` and
/// "showing 1–50 of 312") and what the wire protocol of point 23 will carry, so
/// the server of point 24 can answer with the same type.
#[derive(Clone, Debug)]
pub struct QueryResult {
    /// The columns of every batch, in order.
    pub schema: Schema,
    /// The result rows, cut into batches.
    pub batches: Vec<RecordBatch>,
    /// Rows that matched the filter, **before** `offset`/`limit`.
    pub total_count: u64,
}

/// What the executor refuses to do.
#[derive(Debug)]
pub enum ExecuteError {
    /// There is no data to run the query against.
    NoBatches,
    /// The query names a column the data does not carry.
    MissingField { field: String },
    /// The data carries a column type the query model does not know.
    UnknownType { field: String, found: ArrowDataType },
    /// A column carries a type the query was not validated against.
    TypeMismatch {
        field: String,
        expected: DataType,
        found: ArrowDataType,
    },
    /// A cell or literal does not fit its column.
    Value { field: String, message: String },
    /// Arrow refused a kernel.
    Arrow(ArrowError),
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::NoBatches => f.write_str("no batch to execute against"),
            ExecuteError::MissingField { field } => {
                write!(f, "no column named {field:?}")
            }
            ExecuteError::UnknownType { field, found } => {
                write!(f, "column {field:?} has the unknown type {found}")
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
            ExecuteError::Arrow(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ExecuteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExecuteError::Arrow(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ArrowError> for ExecuteError {
    fn from(error: ArrowError) -> Self {
        ExecuteError::Arrow(error)
    }
}

/// Runs a validated query against batches.
///
/// The batches are concatenated first, so a query sees the data set as one
/// table — paging and sorting are defined over the whole result, not per batch.
/// The output is a single batch; chunking the result is the caller's business
/// (the grid asks for it through `limit`/`offset`, the export path through the
/// wire protocol).
pub fn execute(
    batches: &[RecordBatch],
    query: &ValidatedQuery,
) -> Result<QueryResult, ExecuteError> {
    let Some(first) = batches.first() else {
        return Err(ExecuteError::NoBatches);
    };

    let mut batch = concat_batches(&first.schema(), batches)?;

    if let Some(filter) = &query.filter {
        let mask = filter::evaluate(filter, &batch)?;
        batch = filter_record_batch(&batch, &mask)?;
    }

    // Rules S10–S12: grouping and aggregation, when the query asks for them.
    // They run *before* the sort, so `sort` can name an aggregate alias.
    if !query.group.is_empty() || !query.aggregate.is_empty() {
        batch = aggregate::run(&batch, query)?;
    }

    // Before paging: this is the number the grid shows next to the page.
    let total_count = batch.num_rows() as u64;

    // Projection first: it only picks column references, and whatever comes
    // after it then copies the columns the query returns, not all of them. The
    // sort may do that, because `sort` names *output* columns.
    let batch = project(&batch, &query.output_schema)?;
    let batch = if query.sort.is_empty() {
        page(&batch, query.offset, query.limit)
    } else {
        order::sort_page(
            &batch,
            &query.sort,
            usize::try_from(query.offset.unwrap_or(0)).unwrap_or(usize::MAX),
            query
                .limit
                .map(|limit| usize::try_from(limit).unwrap_or(usize::MAX)),
        )?
    };

    Ok(QueryResult {
        schema: query.output_schema.clone(),
        batches: vec![batch],
        total_count,
    })
}

/// Cuts the requested window out of an unsorted result — a `limit` alone, since
/// rule S6 lets validation reject an `offset` without `sort`. A sorted result is
/// cut by [`order::sort_page`], which copies only the page.
///
/// An `offset` beyond the last row is an empty page, not an error: the grid asks
/// for page N after the data shrank, and it should see nothing rather than a
/// failure.
fn page(batch: &RecordBatch, offset: Option<u64>, limit: Option<u64>) -> RecordBatch {
    let len = batch.num_rows();
    let offset = usize::try_from(offset.unwrap_or(0))
        .unwrap_or(usize::MAX)
        .min(len);
    let length = match limit {
        Some(limit) => usize::try_from(limit)
            .unwrap_or(usize::MAX)
            .min(len - offset),
        None => len - offset,
    };
    batch.slice(offset, length)
}

/// The query-model type of one input column.
///
/// A column whose Arrow type has no counterpart in the query model cannot be
/// filtered, grouped or aggregated — that is an error, not a guess.
pub(crate) fn data_type_of(batch: &RecordBatch, index: usize) -> Result<DataType, ExecuteError> {
    let field = batch.schema().field(index).clone();
    DataType::from_arrow(field.data_type()).ok_or_else(|| ExecuteError::UnknownType {
        field: field.name().clone(),
        found: field.data_type().clone(),
    })
}

/// Keeps the output columns, in the order the query declares them.
///
/// The order comes from the validated query's `output_schema`, not from
/// `select`: with aggregation (point 08) `select` no longer lists every output
/// column, and the schema is the authority both sides already agreed on.
fn project(batch: &RecordBatch, output: &Schema) -> Result<RecordBatch, ExecuteError> {
    let mut arrays = Vec::with_capacity(output.fields().len());
    for field in output.fields() {
        let index = batch.schema().index_of(field.name.as_str()).map_err(|_| {
            ExecuteError::MissingField {
                field: field.name.to_string(),
            }
        })?;
        let column = batch.column(index);
        let expected = ArrowDataType::from(field.data_type);
        if column.data_type() != &expected {
            return Err(ExecuteError::TypeMismatch {
                field: field.name.to_string(),
                expected: field.data_type,
                found: column.data_type().clone(),
            });
        }
        arrays.push(column.clone());
    }

    let schema = Arc::new(arrow_schema::Schema::from(output));
    Ok(RecordBatch::try_new(schema, arrays)?)
}
