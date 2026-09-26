//! The export notation in the browser (plan point 83).
//!
//! Two functions over one result JSON, so `exportRows` in `loader.js` (point
//! 84) can write an export piece by piece with the same code the server uses
//! (`opengrid-export`). They are exported from the module for `loader.js` and
//! are not part of the documented API (`docs/api.md`): a page exports through
//! `exportRows`.
//!
//! A pivot is exported by the element that shows it, through the documented
//! `get_pivot` (issue #3); what that needs of the notation is here too.

use opengrid_datasource::wire::result_from_json;
#[cfg(feature = "pivot")]
use opengrid_export::PivotLabels;
use opengrid_export::{CsvOptions, csv_header, csv_rows, json_rows};
#[cfg(feature = "pivot")]
use opengrid_pivot::{PivotColumn, PivotResult};
#[cfg(feature = "pivot")]
use opengrid_types::{FieldName, Value};
use wasm_bindgen::prelude::*;

#[cfg(feature = "pivot")]
use crate::texts::GridTexts;

/// One piece of a CSV: with `header`, the header line (and the byte order
/// mark) first. `options`: `{ delimiter, bom, protectFormulas, null }`, each
/// optional (docs/guides/export.md, point 87).
#[wasm_bindgen(js_name = export_csv)]
pub fn export_csv(result_json: &str, options: JsValue, header: bool) -> Result<String, JsError> {
    let result = result_from_json(result_json).map_err(|error| JsError::new(&error.to_string()))?;
    let options = csv_options(&options)?;
    let mut out = String::new();
    if header {
        out.push_str(&csv_header(&result.schema, &options));
    }
    out.push_str(&csv_rows(&result, &options));
    Ok(out)
}

/// One piece of a JSON array: the rows as objects, joined by commas. `first`:
/// no row has been written yet (not "the first piece" — after an empty first
/// piece it is still true), so no comma leads. `loader.js` writes the brackets.
#[wasm_bindgen(js_name = export_json)]
pub fn export_json(result_json: &str, first: bool) -> Result<String, JsError> {
    let result = result_from_json(result_json).map_err(|error| JsError::new(&error.to_string()))?;
    Ok(json_rows(&result, first))
}

/// The options of `get_pivot` (issue #3): the CSV options of [`export_csv`],
/// and nothing else. The page calls `get_pivot` itself, with no `loader.js` in
/// between to check the keys, so a misspelt one — `delimeter` — is refused
/// here rather than exported past with a comma.
#[cfg(feature = "pivot")]
pub(crate) fn pivot_options(value: &JsValue) -> Result<CsvOptions, JsError> {
    if value.is_object() {
        let known = ["delimiter", "bom", "protectFormulas", "null"];
        for key in js_sys::Object::keys(value.unchecked_ref::<js_sys::Object>()).iter() {
            let key = key.as_string().unwrap_or_default();
            if !known.contains(&key.as_str()) {
                return Err(JsError::new(&format!(
                    "{key}: not an option (delimiter, bom, protectFormulas, null)"
                )));
            }
        }
    }
    csv_options(value)
}

/// The pivot answer an element shows, as CSV with that element's `texts`.
///
/// `answer` is the pivot wire form (plan point 53) exactly as the element
/// rendered it; it is read into a [`PivotResult`] so that the values go through
/// the same notation as every other export.
#[cfg(feature = "pivot")]
pub(crate) fn pivot_csv(
    answer: &str,
    texts: &GridTexts,
    options: &CsvOptions,
) -> Result<String, JsError> {
    let pivot = pivot_from_json(answer).map_err(|error| JsError::new(&error))?;
    Ok(opengrid_export::pivot_csv(&pivot, texts, options))
}

