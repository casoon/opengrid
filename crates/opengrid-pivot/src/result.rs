//! The shape of a pivot's answer.

use opengrid_datasource::QueryResult;
use opengrid_types::{FieldName, Value};

/// One generated leaf column: which column-dimension values it stands for, and
/// which measure it holds.
///
/// The **name** of the column in [`PivotResult::data`] is positional
/// (`total_0`, `total_1`, …) because a data value is not an identifier — `2026`
/// does not start with a letter, NULL has no spelling, and the empty string is
/// not a name. The meaning lives here, in `path`, where a UI can render it as
/// a heading and a screen reader can read it.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotColumn {
    /// The column-dimension values, outermost first. Empty when the pivot has
    /// no column dimension.
    pub path: Vec<Value>,
    /// The measure alias, as the pivot query declared it.
    pub measure: FieldName,
}

/// A pivot's answer: a table, plus the two things a table cannot say.
///
/// The cells are an ordinary [`QueryResult`] — column-oriented, Arrow-free
/// (E14), the same shape the wire format carries (E17) — so everything that
/// already handles a result handles this one. Beside it stand the two pieces of
/// structure a flat schema cannot hold: how deep each row is, and what each
/// column stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotResult {
    /// Row-dimension columns first, then one column per [`PivotColumn`].
    pub data: QueryResult,
    /// Per row: how many row dimensions are set.
    ///
    /// `rows.len()` is a detail row, `0` is the grand total, anything between is
    /// a subtotal. **A subtotal is not marked by NULL**: rule S10 gives NULL its
    /// own group, so a NULL in a dimension column is a real value and this
    /// number is the only thing that tells the two apart. It is the same job
    /// `GROUPING()` does in SQL.
    pub row_levels: Vec<u16>,
    /// One per generated column, in the order they appear in `data`.
    pub columns: Vec<PivotColumn>,
}

impl PivotResult {
    /// How many rows the pivot has, subtotals and grand total included.
    pub fn row_count(&self) -> usize {
        self.row_levels.len()
    }

    /// Whether the row at `index` is a subtotal or the grand total.
    pub fn is_total(&self, index: usize, row_dimensions: usize) -> bool {
        self.row_levels
            .get(index)
            .is_some_and(|level| usize::from(*level) < row_dimensions)
    }
}

/// The JSON form of a pivot answer (plan point 53).
///
/// The cells travel as the ordinary result form of E17, under `result`, so a
/// reader that already knows that shape needs to learn only two new keys: the
/// level per row and what each generated column stands for. A pivot is an
/// arrangement of a result, and the wire form says so.
pub fn pivot_to_json(result: &PivotResult, row_dimensions: &[FieldName]) -> String {
    let columns: Vec<serde_json::Value> = result
        .columns
        .iter()
        .map(|column| {
            serde_json::json!({
                "path": column.path,
                "measure": column.measure.as_str(),
            })
        })
        .collect();

    let body = serde_json::json!({
        "row_dimensions": row_dimensions
            .iter()
            .map(FieldName::as_str)
            .collect::<Vec<_>>(),
        "columns": columns,
        "levels": result.row_levels,
        "result": serde_json::from_str::<serde_json::Value>(
            &opengrid_datasource::wire::result_to_json(&result.data),
        )
        .expect("the result form is JSON"),
    });
    body.to_string()
}
