//! The `web-sys` renderer (browser only).
//!
//! A thin glue layer: it turns the abstract [`Patch`](crate::patch::Patch)
//! operations into real DOM calls but makes no decisions of its own
//! (plan/spezifikation/08-rendering.md §DOM-Brücke).

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, Node};

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

impl Renderer for WebRenderer {
    type Node = Node;

    fn create_element(&mut self, tag: &str) -> Node {
        self.document
            .create_element(tag)
            .expect("renderer tags are valid element names")
            .into()
    }

    /// `style` goes through the CSSOM (`element.style.cssText`), not
    /// `setAttribute`: a page whose policy has `style-src` without
    /// 'unsafe-inline' refuses a style attribute set from script, and the grid
    /// positions its rows with one. The CSSOM is not governed by `style-src`;
    /// the attribute reads back the same either way.
    fn set_attribute(&mut self, node: &Node, name: &str, value: &str) {
        if name == "style"
            && let Some(element) = node.dyn_ref::<HtmlElement>()
        {
            element.style().set_css_text(value);
            return;
        }
        if let Some(element) = node.dyn_ref::<Element>() {
            element
                .set_attribute(name, value)
                .expect("renderer attribute names are valid");
        }
    }

    fn remove_attribute(&mut self, node: &Node, name: &str) {
        if let Some(element) = node.dyn_ref::<Element>() {
            element
                .remove_attribute(name)
                .expect("renderer attribute names are valid");
        }
    }

    fn set_text(&mut self, node: &Node, text: &str) {
        node.set_text_content(Some(text));
    }

    fn append_child(&mut self, parent: &Node, child: &Node) {
        parent
            .append_child(child)
            .expect("renderer appends children to node parents");
    }
}
