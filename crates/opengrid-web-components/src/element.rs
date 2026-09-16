//! `<opengrid-table>` — the registered element (point 13, browser only).
//!
//! The lifecycle is the one from [`opengrid_web_core::element`]: on connect it
//! attaches an **open** shadow root, renders the empty table skeleton as one
//! patch list and mirrors the host `label` to the table's `aria-label` (E8/R6).
//! `attributeChangedCallback` keeps the caption and the ARIA label in sync when
//! the host label changes. Point 14 adds columns, data and `aria-sort` here.
//!
//! Nothing in this module decides anything about the data: it renders a shell.
//! The data interface is [`opengrid_web_core::provider`], attached per host and
//! driven by point 14.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlElement, Node, ShadowRoot};

use opengrid_web_core::element::{
    ARIA_LABEL_ATTRIBUTE, LABEL_ATTRIBUTE, attach_open_shadow_root, define, mirror_label,
};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::renderer::{Dom, WebRenderer};

use crate::table::{self, TABLE_TAG};

/// Registers `<opengrid-table>`; safe to call more than once.
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
    )
}

/// Renders the skeleton into a fresh open shadow root.
///
/// Reconnecting a host fires `connectedCallback` again; the guard keeps the
/// already-rendered root instead of appending a second table.
fn on_connected(host: HtmlElement) {
    if let Ok(root) = attach_open_shadow_root(&host)
        && root.child_element_count() == 0
    {
        render_skeleton(&host, &root);
    }
}

/// Nothing to tear down yet: the render tree is stateless in point 13.
fn on_disconnected(_host: HtmlElement) {}

/// Re-mirrors `label` when the host attribute changes.
fn on_attribute_changed(
    host: HtmlElement,
    name: String,
    _old_value: Option<String>,
    new_value: Option<String>,
) {
    if name != LABEL_ATTRIBUTE {
        return;
    }
    if let Some(root) = host.shadow_root() {
        update_label(&root, new_value.as_deref());
    }
}

/// Builds the empty table as one patch list and applies it in one pass.
fn render_skeleton(host: &HtmlElement, root: &ShadowRoot) {
    let Some(document) = host.owner_document() else {
        return;
    };
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    table::build_empty_table(
        &mut buffer,
        &mut nodes,
        host.get_attribute(LABEL_ATTRIBUTE).as_deref(),
    );

    let root_node: Node = root.clone().unchecked_into();
    let mut dom = Dom::new(WebRenderer::from_document(document), root_node);
    dom.apply_buffer(&buffer);
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
