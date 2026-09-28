//! Sorting: the keys of the query model into a row order, and the page cut
//! out of it.
//!
//! The order itself — NULL placement independent of the direction (S3),
//! `binary` collation (S4), IEEE total order for floats (S7), ties in input
//! order (S6) — is [`opengrid_columns::sort::order`]'s, written down there. This
//! module binds the query's keys to the output columns and copies the page.

use opengrid_columns::Table;
use opengrid_columns::sort::{SortKey, order};
use opengrid_query::{NullsOrder, Sort, SortDirection};

use super::ExecuteError;

/// Sorts a table by the output columns the query names and returns only the
/// rows of the page — `offset`, then at most `limit` rows.
///
/// Only the page is copied: the sort works on positions, and the gather runs
/// over the rows that are returned, not over every row of the input.
pub(crate) fn sort_page(
    table: &Table,
    keys: &[Sort],
    offset: usize,
    limit: Option<usize>,
) -> Result<Table, ExecuteError> {
    let mut sort_keys = Vec::with_capacity(keys.len());
    for key in keys {
        sort_keys.push(SortKey {
            column: super::column(table, key.field.as_str())?,
            descending: matches!(key.direction, SortDirection::Desc),
            // Rule S3: not flipped for `desc`, unlike PostgreSQL's default.
            nulls_first: matches!(key.nulls, NullsOrder::First),
        });
    }

    let rows = table.num_rows();
    let offset = offset.min(rows);
    let end = limit.map_or(rows, |limit| offset.saturating_add(limit).min(rows));
    let positions = order(&sort_keys, end).map_err(|message| ExecuteError::TooLarge { message })?;
    Ok(table.take(&positions[offset..]))
}
