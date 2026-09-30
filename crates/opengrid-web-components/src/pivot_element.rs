//! `<opengrid-pivot>` — the browser glue (plan point 32).
//!
//! The same lifecycle as the table: an open shadow root, a whole render per
//! answer, and the engine never named — only the provider's JSON-in/JSON-out
//! Promise. What differs is what travels: a pivot request, and the pivot wire
//! form of point 53 coming back.
//!
//! There is no interaction to preserve here, so a failure simply becomes the
//! status line's text. That is the point of Table Mode: nothing to lose.
//!
//! The answer the element shows is kept beside it, so that `get_pivot` exports
//! exactly that (issue #3).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::JsError;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::HtmlElement;

use opengrid_web_core::element::{LABEL_ATTRIBUTE, attach_open_shadow_root, define};
use opengrid_web_core::host::{existing_id, id as host_id, on_release};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::element::{apply, clear_root, describe, is_latest, next_request};
use crate::pivot::{
    self, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, PIVOT_TAG, PivotLook, PivotModel, PlainLook,
    ROWS_ATTRIBUTE, VALUES_ATTRIBUTE,
};
use crate::texts;

/// Registers `<opengrid-pivot>`.
pub fn define_pivot() -> Result<(), JsValue> {
    define(
        PIVOT_TAG,
        pivot::OBSERVED,
        on_connected,
        |_host| {},
        on_attribute_changed,
    )
}

fn on_connected(host: HtmlElement) {
    if let Ok(root) = attach_open_shadow_root(&host)
        && root.child_element_count() == 0
    {
        crate::theme::adopt_table_look(&root);
        render(&host, None, "", "loading", &PlainLook);
    }
    run(&host);
}

fn on_attribute_changed(
    host: HtmlElement,
    name: String,
    _old: Option<String>,
    _new: Option<String>,
) {
    match name.as_str() {
        LABEL_ATTRIBUTE | DATASOURCE_ATTRIBUTE | ROWS_ATTRIBUTE | COLUMNS_ATTRIBUTE
        | VALUES_ATTRIBUTE => run(&host),
        _ => {}
    }
}

/// Builds the request from the host attributes and runs it.
///
/// Missing attributes are "not ready yet", not errors — except the measures: an
/// empty or unreadable `values` is a mistake the page can fix, so it is shown.
pub(crate) fn run(host: &HtmlElement) {
    let Some(provider) = provider(host) else {
        return;
    };
    if host.shadow_root().is_none() {
        return;
    }
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let rows = pivot::parse_dimensions(host.get_attribute(ROWS_ATTRIBUTE).as_deref());
    let columns = pivot::parse_dimensions(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());

    let request_json = match pivot::pivot_json(
        &source,
        &rows,
        &columns,
        host.get_attribute(VALUES_ATTRIBUTE).as_deref(),
    ) {
        Ok(request_json) => request_json,
        Err(message) => {
            // A request that cannot be built supersedes an earlier one as
            // well: its error is what the element shows, not an answer that
            // arrives after it.
            next_request(host);
            let texts = texts::texts(host);
            render(host, None, &texts.error(&message), "error", &PlainLook);
            return;
        }
    };

    let request = next_request(host);
    let texts = texts::texts(host);
    render(host, None, &texts.loading, "loading", &PlainLook);

    let promise = provider.pivot(&request_json, "");
    let host = host.clone();
    spawn_local(async move {
        let outcome = JsFuture::from(promise).await;
        // A newer request was made since (an attribute changed again): its
        // answer is the one to draw, whenever it comes.
        if !is_latest(&host, request) {
            return;
        }
        let texts = texts::texts(&host);
        match outcome {
            Ok(value) => match pivot_json(&value) {
                Ok(json) => match pivot::parse_result(&json) {
                    Ok(mut model) => {
                        model.column_dimensions = columns;
                        // The grand total is always a row, so "no matches" means
                        // nothing but the total came back.
                        let (status, state) = if model.rows.len() <= 1 {
                            (texts.empty.clone(), "empty")
                        } else {
                            (texts.matches(model.rows.len() as u64), "ready")
                        };
                        match HostLook::new(&host) {
                            Ok(look) => render(&host, Some((&model, &json)), &status, state, &look),
                            Err(message) => {
                                render(&host, None, &texts.error(&message), "error", &PlainLook)
                            }
                        }
                    }
                    Err(message) => {
                        render(&host, None, &texts.error(&message), "error", &PlainLook)
                    }
                },
                Err(message) => render(&host, None, &texts.error(&message), "error", &PlainLook),
            },
            Err(value) => render(
                &host,
                None,
                &texts.error(&describe(&value)),
                "error",
                &PlainLook,
            ),
        }
    });
}

/// A provider's pivot answer as the pivot wire form, whichever form it came in.
///
/// The binary form (E35) is read strictly and written back as JSON: the pivot
/// renders and exports from that one form, so the headings read the same
/// whichever way the answer travelled. A pivot's size is bounded by its column
/// limit, so the detour is small.
fn pivot_json(value: &JsValue) -> Result<String, String> {
    match crate::element::answer(value) {
        Some(crate::element::Answer::Json(json)) => Ok(json),
        Some(crate::element::Answer::Binary(bytes)) => {
            let (result, rows) =
                opengrid_pivot::pivot_from_bytes(&bytes).map_err(|error| error.to_string())?;
            Ok(opengrid_pivot::pivot_to_json(&result, &rows))
        }
        None => Err(crate::element::NOT_AN_ANSWER.to_owned()),
    }
}

