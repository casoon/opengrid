//! The texts the components write themselves (plan point 48).
//!
//! Everything a user reads that does not come from the data lives here: the
//! status line, the labels of the filter row, the operator names. Two rules
//! shape the module.
//!
//! * **One language, English.** The rest of the public API is English
//!   (`window-size`, `datasource`, the part names, the events), so the built-in
//!   texts are too. Before this point they were half German and half English,
//!   which a screen reader announced in whatever voice the document's `lang`
//!   selected (WCAG 3.1.2).
//! * **The page has the last word.** `set_texts(host, texts)` overrides any
//!   subset of them — the same seam as `set_provider`, so a page wires texts and
//!   data the same way. A `lang` given with them is written onto the component's
//!   content, so the announcement is spoken in the language of the text and not
//!   of the page.
//!
//! What is deliberately *not* here: the diagnoses of engine and provider
//! ("result has no total_count"). They are developer-facing and travel behind a
//! translated sentence, as the `{cause}` of [`GridTexts::error`].
//!
//! The struct and its substitution are portable — they are unit-tested on the
//! host; only the per-host registry and the JS conversion are `wasm32`-only.

/// The texts one component instance renders.
///
/// Every field is a template; `{count}`, `{column}` and `{cause}` are the only
/// placeholders, and each field documents which of them it may use. A field the
/// page does not override keeps the English default of [`Default`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridTexts {
    /// BCP 47 language tag written onto the elements that carry **these** texts
    /// — the filter row and the status line — or empty to leave the document's
    /// language in place.
    ///
    /// Deliberately not on a wrapper around the table: the cells and the column
    /// headers are the page's data, in the page's language, and declaring them
    /// English would be the very WCAG 3.1.2 failure this point removes.
    pub lang: String,
    /// While a query runs.
    pub loading: String,
    /// Exactly one matching row. May use `{count}`.
    pub matches_one: String,
    /// More than one matching row. May use `{count}`.
    pub matches_other: String,
    /// No matching row. The zero case has its own state, so no placeholder.
    pub empty: String,
    /// A failed query. Should use `{cause}` — the untranslated diagnosis.
    pub error: String,
    /// A failed query whose cause is empty.
    pub error_unknown: String,
    /// Accessible name of the filter row.
    pub filter_group: String,
    /// Accessible name of a column's operator control. May use `{column}`.
    pub operator_label: String,
    /// Accessible name of a column's value input. May use `{column}`.
    pub value_label: String,
    /// The button that empties the filter row.
    pub clear: String,
    /// A filter input the column cannot hold (point 51). May use `{column}` and
    /// `{value}`.
    pub filter_invalid: String,
    /// The header of a pivot group whose dimension value is NULL.
    ///
    /// A header cell must not be empty: a sighted reader sees a blank and
    /// understands "no country", a screen reader announces nothing at all.
    pub no_value: String,
    /// The header of a pivot group whose dimension value is the **empty
    /// string** — a different group from NULL (rule S14), and it has to look
    /// and sound different too.
    pub empty_value: String,
    /// The row header of a pivot's grand total (plan point 32).
    pub total: String,
    /// The row header of a pivot's subtotal. May use `{value}` — the value of
    /// the dimension the subtotal closes.
    pub subtotal: String,
    /// The readable names of the filter operators, in the order of
    /// [`FILTER_OPERATORS`](crate::grid::FILTER_OPERATORS). The `value` of each
    /// option stays the wire token, so the query is unaffected.
    pub operators: Vec<String>,
}

/// The readable operator names, in the order of
/// [`FILTER_OPERATORS`](crate::grid::FILTER_OPERATORS).
///
/// `gte` is not a word. The wire token stays the option's `value`; only what the
/// user reads changes.
pub const DEFAULT_OPERATORS: &[&str] = &[
    "contains",
    "starts with",
    "is",
    "is not",
    "greater than",
    "greater or equal",
    "less than",
    "less or equal",
    // Deliberately not "is empty": an empty string **is** a value (rule S14),
    // and calling the absence of a value "empty" would merge the two.
    "has no value",
    "has a value",
];

/// The labels are indexed by the wire tokens' position, so the two lists must
/// have the same length — otherwise an added operator would silently render its
/// raw token.
const _: () = assert!(DEFAULT_OPERATORS.len() == crate::grid::FILTER_OPERATORS.len());

impl Default for GridTexts {
    fn default() -> Self {
        Self {
            lang: "en".to_owned(),
            loading: "Loading …".to_owned(),
            matches_one: "{count} match".to_owned(),
            matches_other: "{count} matches".to_owned(),
            empty: "No matches".to_owned(),
            error: "The data could not be loaded: {cause}".to_owned(),
            error_unknown: "The data could not be loaded.".to_owned(),
            filter_group: "Filter".to_owned(),
            operator_label: "{column} operator".to_owned(),
            value_label: "{column} value".to_owned(),
            clear: "Clear".to_owned(),
            filter_invalid: "{column}: {value} is not a value for this column".to_owned(),
            no_value: "(no value)".to_owned(),
            empty_value: "(empty)".to_owned(),
            total: "Total".to_owned(),
            subtotal: "Total {value}".to_owned(),
            operators: DEFAULT_OPERATORS
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
    }
}

impl GridTexts {
    /// The result count line for `count` matching rows (`count >= 1`).
    pub fn matches(&self, count: u64) -> String {
        let template = if count == 1 {
            &self.matches_one
        } else {
            &self.matches_other
        };
        fill(template, "count", &count.to_string())
    }

