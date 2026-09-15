//! The local query executor: filter, sort, paging and projection.
//!
//! Plan point 07, the pipeline of plan/spezifikation/04-local-engine.md:
//! filter → sort → offset/limit → projection. Grouping and aggregation are
//! point 08 — a query that needs them is **refused** ([`ExecuteError::Unsupported`])
//! instead of being executed half-way, so no caller can mistake a partial
//! result for a complete one.
//!
//! Two field namespaces meet here, and the contract is explicit about which is
//! which: `filter` names *input* columns, `sort` names *output* columns
//! (plan/spezifikation/02-query-modell.md). For a query without grouping the
//! output columns are the selected input columns, so both resolve in the same
//! batch — which is why the projection is the last step, exactly as the
//! documented pipeline order says.
//!
//! Rules S1 (three-valued filter logic), S2 (`in`), S3/S4/S6/S7 (sorting and
//! paging), S13/S14 (string comparison, empty string) are implemented in
//! [`filter`] and [`order`], each with the rule in its doc comment.

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
    /// The query needs something this executor does not have yet. Point 08 adds
    /// `group`/`aggregate`, later points add the rest.
    Unsupported { feature: String },
    /// There is no data to run the query against.
    NoBatches,
    /// The query names a column the data does not carry.
    MissingField { field: String },
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
            ExecuteError::Unsupported { feature } => {
                write!(f, "not supported yet: {feature}")
            }
            ExecuteError::NoBatches => f.write_str("no batch to execute against"),
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
    if !query.group.is_empty() || !query.aggregate.is_empty() {
        return Err(ExecuteError::Unsupported {
            feature: "group and aggregate (plan point 08)".to_owned(),
        });
    }
    let Some(first) = batches.first() else {
        return Err(ExecuteError::NoBatches);
    };

    let mut batch = concat_batches(&first.schema(), batches)?;

    if let Some(filter) = &query.filter {
        let mask = filter::evaluate(filter, &batch)?;
        batch = filter_record_batch(&batch, &mask)?;
    }

    // Before paging: this is the number the grid shows next to the page.
    let total_count = batch.num_rows() as u64;

    if !query.sort.is_empty() {
        batch = order::sort(&batch, &query.sort)?;
    }

    let batch = page(&batch, query.offset, query.limit);

    Ok(QueryResult {
        schema: query.output_schema.clone(),
        batches: vec![project(&batch, &query.output_schema)?],
        total_count,
    })
}

/// Cuts the requested window out of the sorted result (rule S6: `offset`
/// without `sort` never reaches this function — validation rejects it).
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
