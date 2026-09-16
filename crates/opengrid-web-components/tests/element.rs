#![cfg(target_arch = "wasm32")]
//! `<opengrid-table>` in a real browser (point 13).
//!
//! Two properties the plan makes testable:
//!
//! * a registered element connects and gets an **open** shadow root
//!   (E8, plan/spezifikation/14-entscheidungen.md),
//! * a host `label` appears as the inner table's `aria-label`
//!   (E8/R6, plan/spezifikation/13-risiken.md).
//!
//! Run with `CHROMEDRIVER=chromedriver cargo test --target
//! wasm32-unknown-unknown -p opengrid-web-components --test element` (headless
//! Chrome). On the host the file compiles to nothing, so `just check` stays
//! green.

use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlElement, ShadowRoot, ShadowRootMode};

wasm_bindgen_test_configure!(run_in_browser);

/// The test document.
fn document() -> web_sys::Document {
    web_sys::window()
        .expect("a window")
        .document()
        .expect("a document")
}

/// Registers the element, creates a host (optionally labelled) and connects it.
fn mount(label: Option<&str>) -> HtmlElement {
    opengrid_web_components::register().expect("opengrid-table registers");
    let element = document().create_element("opengrid-table").expect("create");
    if let Some(label) = label {
        element.set_attribute("label", label).expect("set label");
    }
    let host: HtmlElement = element.unchecked_into();
    document()
        .body()
        .expect("a body")
        .append_child(&host)
        .expect("connect");
    host
}

/// The open shadow root of a connected host.
fn shadow(host: &HtmlElement) -> ShadowRoot {
    host.shadow_root().expect("an open shadow root")
}

/// (a) A registered element connects and renders into an open shadow root.
#[wasm_bindgen_test]
fn element_connects_with_an_open_shadow_root() {
    let host = mount(None);
    let root = shadow(&host);
    assert_eq!(root.mode(), ShadowRootMode::Open);
    assert!(
        root.query_selector("table").expect("query").is_some(),
        "the skeleton table is rendered inside the shadow root"
    );
    host.remove();
}

/// (b) A host `label` yields `aria-label` on the inner table (E8/R6).
#[wasm_bindgen_test]
fn host_label_becomes_the_aria_label_of_the_table() {
    let host = mount(Some("X"));
    let root = shadow(&host);

    let table = root.query_selector("table").expect("query").expect("table");
    assert_eq!(table.get_attribute("aria-label").as_deref(), Some("X"));

    let caption = root
        .query_selector("caption")
        .expect("query")
        .expect("caption");
    assert_eq!(caption.text_content().as_deref(), Some("X"));

    host.remove();
}
