//! Sorting: the keys of the query model into a row order, and the page cut
//! out of it.
//!
//! Rules: `nulls` is always explicit and independent of the direction (S3),
//! collation is `binary` in V1 (S4) — UTF-8 compared byte-wise, which is
//! codepoint order, so `"Z" < "z" < "ä"`. Floats follow the IEEE 754 total
//! order (S7): `NaN` after every number, `-0.0` before `0.0`.
//!
//! **Ties keep their input order.** S6 leaves the order of ties undefined, so
//! any order is a legal answer — but the grid pages through a sort by
//! `customer`, which is nothing but ties, and every window of it has to agree
//! with every other. Arrow's sorts are unstable, so the input position is the
//! last key: the order is total, the same for every `limit`, and a sort may
//! stop as soon as it has the rows a page needs (point 44).
//!
//! Two paths to the same order, chosen by what was measured to be fast
//! (plan/spezifikation/12-qualitaet.md §Sortieren):
//!
//! - **One key:** Arrow's single-column sort, which is typed and quick — above
//!   all on data that is already in order, such as the grid's default sort by
//!   `id`. Its ties are then put back into input order, run by run.
//! - **Several keys:** the keys as `arrow-row` rows — every key column encoded
//!   once into bytes that compare with `memcmp` exactly as the values would, so
//!   comparing several keys is one byte comparison instead of a walk through
//!   typed columns, and the input position as the final key is nearly free.

use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_ord::partition::partition;
use arrow_ord::sort::{SortOptions, sort_to_indices};
use arrow_row::{Row, RowConverter, SortField};
use arrow_schema::ArrowError;
use arrow_select::take::{take, take_record_batch};
use opengrid_query::{NullsOrder, Sort, SortDirection};

use super::ExecuteError;

/// Sorts a batch by the output columns the query names and returns only the
/// rows of the page — `offset`, then at most `limit` rows.
///
/// Only the page is copied: the sort works on positions, and `take` runs over
/// the rows that are returned, not over every row of the input.
pub(crate) fn sort_page(
    batch: &RecordBatch,
    keys: &[Sort],
    offset: usize,
    limit: Option<usize>,
) -> Result<RecordBatch, ExecuteError> {
    let mut columns = Vec::with_capacity(keys.len());
    for key in keys {
        let index = batch.schema().index_of(key.field.as_str()).map_err(|_| {
            ExecuteError::MissingField {
                field: key.field.to_string(),
            }
        })?;
        let options = SortOptions {
            descending: matches!(key.direction, SortDirection::Desc),
            // Rule S3: not flipped for `desc`, unlike PostgreSQL's default.
            nulls_first: matches!(key.nulls, NullsOrder::First),
        };
        columns.push((batch.column(index).clone(), options));
    }

    let rows = batch.num_rows();
    let offset = offset.min(rows);
    let end = limit.map_or(rows, |limit| offset.saturating_add(limit).min(rows));
    let order = match columns.as_slice() {
        [(column, options)] => one_key(column, *options)?,
        _ => several_keys(&columns, end)?,
    };
    let page = UInt32Array::from(order[offset..end].to_vec());
    Ok(take_record_batch(batch, &page)?)
}

/// Input positions in sort order, for a single key.
///
/// Arrow sorts the column; then every run of equal values — equal as `distinct`
/// sees it, which is the sort's own notion: floats by total order, NULLs as one
/// value — is put back into input order. A column without ties costs one linear
/// pass for that.
fn one_key(column: &ArrayRef, options: SortOptions) -> Result<Vec<u32>, ArrowError> {
    let indices = sort_to_indices(column, Some(options), None)?;
    let sorted = take(column, &indices, None)?;
    let mut order = indices.values().to_vec();
    for run in partition(&[sorted])?.ranges() {
        if run.len() > 1 {
            order[run].sort_unstable();
        }
    }
    Ok(order)
}

/// Input positions in sort order, for several keys — at least the first `end`
/// of them in their final order.
///
/// The keys are encoded as rows and sorted as (row, position) pairs; moving the
/// pairs was measured faster than moving positions alone and looking the row up
/// on every comparison. With a `limit`, the pairs before `end` are selected
/// first (linear) and only those are sorted: the order is total, so that front
/// is exactly the front of the full sort, whatever `limit` asked.
fn several_keys(columns: &[(ArrayRef, SortOptions)], end: usize) -> Result<Vec<u32>, ArrowError> {
    let fields = columns
        .iter()
        .map(|(column, options)| SortField::new_with_options(column.data_type().clone(), *options))
        .collect();
    let arrays: Vec<ArrayRef> = columns.iter().map(|(column, _)| column.clone()).collect();
    let rows = RowConverter::new(fields)?.convert_columns(&arrays)?;

    // Four-byte positions, like Arrow's own sort indices: a wasm32 address
    // space holds far fewer rows, and a native batch past 2^32 rows is refused
    // rather than wrapped.
    let total = rows.num_rows();
    u32::try_from(total)
        .map_err(|_| ArrowError::ComputeError(format!("{total} rows are too many to sort")))?;
    let mut order: Vec<(Row<'_>, u32)> = rows
        .iter()
        .enumerate()
        .map(|(at, row)| (row, at as u32))
        .collect();
    if end < total && end > 0 {
        order.select_nth_unstable(end - 1);
        order.truncate(end);
    } else if end == 0 {
        order.clear();
    }
    order.sort_unstable();
    Ok(order.into_iter().map(|(_, at)| at).collect())
}
