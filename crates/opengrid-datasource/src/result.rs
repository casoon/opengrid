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
    /// For one level of a tree (E38): what each row is, beyond its values.
    pub tree: Option<TreeLevel>,
}

/// What a tree query answers beside the rows of its level (E38, T2–T5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeLevel {
    /// Per row of the page: how many visible children it has (T4).
    pub children: Vec<u64>,
    /// Per row of the page: a match, or an ancestor shown as context (T5).
    pub matched: Vec<bool>,
    /// Matches in the whole tree — the count a reader is told (T5).
    pub matches: u64,
    /// Nodes whose parent does not exist, shown as roots (T2).
    pub orphans: u64,
}

impl QueryResult {
    /// Builds a result from typed columns.
    pub fn new(schema: Schema, columns: Vec<Vec<Value>>, total_count: u64) -> Self {
        Self {
            schema,
            columns,
            total_count,
            tree: None,
        }
    }

    /// The number of rows on this page (not [`total_count`](Self::total_count),
    /// which counts the whole result).
    pub fn row_count(&self) -> usize {
        self.columns.first().map_or(0, Vec::len)
    }
}
