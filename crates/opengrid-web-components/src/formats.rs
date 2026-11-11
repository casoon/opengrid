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
///
/// Test-only: in the browser there is always a [`Formatter`], and one without
/// formats renders exactly this. It exists so the portable patch computation
/// stays testable without a browser.
#[cfg(test)]
pub struct Plain;

#[cfg(test)]
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
        // The elements, comma-separated; the grid draws each as a chip (issue #185).
        Value::List(items) => items.iter().map(plain_text).collect::<Vec<_>>().join(", "),
    }
}

/// The value as an event gives it to the page: [`plain_text`], except that a
/// list is the JSON array it is on the wire (issue #185) — the text a page can
/// read back, where `a, b` could not tell `["a, b"]` from `["a","b"]`.
pub fn event_text(value: &Value) -> String {
    match value {
        Value::List(_) => opengrid_json::ToJson::to_json(value).to_string(),
        other => plain_text(other),
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::formats;
#[cfg(all(target_arch = "wasm32", any(feature = "grid", feature = "pivot")))]
pub use host::set_formats_for;

/// Per-host storage of the column formats.
///
/// **On the host, not in a Rust table** (point 74), like the provider: a
/// format function is the page's, and a closure over a component's ref reaches
/// the host. Held from WASM, that cycle could never be collected; on the host
/// it is ordinary garbage.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::collections::HashMap;
    use std::rc::Rc;

    use opengrid_types::{Schema, Value};
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::HtmlElement;

    use super::{CellFormat, plain_text};

    // A static snippet, not code built at run time: `new Function` throws
    // under any CSP without `'unsafe-eval'` — a strict page's, and every MCP
    // Apps host's (issue #139).
    #[wasm_bindgen(inline_js = "
// The language of the page at `host`: the nearest `lang` attribute, through
// shadow roots as well; empty when there is none or it is empty.
export function __opengrid_page_lang(host) {
    for (let node = host; node; node = node.parentElement || (node.getRootNode && node.getRootNode().host) || null) {
        const lang = node.getAttribute && node.getAttribute('lang');
        if (lang !== null && lang !== undefined) return lang.trim();
    }
    return '';
}

// One observer for every host whose formats follow the page's language. It
// holds the hosts weakly, so a removed grid is still collected.
const followers = new Set();
const known = new WeakSet();
const roots = new WeakSet();
let observer = null;
export function __opengrid_follow_lang(host, relocalize) {
    if (!known.has(host)) {
        known.add(host);
        followers.add(new WeakRef(host));
    }
    if (!observer) {
        observer = new MutationObserver(() => {
            for (const ref of [...followers]) {
                const host = ref.deref();
                if (!host) followers.delete(ref);
                else if (host.isConnected) relocalize(host);
            }
        });
    }
    const watch = (node) => {
        if (node && !roots.has(node)) {
            roots.add(node);
            observer.observe(node, { attributes: true, attributeFilter: ['lang'], subtree: true });
        }
    };
    watch(host.ownerDocument.documentElement);
    const root = host.getRootNode();
    if (root !== host.ownerDocument) watch(root);
}

export function __opengrid_intl_formatter(kind, locale, options) {
    const loc = locale || undefined;
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
    };
}
")]
    extern "C" {
        #[wasm_bindgen(catch, js_name = __opengrid_intl_formatter)]
        fn intl_formatter(kind: &str, locale: &str, options: &JsValue) -> Result<JsValue, JsValue>;
        #[cfg(any(feature = "grid", feature = "pivot"))]
        #[wasm_bindgen(js_name = __opengrid_page_lang)]
        fn page_lang(host: &HtmlElement) -> String;
        #[cfg(any(feature = "grid", feature = "pivot"))]
        #[wasm_bindgen(js_name = __opengrid_follow_lang)]
        fn follow_lang(host: &HtmlElement, relocalize: &js_sys::Function);
    }

    /// The formats as the page gave them, before `Intl` options became
    /// functions — what a change of `lang` resolves again.
    #[cfg(any(feature = "grid", feature = "pivot"))]
    fn source_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.formats.source")
    }

    /// The page language the formats were resolved with.
    #[cfg(any(feature = "grid", feature = "pivot"))]
    fn lang_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.formats.lang")
    }

    /// The global symbol the resolved formats are stored under.
    fn formats_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.formats")
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
        /// Reads a plain JS object `{ column: fn | IntlOptions }`; an `Intl`
        /// entry without a `locale` takes `page_lang` (empty: the browser's).
        pub fn from_js(value: &JsValue, page_lang: &str) -> Self {
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
                } else if let Some(function) = intl_function(&entry, page_lang) {
                    by_name.insert(name, function);
                }
            }
            Self { by_name }
        }

        pub fn function(&self, name: &str) -> Option<&js_sys::Function> {
            self.by_name.get(name)
        }
    }

    /// Builds a formatter from an `Intl` options object, once.
    ///
    /// `{ "kind": "number", "locale": "de-DE", ... }` — the rest of the object
    /// goes to `Intl` unchanged, so everything `Intl` can do is available
    /// without this module knowing about it.
    fn intl_function(options: &JsValue, page_lang: &str) -> Option<js_sys::Function> {
        let kind = js_sys::Reflect::get(options, &JsValue::from_str("kind"))
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "number".to_owned());
        let locale = js_sys::Reflect::get(options, &JsValue::from_str("locale"))
            .ok()
            .and_then(|value| value.as_string())
            .filter(|locale| !locale.trim().is_empty())
            // Explicit `locale`, then the page's `lang`, then the browser's.
            .unwrap_or_else(|| page_lang.to_owned());

        // Built in JS so the options object travels as it is; the closure is
        // created once per column and reused for every cell.
        intl_formatter(&kind, &locale, options)
            .ok()
            .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
    }

    /// Resolves `value` — `Intl` options become functions here, once — and
    /// attaches the result to `host`, replacing any previous formats.
    #[cfg(any(feature = "grid", feature = "pivot"))]
    pub fn set_formats_for(host: &HtmlElement, value: &JsValue) {
        let lang = page_lang(host);
        let resolved = js_sys::Object::new();
        for (name, function) in ColumnFormats::from_js(value, &lang).by_name {
            let _ = js_sys::Reflect::set(&resolved, &JsValue::from_str(&name), &function);
        }
        let _ = js_sys::Reflect::set(host.as_ref(), formats_symbol().as_ref(), &resolved);
        let _ = js_sys::Reflect::set(host.as_ref(), source_symbol().as_ref(), value);
        let _ = js_sys::Reflect::set(
            host.as_ref(),
            lang_symbol().as_ref(),
            &JsValue::from_str(&lang),
        );
        if uses_page_lang(value) {
            follow(host);
        }
    }

    /// Whether an `Intl` entry has no `locale` of its own, so the page's `lang`
    /// decides how it reads.
    #[cfg(any(feature = "grid", feature = "pivot"))]
    fn uses_page_lang(value: &JsValue) -> bool {
        let Some(object) = value.dyn_ref::<js_sys::Object>() else {
            return false;
        };
        js_sys::Object::values(object).iter().any(|entry| {
            !entry.is_function()
                && js_sys::Reflect::get(&entry, &JsValue::from_str("locale"))
                    .ok()
                    .and_then(|locale| locale.as_string())
                    .is_none_or(|locale| locale.trim().is_empty())
        })
    }

    /// Resolves the formats again when the page's language changes: the
    /// elements redraw with the numbers and dates of the new language.
    #[cfg(any(feature = "grid", feature = "pivot"))]
    fn follow(host: &HtmlElement) {
        thread_local! {
            static RELOCALIZE: js_sys::Function = wasm_bindgen::closure::Closure::<dyn FnMut(HtmlElement)>::new(
                |host: HtmlElement| {
                    let used = js_sys::Reflect::get(host.as_ref(), lang_symbol().as_ref())
                        .ok()
                        .and_then(|lang| lang.as_string())
                        .unwrap_or_default();
                    if page_lang(&host) == used {
                        return;
                    }
                    let source = js_sys::Reflect::get(host.as_ref(), source_symbol().as_ref())
                        .unwrap_or(JsValue::UNDEFINED);
                    crate::element::set_formats(&host, source);
                },
            )
            .into_js_value()
            .unchecked_into();
        }
        RELOCALIZE.with(|relocalize| follow_lang(host, relocalize));
    }

    /// The formats attached to `host`, or empty ones.
    pub fn formats(host: &HtmlElement) -> Rc<ColumnFormats> {
        let stored = js_sys::Reflect::get(host.as_ref(), formats_symbol().as_ref())
            .unwrap_or(JsValue::UNDEFINED);
        // What is stored is functions already, so no language is needed.
        Rc::new(ColumnFormats::from_js(&stored, ""))
    }

    /// The formats bound to a schema, ready for the renderer.
    pub struct Formatter {
        columns: Vec<Option<js_sys::Function>>,
        /// The labels of the columns that store keys (issue #326); empty
        /// where no column has any.
        #[cfg(feature = "grid")]
        lookups: Vec<Option<Rc<crate::presentation::Lookup>>>,
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
                #[cfg(feature = "grid")]
                lookups: Vec::new(),
            }
        }

        /// With the labels `styles` gives the columns that store keys: such a
        /// column shows its label, and the label leads over a format.
        #[cfg(feature = "grid")]
        pub fn with_lookups(
            mut self,
            styles: &crate::presentation::ColumnStyles,
            schema: &Schema,
        ) -> Self {
            self.lookups = schema
                .fields()
                .iter()
                .map(|field| styles.lookup(field.name.as_str()).cloned())
                .collect();
            self
        }
    }

    impl CellFormat for Formatter {
        fn text(&self, column: usize, value: &Value) -> String {
            #[cfg(feature = "grid")]
            if let Some(Some(lookup)) = self.lookups.get(column) {
                return lookup.text(value);
            }
            let plain = plain_text(value);
            let Some(Some(function)) = self.columns.get(column) else {
                // No format for this column: no boundary crossing (R1).
                return plain;
            };
            let json =
                js_sys::JSON::parse(&opengrid_json::to_string(value)).unwrap_or(JsValue::NULL);
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

#[cfg(all(target_arch = "wasm32", feature = "pivot"))]
pub use host::ColumnFormats;
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
