//! `<opengrid-table>` — the registered element (points 13/14, browser only).
//!
//! The lifecycle is the one from [`opengrid_web_core::element`]: on connect it
//! attaches an **open** shadow root, renders the empty table skeleton as one
//! patch list and mirrors the host `label` to the table's `aria-label` (E8/R6).
//! `attributeChangedCallback` keeps the caption and the ARIA label in sync when
//! the host label changes.
//!
//! Point 14 adds the data path. The host attributes `datasource` and `columns`
//! build the query (plan/spezifikation/02-query-modell.md §JSON-Vertrag), a
//! provider attached with the exported [`set_provider`] executes it, and the
//! result is rendered as one patch list. Header `<button>`s sort the single
//! column asc → desc → none and re-run the query; `aria-sort` lives on the
//! `<th>` (plan/spezifikation/09-accessibility.md §Zwei Rendering-Modi).
//!
//! The engine itself is never named here: only
//! [`opengrid_web_core::provider`]'s JSON-in/JSON-out Promise is.

use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Document, Element, Event, HtmlElement, Node, ShadowRoot};

use opengrid_web_core::element::{
    ARIA_LABEL_ATTRIBUTE, LABEL_ATTRIBUTE, attach_open_shadow_root, define, mirror_label,
};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::set_provider as attach_provider;
use opengrid_web_core::provider::{DataProvider, JsProvider, provider};
use opengrid_web_core::renderer::{Dom, WebRenderer};

use crate::table::{
    self, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, SortDirection, TABLE_TAG, TableModel,
};
use crate::texts;

/// Registers `<opengrid-table>` and `<opengrid-grid>`; safe to call more than
/// once.
///
/// `loader.js` calls this after the WASM module is initialised; the browser test
/// calls it directly.
#[wasm_bindgen]
pub fn register() -> Result<(), JsValue> {
    define(
        TABLE_TAG,
        table::OBSERVED,
        on_connected,
        on_disconnected,
        on_attribute_changed,
    )?;
    crate::grid_element::define_grid()
}

/// Attaches a data provider to a host element (points 14/16).
///
/// `provider` is a JS object with an `execute(queryJson)` method that answers a
/// Promise (or a value). Exported as `set_provider` from the components module,
/// so the page wires its engine before or after connect; a connected host whose
/// `datasource`/`columns` are present re-runs its query immediately. The tag
/// name decides whether the table or the grid path runs.
#[wasm_bindgen(js_name = set_provider)]
pub fn set_provider(host: &HtmlElement, provider: JsValue) {
    let provider: Rc<dyn DataProvider> = Rc::new(JsProvider::new(provider));
    attach_provider(host, provider);
    if host.tag_name().eq_ignore_ascii_case("opengrid-grid") {
        crate::grid_element::start(host);
    } else {
        run_query(host, None, None);
    }
}

/// Overrides the texts a component writes itself (point 48).
///
/// `texts` is a plain JS object with any subset of the keys `lang`, `loading`,
/// `matchesOne`, `matchesOther`, `empty`, `error`, `errorUnknown`,
/// `filterGroup`, `operatorLabel`, `valueLabel`, `clear` and `operators`; every
/// key left out keeps its English default. `{count}`, `{column}` and `{cause}`
/// are the placeholders. A `lang` is written onto the elements that carry these
/// texts — the grid's filter row and status line, the table's error paragraph —
/// so they are announced in the language they are written in. Never onto the
/// data: the cells are the page's, in the page's language.
///
/// Exported as `set_texts` beside `set_provider`, and like it, it takes effect
/// immediately: the grid rebuilds, because its labels are part of the one-time
/// skeleton (the filter row, the scroll offset and the focus are carried
/// across). Call it **before** wiring the provider and the component renders the
/// right words from its first paint.
#[wasm_bindgen(js_name = set_texts)]
pub fn set_texts(host: &HtmlElement, values: JsValue) {
    texts::store(host, Rc::new(texts::from_js(&values)));
    if host.shadow_root().is_none() {
        // Not connected yet — `connectedCallback` will read the stored texts.
        return;
    }
    if host.tag_name().eq_ignore_ascii_case("opengrid-grid") {
        crate::grid_element::retext(host);
    } else {
        run_query(host, None, None);
    }
}