    /// The sentence for a failed query. An empty cause takes
    /// [`error_unknown`](Self::error_unknown), so no template ever renders a
    /// dangling separator.
    pub fn error(&self, cause: &str) -> String {
        let cause = cause.trim();
        if cause.is_empty() {
            self.error_unknown.clone()
        } else {
            fill(&self.error, "cause", cause)
        }
    }

    /// The accessible name of `column`'s operator control.
    pub fn operator_label(&self, column: &str) -> String {
        fill(&self.operator_label, "column", column)
    }

    /// The accessible name of `column`'s value input.
    pub fn value_label(&self, column: &str) -> String {
        fill(&self.value_label, "column", column)
    }

    /// The sentence for a filter input the column cannot hold.
    pub fn filter_invalid(&self, column: &str, value: &str) -> String {
        fill(
            &fill(&self.filter_invalid, "column", column),
            "value",
            value,
        )
    }

    /// A dimension value as a **header** reads it.
    ///
    /// NULL and the empty string are two different groups (S10, S14) and two
    /// different words; neither may render as an empty header cell.
    pub fn dimension(&self, value: Option<&str>) -> String {
        match value {
            None => self.no_value.clone(),
            Some("") => self.empty_value.clone(),
            Some(text) => text.to_owned(),
        }
    }

    /// The row header of the subtotal that closes `value`.
    ///
    /// A subtotal has to be **readable** as one, not only shaded: colour alone
    /// is not information (WCAG 1.4.1), and a screen reader announces this text
    /// where a sighted reader sees the shading.
    pub fn subtotal(&self, value: &str) -> String {
        fill(&self.subtotal, "value", value)
    }

    /// The readable name of the operator at `index`, falling back to the wire
    /// token when there is none.
    ///
    /// The list is indexed like
    /// [`FILTER_OPERATORS`](crate::grid::FILTER_OPERATORS) — a compile-time
    /// assertion keeps the two the same length — and an empty entry means "no
    /// label", so a page can translate one operator without restating the rest.
    pub fn operator(&self, index: usize, token: &str) -> String {
        match self.operators.get(index) {
            Some(name) if !name.is_empty() => name.clone(),
            _ => token.to_owned(),
        }
    }
}

/// Replaces every `{name}` in `template`.
fn fill(template: &str, name: &str, value: &str) -> String {
    template.replace(&format!("{{{name}}}"), value)
}

#[cfg(target_arch = "wasm32")]
pub use host::texts;
#[cfg(target_arch = "wasm32")]
pub(crate) use host::{from_js, store};

/// Per-host storage of the texts, mirroring the provider seam.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use wasm_bindgen::JsValue;
    use web_sys::HtmlElement;

    use super::GridTexts;

    thread_local! {
        static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
        static TEXTS: RefCell<HashMap<u32, Rc<GridTexts>>> = RefCell::new(HashMap::new());
    }

