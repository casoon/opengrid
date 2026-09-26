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
use wasm_bindgen::prelude::*;

#[cfg(feature = "pivot")]
use crate::texts::GridTexts;

/// The keys of the CSV options object, in the order [`csv_options`] reads
/// them. One list, so the reader and the key check of `get_pivot` cannot
/// disagree about what an option is. `exportRows` keeps the same list as
/// `CSV_OPTIONS` in packages/opengrid/loader.js to split its own options from
/// these; the two must stay in step (checked in `api.rs`, since this module is
/// wasm32-only and its tests would not run on the host).
const CSV_OPTION_KEYS: [&str; 4] = ["delimiter", "bom", "protectFormulas", "null"];

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
        for key in js_sys::Object::keys(value.unchecked_ref::<js_sys::Object>()).iter() {
            let key = key.as_string().unwrap_or_default();
            if !CSV_OPTION_KEYS.contains(&key.as_str()) {
                return Err(JsError::new(&format!(
                    "{key}: not an option ({})",
                    CSV_OPTION_KEYS.join(", ")
                )));
            }
        }
    }
    csv_options(value)
}

/// The pivot answer an element shows, as CSV with that element's `texts`.
///
/// `answer` is the pivot wire form (plan point 53) exactly as the element
/// rendered it. It is read back strictly (`pivot_from_json`), because a page's
/// own provider may have sent it: an answer of the wrong shape is an error with
/// a sentence, not a trap in the writer.
#[cfg(feature = "pivot")]
pub(crate) fn pivot_csv(
    answer: &str,
    texts: &GridTexts,
    options: &CsvOptions,
) -> Result<String, JsError> {
    let (pivot, _) = opengrid_pivot::pivot_from_json(answer)
        .map_err(|error| JsError::new(&format!("the shown pivot: {error}")))?;
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

/// The options object: each key optional, a key that is there of its type —
/// `bom: "false"` is refused rather than read as the default.
fn csv_options(value: &JsValue) -> Result<CsvOptions, JsError> {
    let [delimiter_key, bom_key, protect_key, null_key] = CSV_OPTION_KEYS;
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
    if let Some(delimiter) = get(delimiter_key) {
        let delimiter = delimiter
            .as_string()
            .ok_or_else(|| wrong(delimiter_key, "a string"))?;
        let mut chars = delimiter.chars();
        match (chars.next(), chars.next()) {
            (Some(one), None) => options.delimiter = one,
            _ => return Err(wrong(delimiter_key, "one character")),
        }
    }
    if let Some(bom) = get(bom_key) {
        options.bom = bom.as_bool().ok_or_else(|| wrong(bom_key, "a boolean"))?;
    }
    if let Some(protect) = get(protect_key) {
        options.protect_formulas = protect
            .as_bool()
            .ok_or_else(|| wrong(protect_key, "a boolean"))?;
    }
    if let Some(null) = get(null_key) {
        options.null = null
            .as_string()
            .ok_or_else(|| wrong(null_key, "a string"))?;
    }
    options.check().map_err(JsError::new)?;
    Ok(options)
}
