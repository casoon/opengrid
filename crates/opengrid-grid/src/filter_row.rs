//! The filter row as data (plan point 51): what a reader chose per column, and
//! the `filter` expression it means, with every literal typed for its column.
//!
//! Portable (issue #144): the element builds its query from it, and so does
//! `opengrid-wasm`'s `view_query` — one translation, wherever a view is read.

use opengrid_query::{CmpOp, FilterExpr};
use opengrid_types::{DataType, FieldName, Schema};

/// One row of the filter UI: a column, an operator and the typed value.
///
/// Values are always plain strings — the display schema is all `Utf8` in Phase B
/// and the result JSON carries no types (point 23). The literal is therefore sent
/// as a JSON string; comparing it works for text columns and is a documented
/// limitation for the others until point 23.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterEntry {
    /// The output column the comparison names.
    pub column: String,
    /// The chosen operator.
    pub op: FilterOp,
    /// The raw value text; an empty (or whitespace-only) value means "no filter",
    /// except for the operators that take none.
    pub value: String,
}

/// What the operator control can be set to (plan point 51).
///
/// `CmpOp` covers the comparisons; the two null tests are not comparisons in the
/// AST — they are their own filter nodes — so the control's choice needs a type
/// that can hold either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOp {
    Cmp(CmpOp),
    IsNull,
    IsNotNull,
}

impl FilterOp {
    /// Reads a wire token, including the two null tests.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "is_null" => Some(FilterOp::IsNull),
            "is_not_null" => Some(FilterOp::IsNotNull),
            other => CmpOp::parse(other).map(FilterOp::Cmp),
        }
    }

    /// The wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            FilterOp::Cmp(op) => op.as_str(),
            FilterOp::IsNull => "is_null",
            FilterOp::IsNotNull => "is_not_null",
        }
    }
}

/// What a filter row entry can be wrong about (plan point 51).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterProblem {
    /// The column whose input does not fit.
    pub column: String,
    /// What the user typed.
    pub value: String,
}

/// The `filter` expression of the filter row, with literals typed per column.
///
/// Until point 51 every value went out as a JSON string. Against a `Decimal`,
/// `Int64`, `Date` or `Bool` column that is a **type error** by the rules of
/// §Literaltypen — which is why filtering only ever worked on text. Now each
/// literal is written in the notation its column expects (§Typsystem: decimal as
/// a string, date `YYYY-MM-DD`, timestamp ISO-8601 `Z`, numbers as JSON numbers,
/// booleans as JSON booleans).
///
/// An entry the column cannot accept is **not** sent: it comes back as a
/// [`FilterProblem`], so the grid can say so instead of letting the engine or the
/// server answer with a validation error the user did not cause.
///
/// Entries with a blank value are skipped — except for the two operators that
/// take no value at all. A schema without the column is skipped rather than
/// turned into a malformed query.
pub fn filter_expr(
    entries: &[FilterEntry],
    schema: &Schema,
) -> Result<Option<FilterExpr>, Vec<FilterProblem>> {
    let mut comparisons = Vec::new();
    let mut problems = Vec::new();

    for entry in entries {
        let Ok(field) = FieldName::new(entry.column.as_str()) else {
            continue;
        };
        let Some(column) = schema.field(entry.column.as_str()) else {
            continue;
        };

        match entry.op {
            FilterOp::IsNull => comparisons.push(FilterExpr::IsNull { field }),
            FilterOp::IsNotNull => comparisons.push(FilterExpr::IsNotNull { field }),
            FilterOp::Cmp(op) => {
                if entry.value.trim().is_empty() {
                    continue;
                }
                match literal(&entry.value, column.data_type) {
                    Some(value) => comparisons.push(FilterExpr::Cmp { field, op, value }),
                    None => problems.push(FilterProblem {
                        column: entry.column.clone(),
                        value: entry.value.clone(),
                    }),
                }
            }
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(if comparisons.is_empty() {
        None
    } else {
        Some(FilterExpr::And(comparisons))
    })
}

/// One typed literal in the notation its column expects, or `None` when the text
/// is not a value of that type.
///
/// The checking is deliberately shallow — it decides whether the JSON is of the
/// right *kind*, and validation against the schema does the rest. What it must
/// not do is pass something through that will fail later: the user typed it, so
/// the user should hear about it here.
pub fn literal(text: &str, data_type: DataType) -> Option<opengrid_json::Json> {
    let text = text.trim();
    match data_type {
        DataType::Utf8 => Some(opengrid_json::Json::String(text.to_owned())),
        DataType::Bool => match text {
            "true" | "1" => Some(opengrid_json::Json::Bool(true)),
            "false" | "0" => Some(opengrid_json::Json::Bool(false)),
            _ => None,
        },
        DataType::Int64 => text.parse::<i64>().ok().map(Into::into),
        DataType::Float64 => {
            // E13: the three non-finite values travel as those exact words.
            if matches!(text, "NaN" | "Infinity" | "-Infinity") {
                return Some(opengrid_json::Json::String(text.to_owned()));
            }
            let number = text.parse::<f64>().ok()?;
            opengrid_json::Number::from_f64(number).map(opengrid_json::Json::Number)
        }
        // A decimal travels as a string so no precision is lost on the way
        // (§Typsystem). Checked here for shape only.
        DataType::Decimal { .. } => {
            let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
            let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
            let ok = !whole.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && fraction.bytes().all(|b| b.is_ascii_digit());
            ok.then(|| opengrid_json::Json::String(text.to_owned()))
        }
        DataType::Date => {
            // `YYYY-MM-DD`, which is what `<input type="date">` produces.
            let parts: Vec<&str> = text.split('-').collect();
            let ok = parts.len() == 3
                && parts[0].len() == 4
                && parts[1].len() == 2
                && parts[2].len() == 2
                && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()));
            ok.then(|| opengrid_json::Json::String(text.to_owned()))
        }
        DataType::Timestamp => {
            let ok = text.len() >= 20 && text.ends_with('Z') && text.contains('T');
            ok.then(|| opengrid_json::Json::String(text.to_owned()))
        }
    }
}
