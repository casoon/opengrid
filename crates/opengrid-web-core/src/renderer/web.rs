//! The `web-sys` renderer (browser only).
//!
//! A thin glue layer: it turns the abstract [`Patch`](crate::patch::Patch)
//! operations into real DOM calls but makes no decisions of its own
//! (plan/spezifikation/08-rendering.md §DOM-Brücke).

use wasm_bindgen::prelude::wasm_bindgen;
use web_sys::{Document, Node};

use super::Renderer;

/// A [`Renderer`] over a real document.
///
/// The node type is [`web_sys::Node`] so the render root can be a
/// `ShadowRoot`/`DocumentFragment` as well as an `Element`; attribute and text
/// operations narrow to `Element` where the DOM requires it.
pub struct WebRenderer {
    document: Document,
}

impl WebRenderer {
    /// A renderer for the current document.
    pub fn new() -> Result<Self, wasm_bindgen::JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("no document"))?;
        Ok(Self::from_document(document))
    }

    /// A renderer for a given document (e.g. an element's owner document).
    pub fn from_document(document: Document) -> Self {
        Self { document }
    }

    /// The document elements are created in.
    pub fn document(&self) -> &Document {
        &self.document
    }
}

#[wasm_bindgen(inline_js = "
// Writes nothing when the node already shows exactly this text: one Text node
// with these characters, or no child for the empty string. A recycled cell
// that shows the same value again, and the status line on every scroll frame,
// then neither lose their Text node nor queue a mutation (a live region may
// announce a replaced node again). Whatever else the node holds, such as the
// chips drawn over a cell, is not equal and is replaced as before.
export function __opengrid_set_text(node, text) {
    const first = node.firstChild;
    if (first === null ? text === \"\" : first.nodeType === 3 && first.nextSibling === null && first.data === text) {
        return;
    }
    node.textContent = text;
}

// The same for attributes: a recycled slot is patched with the whole set of
// attributes a row has, nearly all of them already as they should be, and most
// of the removals name an attribute that is not there. Each is one comparison
// against the element instead of a write the browser has to record.
// `style` goes through the CSSOM (`style.cssText`), not `setAttribute`: a page
// whose policy has `style-src` without 'unsafe-inline' refuses a style
// attribute set from script, and the grid positions its rows with one. The
// CSSOM is not governed by `style-src`; the attribute reads back the same
// either way.
export function __opengrid_set_attribute(node, name, value) {
    if (name === \"style\" && node instanceof HTMLElement) {
        if (node.style.cssText !== value) {
            node.style.cssText = value;
        }
        return;
    }
    if (node.nodeType === 1 && node.getAttribute(name) !== value) {
        node.setAttribute(name, value);
    }
}

export function __opengrid_remove_attribute(node, name) {
    if (node.nodeType === 1 && node.hasAttribute(name)) {
        node.removeAttribute(name);
    }
}
")]
extern "C" {
    #[wasm_bindgen(js_name = __opengrid_set_text)]
    fn set_text_if_changed(node: &Node, text: &str);

    #[wasm_bindgen(js_name = __opengrid_set_attribute)]
    fn set_attribute_if_changed(node: &Node, name: &str, value: &str);

    #[wasm_bindgen(js_name = __opengrid_remove_attribute)]
    fn remove_attribute_if_present(node: &Node, name: &str);
}

impl Renderer for WebRenderer {
    type Node = Node;

    fn create_element(&mut self, tag: &str) -> Node {
        self.document
            .create_element(tag)
            .expect("renderer tags are valid element names")
            .into()
    }

    /// Written only where it differs from the element, and `style` through the
    /// CSSOM (see the helper's comment above).
    fn set_attribute(&mut self, node: &Node, name: &str, value: &str) {
        set_attribute_if_changed(node, name, value);
    }

    fn remove_attribute(&mut self, node: &Node, name: &str) {
        remove_attribute_if_present(node, name);
    }

    fn set_text(&mut self, node: &Node, text: &str) {
        set_text_if_changed(node, text);
    }

    fn append_child(&mut self, parent: &Node, child: &Node) {
        parent
            .append_child(child)
            .expect("renderer appends children to node parents");
    }
}
