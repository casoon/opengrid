//! `<opengrid-pivot>` — the browser glue (plan point 32).
//!
//! The same lifecycle as the table: an open shadow root, a whole render per
//! answer, and the engine never named — only the provider's JSON-in/JSON-out
//! Promise. What differs is what travels: a pivot request, and the pivot wire
//! form of point 53 coming back.
//!
//! There is no interaction to preserve here, so a failure simply becomes the
//! status line's text. That is the point of Table Mode: nothing to lose.

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::HtmlElement;

use opengrid_web_core::element::{LABEL_ATTRIBUTE, attach_open_shadow_root, define};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::element::{apply, clear_root, describe};
use crate::pivot::{
    self, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, PIVOT_TAG, PivotModel, ROWS_ATTRIBUTE,
    VALUES_ATTRIBUTE,
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
        render(&host, None, "", "loading");
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

    let request = match pivot::pivot_json(
        &source,
        &rows,
        &columns,
        host.get_attribute(VALUES_ATTRIBUTE).as_deref(),
    ) {
        Ok(request) => request,
        Err(message) => {
            let texts = texts::texts(host);
            render(host, None, &texts.error(&message), "error");
            return;
        }
    };

    let texts = texts::texts(host);
    render(host, None, &texts.loading, "loading");

    let promise = provider.execute(&request, "");
    let host = host.clone();
    spawn_local(async move {
        let texts = texts::texts(&host);
        match JsFuture::from(promise).await {
            Ok(value) => match value.as_string() {
                Some(json) => match pivot::parse_result(&json) {
                    Ok(model) => {
                        // The grand total is always a row, so "no matches" means
                        // nothing but the total came back.
                        let (status, state) = if model.rows.len() <= 1 {
                            (texts.empty.clone(), "empty")
                        } else {
                            (texts.matches(model.rows.len() as u64), "ready")
                        };
                        render(&host, Some(&model), &status, state);
                    }
                    Err(message) => render(&host, None, &texts.error(&message), "error"),
                },
                None => render(
                    &host,
                    None,
                    &texts.error("provider returned a non-string result"),
                    "error",
                ),
            },
            Err(value) => render(&host, None, &texts.error(&describe(&value)), "error"),
        }
    });
}

/// Clears the root and renders the whole pivot as one patch list.
fn render(host: &HtmlElement, model: Option<&PivotModel>, status: &str, state: &str) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    clear_root(&root);

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
    );
    apply(&root, document, &buffer);
}
