//! Per-column display formatting (plan point 42).
//!
//! # The line that is not crossed
//!
//! > Formatting is **display**. It may never reach a query.
//!
//! Sorting is binary (S4), time is UTC (S9), a decimal is exact (S8). A
//! formatted value is a string for eyes; the filter, the sort and every
//! comparison keep working on the value. A grid that sorted by what it printed
//! would have stopped honouring its own semantics.
//!
//! # Why `Intl` and not a Rust locale library
//!
//! The browser already carries the locale data. Pulling an ICU crate into WASM
//! would cost more than the whole engine does today (1.85 MB raw,
//! 12-qualitaet.md §WASM-Größe) — risk R3 — to do worse what
//! `Intl.NumberFormat` does natively.
//!
//! # The cost, named
//!
//! A formatter is a call across the WASM/JS boundary **per cell**, which R1
//! warns about. Two things keep it small: a column without a format never
//! crosses at all (the Rust side formats it), and an `Intl` options object is
//! turned into one formatter per column, once, instead of per cell.
//!
//! # The limit of the `Intl` shortcut
//!
//! `Intl.NumberFormat` takes a JavaScript number, so a decimal handed to it
//! goes through a `f64` — beyond 2^53 that loses digits. The **value** stays
//! exact (S8); only what an `Intl` options object prints does not. A column of
//! such decimals takes a function instead, which receives the exact text and
//! can format it without ever creating a number.

use opengrid_types::Value;

/// How a cell becomes text.
///
/// The grid renderer asks this for every cell it writes. [`Plain`] is the
/// answer when nobody supplied a format — and the one the portable tests use,
/// so the patch computation stays testable without a browser.
pub trait CellFormat {
    /// The text of `value` in the column at `column`.
    fn text(&self, column: usize, value: &Value) -> String;
}

/// The built-in rendering: the value's own notation.
///
/// NULL is empty — the column header says which column, and "no value" is not a
/// word the data chose.
pub struct Plain;

impl CellFormat for Plain {
    fn text(&self, _column: usize, value: &Value) -> String {
        plain_text(value)
    }
}

/// The value as the project writes it: decimals exact, dates ISO, UTC.
pub fn plain_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Int64(number) => number.to_string(),
        Value::Float64(number) => number.to_string(),
        Value::Decimal(decimal) => decimal.to_string(),
        Value::Utf8(text) => text.clone(),
        Value::Date(date) => date.to_string(),
        Value::Timestamp(timestamp) => timestamp.to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::{formats, set_formats_for, store};