/// The element's words are the export's: the very functions it renders its
/// row and group headers with.
#[cfg(feature = "pivot")]
impl PivotLabels for GridTexts {
    fn dimension(&self, value: Option<&str>) -> String {
        GridTexts::dimension(self, value)
    }
    fn total(&self) -> String {
        self.total.clone()
    }
    fn subtotal(&self, value: &str) -> String {
        GridTexts::subtotal(self, value)
    }
}

/// Reads the pivot wire form back into a [`PivotResult`].
///
/// The cells are the ordinary typed result form. A column's **path** travels
/// as bare JSON scalars, without the column dimension's type, so a date comes
/// back as its text; that is enough here, where a path value is only ever read
/// as the text of a header, and that text is the same whichever type it had —
/// the one the element shows.
#[cfg(feature = "pivot")]
fn pivot_from_json(answer: &str) -> Result<PivotResult, String> {
    use serde_json::Value as Json;

    let body: Json = serde_json::from_str(answer).map_err(|error| error.to_string())?;
    let data = result_from_json(&body["result"].to_string()).map_err(|error| error.to_string())?;
    let row_levels = body["levels"]
        .as_array()
        .ok_or("result has no levels")?
        .iter()
        .map(|level| {
            level
                .as_u64()
                .and_then(|level| u16::try_from(level).ok())
                .ok_or("a level is a small number")
        })
        .collect::<Result<Vec<u16>, _>>()?;
    let columns = body["columns"]
        .as_array()
        .ok_or("result has no columns")?
        .iter()
        .map(|column| {
            let measure = column["measure"]
                .as_str()
                .ok_or("a column has no measure")?;
            let path = column["path"]
                .as_array()
                .ok_or("a column has no path")?
                .iter()
                .map(|value| match value {
                    Json::Bool(flag) => Value::Bool(*flag),
                    Json::Number(number) => match number.as_i64() {
                        Some(integer) => Value::Int64(integer),
                        None => number.as_f64().map_or(Value::Null, Value::Float64),
                    },
                    Json::String(text) => Value::Utf8(text.clone()),
                    _ => Value::Null,
                })
                .collect();
            Ok(PivotColumn {
                path,
                measure: FieldName::new(measure).map_err(|error| error.to_string())?,
            })
        })
        .collect::<Result<Vec<PivotColumn>, String>>()?;
    if row_levels.len() != data.row_count() {
        return Err("result has a level for each row".to_owned());
    }
    Ok(PivotResult {
        data,
        row_levels,
        columns,
    })
}

/// The options object: each key optional, a key that is there of its type —
/// `bom: "false"` is refused rather than read as the default.
fn csv_options(value: &JsValue) -> Result<CsvOptions, JsError> {
    let mut options = CsvOptions::default();
    if value.is_undefined() || value.is_null() {
        return Ok(options);
    }
    if !value.is_object() {
        return Err(JsError::new("options: an object"));
    }
    let get = |key: &str| {
        js_sys::Reflect::get(value, &JsValue::from_str(key))
            .ok()
            .filter(|v| !v.is_undefined())
    };
    let wrong = |key: &str, kind: &str| JsError::new(&format!("{key}: {kind}"));
    if let Some(delimiter) = get("delimiter") {
        let delimiter = delimiter
            .as_string()
            .ok_or_else(|| wrong("delimiter", "a string"))?;
        let mut chars = delimiter.chars();
        match (chars.next(), chars.next()) {
            (Some(one), None) => options.delimiter = one,
            _ => return Err(wrong("delimiter", "one character")),
        }
    }
    if let Some(bom) = get("bom") {
        options.bom = bom.as_bool().ok_or_else(|| wrong("bom", "a boolean"))?;
    }
    if let Some(protect) = get("protectFormulas") {
        options.protect_formulas = protect
            .as_bool()
            .ok_or_else(|| wrong("protectFormulas", "a boolean"))?;
    }
    if let Some(null) = get("null") {
        options.null = null.as_string().ok_or_else(|| wrong("null", "a string"))?;
    }
    options.check().map_err(JsError::new)?;
    Ok(options)
}
