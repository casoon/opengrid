//! `opengrid-export` — a query result as CSV or JSON (plan point 83, E33).
//!
//! One implementation of the notation for every place that exports: the
//! server streams through it (point 85), the browser calls it through the
//! element module (point 84). It writes **pieces** — a header, then rows per
//! result — so a million rows never have to be one string.
//!
//! **Raw values, in the notation of the wire form** (E33): a decimal exact as
//! it was stored, a date `YYYY-MM-DD`, a timestamp ISO in UTC with its
//! microseconds, `NaN`/`Infinity`/`-Infinity` spelled out (E13). A display
//! format is the page's; an export is data for the next machine.
//!
//! A pivot is exported **as it is shown** ([`pivot_csv`], issue #3): its
//! values follow the same rules, its headers are the element's words.
//!
//! Portable: no `web-sys`, no `js-sys` (plan/spezifikation/11-crates.md
//! §Portabilität), no Arrow — [`QueryResult`] is Arrow-free (E14).

mod csv;
mod json;
mod pivot;

pub use csv::{CsvOptions, CsvWriter, csv_header, csv_rows};
pub use json::{JsonWriter, json_rows};
pub use pivot::{PATH_SEPARATOR, PivotLabels, pivot_csv};

use opengrid_types::Value;

/// A value as text in the wire notation; `None` for NULL.
///
/// The JSON writer does not use this — it writes the wire form itself — but
/// both follow the same rules, so a CSV cell and a JSON value read the same.
fn plain(value: &Value) -> Option<String> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(flag) => flag.to_string(),
        Value::Int64(number) => number.to_string(),
        // serde_json writes the shortest text that reads back to the same f64,
        // and spells the non-finite ones the way the wire does (E13).
        Value::Float64(_) => match serde_json::to_value(value) {
            Ok(serde_json::Value::String(name)) => name,
            Ok(number) => number.to_string(),
            Err(error) => unreachable!("a float serializes: {error}"),
        },
        Value::Decimal(decimal) => decimal.to_string(),
        Value::Utf8(text) => text.clone(),
        Value::Date(date) => date.to_string(),
        Value::Timestamp(timestamp) => timestamp.to_string(),
    })
}