/// Renders the skeleton into a fresh open shadow root and installs the sort
/// listener once.
///
/// Reconnecting a host fires `connectedCallback` again; the guard keeps the
/// already-rendered root instead of appending a second table or listener.
fn on_connected(host: HtmlElement) {
    if let Ok(root) = attach_open_shadow_root(&host)
        && root.child_element_count() == 0
    {
        render_skeleton(&host, &root);
        add_sort_listener(&root);
    }
    run_query(&host, None, None);
}

/// Nothing to tear down: the root and its listener die with the host.
fn on_disconnected(_host: HtmlElement) {}

/// Re-mirrors `label`, re-runs the query when the data attributes change.
fn on_attribute_changed(
    host: HtmlElement,
    name: String,
    _old_value: Option<String>,
    new_value: Option<String>,
) {
    match name.as_str() {
        LABEL_ATTRIBUTE => {
            if let Some(root) = host.shadow_root() {
                update_label(&root, new_value.as_deref());
            }
        }
        DATASOURCE_ATTRIBUTE | COLUMNS_ATTRIBUTE => run_query(&host, None, None),
        _ => {}
    }
}

/// Builds the empty table as one patch list and applies it in one pass.
fn render_skeleton(host: &HtmlElement, root: &ShadowRoot) {
    let Some(document) = host.owner_document() else {
        return;
    };
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    table::build_table(
        &mut buffer,
        &mut nodes,
        host.get_attribute(LABEL_ATTRIBUTE).as_deref(),
        None,
        None,
    );
    apply(root, document, &buffer);
}

/// Builds the query from the host attributes and runs it through the provider.
///
/// Returns without doing anything if there is no provider, no shadow root, or no
/// columns — those are the "not ready yet" states, not errors. `sort` is the
/// explicit single-column sort; `focus_column` names the header button to focus
/// once the re-render landed, so keyboard sorting does not lose focus.
fn run_query(
    host: &HtmlElement,
    sort: Option<(String, SortDirection)>,
    focus_column: Option<String>,
) {
    let Some(provider) = provider(host) else {
        return;
    };
    if host.shadow_root().is_none() {
        return;
    }
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let columns = table::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    if columns.is_empty() {
        return;
    }
    let query = table::query_json(
        &source,
        &columns,
        sort.as_ref()
            .map(|(field, direction)| (field.as_str(), *direction)),
    );
    // The table has no `mode`: it is the simple element, and point 28's split
    // belongs to the grid.
    let promise = provider.execute(&query, "");

    let host = host.clone();
    spawn_local(async move {
        match JsFuture::from(promise).await {
            Ok(value) => match value.as_string() {
                Some(json) => match table::parse_result(&json) {
                    Ok(model) => {
                        render_data(&host, &model, sort.as_ref(), focus_column.as_deref());
                    }
                    Err(message) => render_error(&host, &message),
                },
                None => render_error(&host, "provider returned a non-string result"),
            },
            Err(value) => render_error(&host, &describe(&value)),
        }
    });
}

/// Clears the root and renders `model` as one patch list.
fn render_data(
    host: &HtmlElement,
    model: &TableModel,
    sort: Option<&(String, SortDirection)>,
    focus_column: Option<&str>,
) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };

    clear_root(&root);
    let label = host.get_attribute(LABEL_ATTRIBUTE);
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    table::build_table(
        &mut buffer,
        &mut nodes,
        label.as_deref(),
        Some(model),
        sort.map(|(field, direction)| (field.as_str(), *direction)),
    );
    apply(&root, document, &buffer);

    if let Some(column) = focus_column {
        focus_header(&root, column);
    }
}

/// Clears the root and renders a short visible error.
///
/// Table mode has no status area: it renders a result, not an interactive grid,
/// so a failure replaces it with an alert. The grid takes the other path — its
/// status line (point 41) reports the failure without destroying the table the
/// user is navigating. `cause` is the untranslated diagnosis; the sentence
/// around it comes from the component's texts (point 48).
fn render_error(host: &HtmlElement, cause: &str) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };

    clear_root(&root);
    let texts = texts::texts(host);
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    table::build_error(&mut buffer, &mut nodes, &texts.error(cause), &texts.lang);
    apply(&root, document, &buffer);
}

/// Applies a whole patch list to the shadow root in one pass (risk R1).
pub(crate) fn apply(root: &ShadowRoot, document: Document, buffer: &PatchBuffer) {
    let root_node: Node = root.clone().unchecked_into();
    let mut dom = Dom::new(WebRenderer::from_document(document), root_node);
    dom.apply_buffer(buffer);
}