/// Per-host storage of the column formats, mirroring the texts seam.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use opengrid_types::{Schema, Value};
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::HtmlElement;

    use super::{CellFormat, plain_text};

    thread_local! {
        static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
        static FORMATS: RefCell<HashMap<u32, Rc<ColumnFormats>>> = RefCell::new(HashMap::new());
    }

    fn id_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.formats_id")
    }

    fn id_of(host: &HtmlElement) -> Option<u32> {
        js_sys::Reflect::get(host.as_ref(), id_symbol().as_ref())
            .ok()
            .and_then(|value| value.as_f64())
            .map(|id| id as u32)
    }

    /// The formats a host carries, keyed by column name.
    ///
    /// Each entry is a JS function taking the **plain** text of the cell and
    /// its JSON value, and answering the text to show. An `Intl` options object
    /// is turned into such a function once, when the formats are stored — not
    /// per cell.
    pub struct ColumnFormats {
        by_name: HashMap<String, js_sys::Function>,
    }

    impl ColumnFormats {
        /// Reads a plain JS object `{ column: fn | IntlOptions }`.
        pub fn from_js(value: &JsValue) -> Self {
            let mut by_name = HashMap::new();
            let Some(object) = value.dyn_ref::<js_sys::Object>() else {
                return Self { by_name };
            };
            for key in js_sys::Object::keys(object).iter() {
                let Some(name) = key.as_string() else {
                    continue;
                };
                let Ok(entry) = js_sys::Reflect::get(object, &key) else {
                    continue;
                };
                if let Ok(function) = entry.clone().dyn_into::<js_sys::Function>() {
                    by_name.insert(name, function);
                } else if let Some(function) = intl_function(&entry) {
                    by_name.insert(name, function);
                }
            }
            Self { by_name }
        }

        /// Whether any column is formatted — a grid without formats never
        /// crosses the boundary at all.
        pub fn is_empty(&self) -> bool {
            self.by_name.is_empty()
        }

        fn function(&self, name: &str) -> Option<&js_sys::Function> {
            self.by_name.get(name)
        }
    }

    /// Builds a formatter from an `Intl` options object, once.
    ///
    /// `{ "kind": "number", "locale": "de-DE", ... }` — the rest of the object
    /// goes to `Intl` unchanged, so everything `Intl` can do is available
    /// without this module knowing about it.
    fn intl_function(options: &JsValue) -> Option<js_sys::Function> {
        let kind = js_sys::Reflect::get(options, &JsValue::from_str("kind"))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "number".to_owned());
        let locale = js_sys::Reflect::get(options, &JsValue::from_str("locale"))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_default();

        // Built in JS so the options object travels as it is; the closure is
        // created once per column and reused for every cell.
        let factory = js_sys::Function::new_with_args(
            "kind, locale, options",
            "const loc = locale || undefined;
             const format = kind === 'date'
               ? new Intl.DateTimeFormat(loc, options)
               : new Intl.NumberFormat(loc, options);
             return (text, value) => {
               if (value === null || value === undefined) return '';
               if (kind === 'date') {
                 const at = Date.parse(typeof value === 'string' && value.length === 10
                   ? value + 'T00:00:00Z' : value);
                 return Number.isNaN(at) ? text : format.format(new Date(at));
               }
               const number = Number(value);
               return Number.isFinite(number) ? format.format(number) : text;
             };",
        );
        factory
            .call3(
                &JsValue::NULL,
                &JsValue::from_str(&kind),
                &JsValue::from_str(&locale),
                options,
            )
            .ok()
            .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
    }

    /// Attaches `formats` to `host`, replacing any previous ones.
    pub fn store(host: &HtmlElement, formats: Rc<ColumnFormats>) {
        let id = id_of(host).unwrap_or_else(|| {
            let id = NEXT_ID.with(|next| {
                let mut next = next.borrow_mut();
                let id = *next;
                *next += 1;
                id
            });
            let _ = js_sys::Reflect::set(
                host.as_ref(),
                id_symbol().as_ref(),
                &JsValue::from_f64(f64::from(id)),
            );
            id
        });
        FORMATS.with(|map| map.borrow_mut().insert(id, formats));
    }

    /// The formats attached to `host`, or empty ones.
    pub fn formats(host: &HtmlElement) -> Rc<ColumnFormats> {
        id_of(host)
            .and_then(|id| FORMATS.with(|map| map.borrow().get(&id).cloned()))
            .unwrap_or_else(|| {
                Rc::new(ColumnFormats {
                    by_name: HashMap::new(),
                })
            })
    }

    /// Stores formats read from a JS object.
    pub fn set_formats_for(host: &HtmlElement, value: &JsValue) {
        store(host, Rc::new(ColumnFormats::from_js(value)));
    }

    /// The formats bound to a schema, ready for the renderer.
    pub struct Formatter {
        columns: Vec<Option<js_sys::Function>>,
    }

    impl Formatter {
        /// Resolves the column names against `schema` once per render.
        pub fn new(formats: &ColumnFormats, schema: &Schema) -> Self {
            Self {
                columns: schema
                    .fields()
                    .iter()
                    .map(|field| formats.function(field.name.as_str()).cloned())
                    .collect(),
            }
        }
    }

    impl CellFormat for Formatter {
        fn text(&self, column: usize, value: &Value) -> String {
            let plain = plain_text(value);
            let Some(Some(function)) = self.columns.get(column) else {
                // No format for this column: no boundary crossing (R1).
                return plain;
            };
            let json = serde_json::to_string(value)
                .ok()
                .and_then(|text| js_sys::JSON::parse(&text).ok())
                .unwrap_or(JsValue::NULL);
            function
                .call2(&JsValue::NULL, &JsValue::from_str(&plain), &json)
                .ok()
                .and_then(|value| value.as_string())
                // A formatter that throws or answers nothing must not blank the
                // cell: the value is still there.
                .unwrap_or(plain)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::Formatter;

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Date, Decimal};

    /// The built-in rendering keeps every notation the project decided on.
    #[test]
    fn the_plain_text_is_the_projects_own_notation() {
        assert_eq!(Plain.text(0, &Value::Null), "", "NULL is not a word");
        assert_eq!(
            Plain.text(0, &Value::Decimal(Decimal::new(-1050, 2))),
            "-10.50",
            "a decimal stays exact — never through an f64"
        );
        assert_eq!(
            Plain.text(0, &Value::Date(Date::from_ymd(2026, 3, 9).unwrap())),
            "2026-03-09"
        );
        assert_eq!(Plain.text(0, &Value::Bool(true)), "true");
    }
}
