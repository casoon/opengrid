//! Sorting: the keys of the query model into a row order, and the page cut
//! out of it.
//!
//! The order itself — NULL placement independent of the direction (S3),
//! `binary` collation (S4), IEEE total order for floats (S7), ties in input
//! order (S6) — is [`opengrid_columns::sort::order`]'s, written down there. This
//! module binds the query's keys to the output columns and copies the page.
//! A key with a locale (S4a) gets its column's collation keys from
//! [`crate::collation`] first; the order then compares those.

use opengrid_columns::Table;
use opengrid_columns::sort::{SortKey, SortKeys, order};
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
    let collated = collated(table, keys)?;
    let sort_keys = sort_keys(table, keys, &collated)?;

    let rows = table.num_rows();
    let offset = offset.min(rows);
    let end = limit.map_or(rows, |limit| offset.saturating_add(limit).min(rows));
    let positions = order(&sort_keys, end).map_err(|message| ExecuteError::TooLarge { message })?;
    Ok(table.take(&positions[offset..]))
}

/// The collation keys of every key that names a locale, `None` for the others.
pub(crate) fn collated(table: &Table, keys: &[Sort]) -> Result<Vec<Option<SortKeys>>, ExecuteError> {
    keys.iter()
        .map(|key| match key.collation.locale() {
            None => Ok(None),
            Some(locale) => {
                let column = super::column(table, key.field.as_str())?;
                crate::collation::sort_keys(locale.as_str(), column)
                    .map(Some)
                    .map_err(|message| ExecuteError::Collation { message })
            }
        })
        .collect()
}

/// The query's keys bound to the table's columns, with what [`collated`] made.
pub(crate) fn sort_keys<'a>(
    table: &'a Table,
    keys: &[Sort],
    collated: &'a [Option<SortKeys>],
) -> Result<Vec<SortKey<'a>>, ExecuteError> {
    keys.iter()
        .zip(collated)
        .map(|(key, collated)| {
            Ok(SortKey {
                column: super::column(table, key.field.as_str())?,
                descending: matches!(key.direction, SortDirection::Desc),
                // Rule S3: not flipped for `desc`, unlike PostgreSQL's default.
                nulls_first: matches!(key.nulls, NullsOrder::First),
                collated: collated.as_ref(),
            })
        })
        .collect()
}