/// Clears the root and renders the whole pivot as one patch list: the model,
/// and the answer it was read from, which is what `get_pivot` exports.
fn render(
    host: &HtmlElement,
    shown: Option<(&PivotModel, &str)>,
    status: &str,
    state: &str,
    look: &dyn PivotLook,
) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    clear_root(&root);
    // Kept and dropped with what is drawn, never apart from it: while a new
    // answer loads, or after an error, the table is empty and so is the export.
    remember(host, shown.map(|(_, answer)| answer));
    let model = shown.map(|(model, _)| model);

    let label = host.get_attribute(LABEL_ATTRIBUTE);
    let texts = texts::texts(host);
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    pivot::build_pivot(
        &mut buffer,
        &mut nodes,
        label.as_deref(),
        model,
        status,
        state,
        &texts,
        look,
    );
    apply(&root, document, &buffer);
}

/// The page's look for a pivot (issue #104): its `set_formats`, by field or
/// measure name, and — with the grid's `set_columns` — its titles.
struct HostLook {
    formats: Rc<crate::formats::ColumnFormats>,
    #[cfg(feature = "grid")]
    titles: Vec<(String, String)>,
}

impl HostLook {
    /// A pivot takes a `title` and nothing else: an option that silently did
    /// nothing would hide the mistake, as it would at the table. A title for a
    /// name the pivot does not show now is fine — its dimensions change with
    /// its attributes.
    fn new(host: &HtmlElement) -> Result<Self, String> {
        #[cfg(feature = "grid")]
        let titles = {
            let raw = crate::presentation::raw(host);
            let mut problems = Vec::new();
            for (name, entry) in raw.iter() {
                for (given, option) in [
                    (entry.width.is_some(), "width"),
                    (entry.align.is_some(), "align"),
                    (entry.mono.is_some(), "mono"),
                    (entry.emphasis.is_some(), "emphasis"),
                    (entry.muted.is_some(), "muted"),
                    (entry.aggregate.is_some(), "aggregate"),
                    (entry.facet.is_some(), "facet"),
                ] {
                    if given {
                        problems.push(format!("{name}: a pivot takes a title, not {option}"));
                    }
                }
            }
            if !problems.is_empty() {
                return Err(problems.join(" "));
            }
            raw.iter()
                .filter_map(|(name, entry)| entry.title.clone().map(|title| (name.clone(), title)))
                .collect()
        };
        Ok(Self {
            formats: crate::formats::formats(host),
            #[cfg(feature = "grid")]
            titles,
        })
    }
}

impl PivotLook for HostLook {
    fn title(&self, name: &str) -> String {
        #[cfg(feature = "grid")]
        if let Some((_, title)) = self.titles.iter().find(|(field, _)| field == name) {
            return title.clone();
        }
        name.to_owned()
    }

    fn text(&self, name: &str, value: &opengrid_json::Json) -> String {
        let plain = pivot::plain(value).unwrap_or_default();
        let Some(function) = self.formats.function(name) else {
            // No format for this name: no boundary crossing (R1).
            return plain;
        };
        let json = js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL);
        function
            .call2(&JsValue::NULL, &JsValue::from_str(&plain), &json)
            .ok()
            .and_then(|text| text.as_string())
            // A formatter that throws or answers nothing must not blank the
            // cell: the value is still there.
            .unwrap_or(plain)
    }
}

thread_local! {
    /// The answer each pivot shows, by host id: the pivot wire form, as the
    /// provider sent it.
    static SHOWN: RefCell<HashMap<u32, Rc<str>>> = RefCell::new(HashMap::new());
}

fn forget(id: u32) {
    SHOWN.with(|map| map.borrow_mut().remove(&id));
}

/// Keeps `answer` as what `host` shows, or forgets what it showed.
fn remember(host: &HtmlElement, answer: Option<&str>) {
    match answer {
        Some(answer) => {
            on_release(forget);
            let id = host_id(host);
            SHOWN.with(|map| map.borrow_mut().insert(id, Rc::from(answer)));
        }
        None => {
            if let Some(id) = existing_id(host) {
                forget(id);
            }
        }
    }
}

/// [`crate::element::get_pivot`] — the shown pivot as CSV, or `null`.
///
/// The options are read first, so a wrong one is an error whether or not a
/// pivot is shown yet.
pub(crate) fn read_pivot(host: &HtmlElement, options: &JsValue) -> Result<JsValue, JsError> {
    let options = crate::export::pivot_options(options)?;
    let Some(answer) =
        existing_id(host).and_then(|id| SHOWN.with(|map| map.borrow().get(&id).cloned()))
    else {
        return Ok(JsValue::NULL);
    };
    let csv = crate::export::pivot_csv(&answer, &texts::texts(host), &options)?;
    Ok(JsValue::from_str(&csv))
}
