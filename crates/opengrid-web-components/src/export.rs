//! The export notation in the browser (plan point 83).
//!
//! Two functions over one result JSON, so `exportRows` in `loader.js` (point
//! 84) can write an export piece by piece with the same code the server uses
//! (`opengrid-export`). They are exported from the module for `loader.js` and
//! are not part of the documented API (`docs/api.md`): a page exports through
//! `exportRows`.

use opengrid_datasource::wire::result_from_json;
use opengrid_export::{CsvOptions, csv_header, csv_rows, json_rows};
use wasm_bindgen::prelude::*;

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
