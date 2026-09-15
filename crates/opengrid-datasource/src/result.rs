use opengrid_types::{Schema, Value};

/// The result of a query, in the shape the wire format has.
///
/// Decision E14: Arrow-free and column-oriented, like the wire format of E6 — one
/// `Vec<Value>` per output column, in schema order, and `total_count` alongside.
/// The grid renders a page from it and asks for the next one; the server of
/// point 24 answers with the same type.
///
/// Invariant: `columns.len()` equals `schema.fields().len()` and every column has
/// the same length. The producers of this crate's users build it from a batch
/// whose schema they already agreed on.
#[derive(Clone, Debug, PartialEq)]
pub struct QueryResult {
    /// The output columns the query declared, in order.
    pub schema: Schema,
    /// One column of values, in schema order.
    pub columns: Vec<Vec<Value>>,
    /// Rows that matched the filter, **before** `offset`/`limit` — the number the
    /// grid shows next to the page (`aria-rowcount`, "showing 1–50 of 312").
    pub total_count: u64,
}

impl QueryResult {
    /// Builds a result from typed columns.
    pub fn new(schema: Schema, columns: Vec<Vec<Value>>, total_count: u64) -> Self {
        Self {
            schema,
            columns,
            total_count,
        }
    }

    /// The number of rows on this page (not [`total_count`](Self::total_count),
    /// which counts the whole result).
    pub fn row_count(&self) -> usize {
        self.columns.first().map_or(0, Vec::len)
    }
}