    /// The global symbol the id is stored under (as the provider seam).
    fn id_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.texts_id")
    }

    /// The id `host` already carries, if any.
    fn id_of(host: &HtmlElement) -> Option<u32> {
        js_sys::Reflect::get(host.as_ref(), id_symbol().as_ref())
            .ok()
            .and_then(|value| value.as_f64())
            .map(|id| id as u32)
    }

    /// Attaches `texts` to `host`, replacing any previous ones.
    ///
    /// A host that already has an id keeps it, so calling `set_texts` twice
    /// replaces the entry instead of leaving the first one behind.
    pub(crate) fn store(host: &HtmlElement, texts: Rc<GridTexts>) {
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
        TEXTS.with(|map| map.borrow_mut().insert(id, texts));
    }

    /// The texts of `host` — the page's, or the English defaults.
    ///
    /// The default is built once per thread: this is read on every rendered
    /// frame, and a grid that never sets texts is the common case.
    pub fn texts(host: &HtmlElement) -> Rc<GridTexts> {
        thread_local! {
            static DEFAULT: Rc<GridTexts> = Rc::new(GridTexts::default());
        }
        let stored = id_of(host).and_then(|id| TEXTS.with(|map| map.borrow().get(&id).cloned()));
        stored.unwrap_or_else(|| DEFAULT.with(Rc::clone))
    }

    /// Reads a JS object into [`GridTexts`], keeping the default of every key it
    /// does not carry.
    ///
    /// A partial object is the normal case: a page that only wants a German
    /// "Clear" should not have to restate the other ten texts. Keys are
    /// camelCase, as JavaScript writes them.
    pub(crate) fn from_js(value: &JsValue) -> GridTexts {
        let mut texts = GridTexts::default();
        let string = |key: &str| -> Option<String> {
            js_sys::Reflect::get(value, &JsValue::from_str(key))
                .ok()
                .and_then(|value| value.as_string())
        };
        let overwrite = |field: &mut String, value: Option<String>| {
            if let Some(value) = value {
                *field = value;
            }
        };
        overwrite(&mut texts.lang, string("lang"));
        overwrite(&mut texts.loading, string("loading"));
        overwrite(&mut texts.matches_one, string("matchesOne"));
        overwrite(&mut texts.matches_other, string("matchesOther"));
        overwrite(&mut texts.empty, string("empty"));
        overwrite(&mut texts.error, string("error"));
        overwrite(&mut texts.error_unknown, string("errorUnknown"));
        overwrite(&mut texts.filter_group, string("filterGroup"));
        overwrite(&mut texts.operator_label, string("operatorLabel"));
        overwrite(&mut texts.value_label, string("valueLabel"));
        overwrite(&mut texts.clear, string("clear"));
        overwrite(&mut texts.filter_invalid, string("filterInvalid"));
        overwrite(&mut texts.no_value, string("noValue"));
        overwrite(&mut texts.empty_value, string("emptyValue"));
        overwrite(&mut texts.total, string("total"));
        overwrite(&mut texts.subtotal, string("subtotal"));

        // `operators` is keyed by the wire token — `{ gte: "greater or equal" }`.
        // A positional array would silently shift every label when one entry is
        // missing or not a string, and it would force a page that wants to
        // translate one operator to restate all eight.
        if let Ok(map) = js_sys::Reflect::get(value, &JsValue::from_str("operators"))
            && map.is_object()
        {
            for (index, token) in crate::grid::FILTER_OPERATORS.iter().enumerate() {
                if let Ok(label) = js_sys::Reflect::get(&map, &JsValue::from_str(token))
                    && let Some(label) = label.as_string()
                    && let Some(slot) = texts.operators.get_mut(index)
                {
                    *slot = label;
                }
            }
        }
        texts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in texts are English — the point of this plan point.
    #[test]
    fn the_defaults_are_english() {
        let texts = GridTexts::default();
        assert_eq!(texts.lang, "en");
        assert_eq!(texts.loading, "Loading …");
        assert_eq!(texts.empty, "No matches");
        assert_eq!(texts.clear, "Clear");
    }

    /// One match is not "1 matches".
    #[test]
    fn the_count_line_has_a_singular() {
        let texts = GridTexts::default();
        assert_eq!(texts.matches(1), "1 match");
        assert_eq!(texts.matches(2), "2 matches");
        assert_eq!(texts.matches(1_234), "1234 matches");
    }

    /// The cause is untranslated, but the sentence around it is not — and an
    /// empty cause never renders a dangling colon.
    #[test]
    fn the_error_carries_its_cause() {
        let texts = GridTexts::default();
        assert_eq!(
            texts.error("unknown source \"orders\""),
            "The data could not be loaded: unknown source \"orders\""
        );
        assert_eq!(texts.error("   "), "The data could not be loaded.");
    }

    /// The refusal names both the column and what was typed — otherwise the
    /// announcement leaves the user guessing which field it means.
    #[test]
    fn an_invalid_filter_names_the_column_and_the_value() {
        let texts = GridTexts::default();
        assert_eq!(
            texts.filter_invalid("qty", "zwei"),
            "qty: zwei is not a value for this column"
        );
    }

    #[test]
    fn labels_name_their_column() {
        let texts = GridTexts::default();
        assert_eq!(texts.operator_label("customer"), "customer operator");
        assert_eq!(texts.value_label("customer"), "customer value");
    }

    /// The user reads a word, the query keeps the token.
    #[test]
    fn operators_read_as_words() {
        let texts = GridTexts::default();
        assert_eq!(texts.operator(0, "contains"), "contains");
        assert_eq!(texts.operator(5, "gte"), "greater or equal");

        // A missing or empty label falls back to the token rather than shifting
        // the others — a label must never name a different operator than the
        // one its `value` sends.
        let sparse = GridTexts {
            operators: vec!["enthält".to_owned(), String::new()],
            ..GridTexts::default()
        };
        assert_eq!(sparse.operator(0, "contains"), "enthält");
        assert_eq!(sparse.operator(1, "starts_with"), "starts_with");
        assert_eq!(sparse.operator(5, "gte"), "gte");
    }

    /// A template without the placeholder is left alone rather than mangled.
    #[test]
    fn a_template_without_a_placeholder_is_kept() {
        let texts = GridTexts {
            matches_other: "Treffer".to_owned(),
            ..GridTexts::default()
        };
        assert_eq!(texts.matches(7), "Treffer");
    }
}
