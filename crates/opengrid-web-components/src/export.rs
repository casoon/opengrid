//! The export notation in the browser (plan point 83).
//!
//! Two functions over one result — its JSON or its binary form — so `exportRows` in `loader.js` (point
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
use opengrid_export::{CsvOptions, csv_header_named, csv_rows, json_rows, with_tree_columns};
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
pub fn export_csv(
    answer: JsValue,
    options: JsValue,
    header: bool,
    headers: JsValue,
) -> Result<String, JsError> {
    let result = read(&answer)?;
    // A flat tree brings its level, path and match columns (T8, #166).
    let result = with_tree_columns(&result);
    let options = csv_options(&options)?;
    let mut out = String::new();
    if header {
        out.push_str(&csv_header_named(
            &result.schema,
            &options,
            &header_texts(&headers),
        ));
    }
    out.push_str(&csv_rows(&result, &options));
    Ok(out)
}

/// The header texts a page gave, `{ column: "Band · Column" }` (issue #353):
/// strings only, anything else is left out.
fn header_texts(headers: &JsValue) -> std::collections::BTreeMap<String, String> {
    let mut texts = std::collections::BTreeMap::new();
    if headers.is_object() {
        for entry in js_sys::Object::entries(&js_sys::Object::from(headers.clone())).iter() {
            let pair = js_sys::Array::from(&entry);
            if let (Some(name), Some(text)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                texts.insert(name, text);
            }
        }
    }
    texts
}

/// One piece of a JSON array: the rows as objects, joined by commas. `first`:
/// no row has been written yet (not "the first piece" — after an empty first
/// piece it is still true), so no comma leads. `loader.js` writes the brackets.
#[wasm_bindgen(js_name = export_json)]
pub fn export_json(answer: JsValue, first: bool) -> Result<String, JsError> {
    let result = read(&answer)?;
    Ok(json_rows(&result, first))
}

/// A provider's answer as a result: the result JSON, or the binary form (E35).
fn read(answer: &JsValue) -> Result<opengrid_datasource::QueryResult, JsError> {
    match crate::element::answer(answer) {
        Some(crate::element::Answer::Json(json)) => {
            result_from_json(&json).map_err(|error| JsError::new(&error.to_string()))
        }
        // A tree's piece carries its part (T8, #166): read it with it, or a
        // worker's answer to a tree's export is refused.
        Some(crate::element::Answer::Binary(bytes)) => {
            crate::shared::result_from_bytes(&bytes).map_err(|error| JsError::new(&error))
        }
        None => Err(JsError::new(crate::element::NOT_AN_ANSWER)),
    }
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
///
/// `calcs` are the calculated measures of the shown pivot, by alias (issue
/// #329): their columns are headed by what they show, as on the screen.
#[cfg(feature = "pivot")]
pub(crate) fn pivot_csv(
    answer: &str,
    texts: &GridTexts,
    calcs: &[(String, opengrid_pivot::Calc)],
    options: &CsvOptions,
) -> Result<String, JsError> {
    let (pivot, _) = opengrid_pivot::pivot_from_json(answer)
        .map_err(|error| JsError::new(&format!("the shown pivot: {error}")))?;
    Ok(opengrid_export::pivot_csv(
        &pivot,
        &Shown { texts, calcs },
        options,
    ))
}

/// The element's words for the pivot it shows: its texts, and which of its
/// measures are calculated.
#[cfg(feature = "pivot")]
struct Shown<'a> {
    texts: &'a GridTexts,
    calcs: &'a [(String, opengrid_pivot::Calc)],
}

/// The words a pivot's export has no data for, as `host` has them now: the
/// texts for a NULL and for an empty dimension value, for the grand total and
/// for a subtotal (with its `{value}`), then what a calculated measure is
/// shown as — a share of the row, the column, the total, a running total
/// (issue #329) — in that order.
///
/// For `exportPivot` in `loader.js` (issue #331), which hands them to the
/// server that writes the workbook; in the order of `PIVOT_TEXTS` there. Like
/// [`export_csv`], exported for the loader and not part of the documented API.
#[cfg(feature = "pivot")]
#[wasm_bindgen(js_name = pivot_words)]
pub fn pivot_words(host: &web_sys::HtmlElement) -> Vec<String> {
    let texts = crate::texts::texts(host);
    vec![
        texts.no_value.clone(),
        texts.empty_value.clone(),
        texts.total.clone(),
        texts.subtotal.clone(),
        texts.share_of_row.clone(),
        texts.share_of_column.clone(),
        texts.share_of_total.clone(),
        texts.running_total.clone(),
    ]
}

/// The element's words are the export's: the very functions it renders its
/// row and group headers with.
#[cfg(feature = "pivot")]
impl PivotLabels for Shown<'_> {
    fn dimension(&self, value: Option<&str>) -> String {
        self.texts.dimension(value)
    }
    fn total(&self) -> String {
        self.texts.total.clone()
    }
    fn subtotal(&self, value: &str) -> String {
        self.texts.subtotal(value)
    }
    fn calc(&self, alias: &str) -> Option<String> {
        let (_, calc) = self.calcs.iter().find(|(measure, _)| measure == alias)?;
        Some(self.texts.calc(Some(*calc)).to_owned())
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
