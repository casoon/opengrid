//! Sorting: the keys of the query model into a row order.
//!
//! Rules: `nulls` is always explicit and independent of the direction (S3),
//! collation is `binary` in V1 (S4) — Arrow compares UTF-8 byte-wise, which is
//! codepoint order, so `"Z" < "z" < "ä"` — and ties keep their input order (S6
//! allows any order, so a stable sort is a legal answer).

use arrow_array::RecordBatch;
use arrow_ord::sort::{SortColumn, SortOptions, lexsort_to_indices};
use arrow_select::take::take_record_batch;
use opengrid_query::{NullsOrder, Sort, SortDirection};

use super::ExecuteError;

/// Sorts a batch by the output columns the query names.
///
/// Float columns sort by Arrow's *total order*, which puts `NaN` after every
/// number — the same order rule S7 gives PostgreSQL. `-0.0` and `0.0` land next
/// to each other; which of the two comes first is a tie (S6).
pub(crate) fn sort(batch: &RecordBatch, keys: &[Sort]) -> Result<RecordBatch, ExecuteError> {
    let mut columns = Vec::with_capacity(keys.len());
    for key in keys {
        let index = batch.schema().index_of(key.field.as_str()).map_err(|_| {
            ExecuteError::MissingField {
                field: key.field.to_string(),
            }
        })?;
        columns.push(SortColumn {
            values: batch.column(index).clone(),
            options: Some(SortOptions {
                descending: matches!(key.direction, SortDirection::Desc),
                // Rule S3: not flipped for `desc`, unlike PostgreSQL's default.
                nulls_first: matches!(key.nulls, NullsOrder::First),
            }),
        });
    }

    let indices = lexsort_to_indices(&columns, None)?;
    Ok(take_record_batch(batch, &indices)?)
}