/// Removes every child of the root before a full re-render.
///
/// One direct call, not one per cell: the patch language has no "clear"
/// operation, and rebuilding the whole table is the smallest correct response to
/// a sort. The data render itself is still exactly one patch list.
pub(crate) fn clear_root(root: &ShadowRoot) {
    while let Some(child) = root.first_child() {
        let _ = root.remove_child(&child);
    }
}

/// Delegated `click` listener: a header button toggles its single column.
///
/// Keyboard activation (Enter/Space) fires a `click` too, so one listener covers
/// both. The closure is owned by the root via the registered callback.
fn add_sort_listener(root: &ShadowRoot) {
    let callback = Closure::<dyn FnMut(Event)>::new(on_header_click).into_js_value();
    let _ = root.add_event_listener_with_callback("click", callback.unchecked_ref());
}

/// Handles a click on a header button.
fn on_header_click(event: Event) {
    let Some(target) = event.target() else {
        return;
    };
    let Ok(target) = target.dyn_into::<Element>() else {
        return;
    };
    let Ok(Some(button)) = target.closest("button[data-column]") else {
        return;
    };
    let Some(column) = button.get_attribute("data-column") else {
        return;
    };
    let Some(root) = event
        .current_target()
        .and_then(|target| target.dyn_into::<ShadowRoot>().ok())
    else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };

    let next = next_sort(&root, &column);
    run_query(&host, next, Some(column));
}

/// The sort state after activating `column`: asc → desc → none, single-column.
fn next_sort(root: &ShadowRoot, column: &str) -> Option<(String, SortDirection)> {
    match current_sort(root) {
        Some((current, SortDirection::Asc)) if current == column => {
            Some((column.to_owned(), SortDirection::Desc))
        }
        Some((current, SortDirection::Desc)) if current == column => None,
        _ => Some((column.to_owned(), SortDirection::Asc)),
    }
}

/// The column currently sorted, read back from the rendered `aria-sort`.
fn current_sort(root: &ShadowRoot) -> Option<(String, SortDirection)> {
    let (selector, direction) = if root
        .query_selector("th[aria-sort=\"ascending\"]")
        .ok()
        .flatten()
        .is_some()
    {
        ("th[aria-sort=\"ascending\"]", SortDirection::Asc)
    } else {
        ("th[aria-sort=\"descending\"]", SortDirection::Desc)
    };
    let th = root.query_selector(selector).ok().flatten()?;
    Some((th.get_attribute("data-column")?, direction))
}

/// Focuses the header button of `column`, if it is still rendered.
fn focus_header(root: &ShadowRoot, column: &str) {
    let selector = format!("th[data-column=\"{column}\"] button");
    if let Ok(Some(button)) = root.query_selector(&selector)
        && let Ok(button) = button.dyn_into::<HtmlElement>()
    {
        let _ = button.focus();
    }
}

/// The message of a rejected provider promise.
///
/// Both providers reject with an `Error`, not a string: the Worker rebuilds one
/// from the worker message and the local engine throws a `JsError`, which is how
/// the engine's `DataSourceError` text reaches the component at all. Reading
/// `.message` is therefore the normal path — without it every backend failure
/// collapsed into the generic fallback, and point 41 needs the cause. A rejected
/// value that is neither a string nor carries a usable `message` falls back to a
/// sentence rather than to `[object Object]`.
pub(crate) fn describe(value: &JsValue) -> String {
    if let Some(message) = value.as_string() {
        return message;
    }
    let message = js_sys::Reflect::get(value, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .filter(|message| !message.trim().is_empty());
    message.unwrap_or_else(|| "the provider rejected the query".to_owned())
}

/// Updates the existing caption and `aria-label` after a label change.
///
/// A label change is not a render cycle: it touches the two affected nodes
/// directly instead of rebuilding the table.
fn update_label(root: &ShadowRoot, label: Option<&str>) {
    let Ok(Some(table)) = root.query_selector("table") else {
        return;
    };
    match mirror_label(label) {
        Some((name, value)) => {
            let _ = table.set_attribute(name, value);
        }
        None => {
            let _ = table.remove_attribute(ARIA_LABEL_ATTRIBUTE);
        }
    }
    if let Ok(Some(caption)) = table.query_selector("caption") {
        caption.set_text_content(Some(label.unwrap_or_default()));
    }
}
