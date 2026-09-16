//! Custom-element lifecycle on an **open** shadow root (E8) and the host
//! `label` → inner `aria-label` mirroring (E8/R6).
//!
//! Geometry: every component lives entirely inside its own shadow root. That is
//! what makes the ARIA-id problem of risk R6 tractable — `aria-labelledby` and
//! `<label for>` do not cross shadow boundaries, so all references stay inside
//! the root and the only thing that crosses is the host attribute `label`, which
//! the component mirrors to `aria-label`
//! (plan/spezifikation/14-entscheidungen.md E8,
//! plan/spezifikation/13-risiken.md R6).
//!
//! The browser glue at the bottom defines the custom element class in a small
//! embedded JavaScript snippet (`wasm-bindgen(inline_js = …)`): a real
//! `class … extends HTMLElement` is required to be a valid custom-element
//! constructor, and none of the Rust state lives there — the class only forwards
//! `connectedCallback`, `disconnectedCallback` and `attributeChangedCallback` to
//! the Rust closures (plan/spezifikation/08-rendering.md §JavaScript-Minimum:
//! loader.js and this glue stay small, the logic is Rust).

/// The host attribute a component is labelled with (E8).
pub const LABEL_ATTRIBUTE: &str = "label";

/// The attribute the host label is mirrored to (E8/R6).
pub const ARIA_LABEL_ATTRIBUTE: &str = "aria-label";

/// Maps a host `label` value to the inner element's `aria-label`.
///
/// Absent or empty label means "no accessible name" — mapping the empty string
/// to `aria-label=""` would be a name that overrides a caption, so it is
/// treated as no label at all.
pub fn mirror_label(label: Option<&str>) -> Option<(&'static str, &str)> {
    label
        .filter(|value| !value.is_empty())
        .map(|value| (ARIA_LABEL_ATTRIBUTE, value))
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use js_sys::{Array, Function};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::*;
    use web_sys::{Element, HtmlElement, ShadowRoot, ShadowRootInit, ShadowRootMode};

    /// Defines a custom element whose lifecycle runs in Rust.
    ///
    /// * `name` — the custom element name, e.g. `opengrid-table`.
    /// * `observed` — the attributes `attributeChangedCallback` reports.
    /// * the three handlers receive the host element (and, for the attribute
    ///   change, the attribute name and its old/new values).
    ///
    /// Calling this twice with the same name is a no-op, so `loader.js` and a
    /// test may both call it defensively.
    pub fn define<C, D, A>(
        name: &str,
        observed: &[&str],
        on_connected: C,
        on_disconnected: D,
        on_attribute_changed: A,
    ) -> Result<(), JsValue>
    where
        C: FnMut(HtmlElement) + 'static,
        D: FnMut(HtmlElement) + 'static,
        A: FnMut(HtmlElement, String, Option<String>, Option<String>) + 'static,
    {
        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let registry = window.custom_elements();
        if !registry.get(name).is_undefined() {
            return Ok(());
        }

        let observed_attributes = Array::new();
        for attribute in observed {
            observed_attributes.push(&JsValue::from_str(attribute));
        }

        let connected: Function = Closure::<dyn FnMut(HtmlElement)>::new(on_connected)
            .into_js_value()
            .unchecked_into();
        let disconnected: Function = Closure::<dyn FnMut(HtmlElement)>::new(on_disconnected)
            .into_js_value()
            .unchecked_into();
        let attribute_changed: Function = Closure::<
            dyn FnMut(HtmlElement, String, Option<String>, Option<String>),
        >::new(on_attribute_changed)
        .into_js_value()
        .unchecked_into();

        define_element_js(
            name,
            &observed_attributes,
            &connected,
            &disconnected,
            &attribute_changed,
        );
        Ok(())
    }

    /// Attaches an **open** shadow root if the host has none and answers it
    /// (E8; open so `::part` and the light-DOM test can reach in).
    pub fn attach_open_shadow_root(host: &HtmlElement) -> Result<ShadowRoot, JsValue> {
        if let Some(root) = host.shadow_root() {
            return Ok(root);
        }
        host.attach_shadow(&ShadowRootInit::new(ShadowRootMode::Open))
    }

    /// Looks up an element by id **inside** the shadow root.
    ///
    /// All ARIA id references go through here; an id in the light DOM is
    /// invisible to assistive technology (R6).
    pub fn element_in_root(root: &ShadowRoot, id: &str) -> Option<Element> {
        root.get_element_by_id(id)
    }

    #[wasm_bindgen(inline_js = "
const OPENGRID_LIFECYCLE = Symbol(\"opengrid.lifecycle\");

export function __opengrid_define_element(name, observed, connected, disconnected, attributeChanged) {
    const hooks = { connected, disconnected, attributeChanged };
    class OpengridElement extends HTMLElement {
        constructor() {
            super();
            this[OPENGRID_LIFECYCLE] = hooks;
        }
        connectedCallback() {
            hooks.connected(this);
        }
        disconnectedCallback() {
            hooks.disconnected(this);
        }
        attributeChangedCallback(attribute, oldValue, newValue) {
            hooks.attributeChanged(this, attribute, oldValue, newValue);
        }
    }
    if (observed.length > 0) {
        Object.defineProperty(OpengridElement, \"observedAttributes\", { value: observed });
    }
    customElements.define(name, OpengridElement);
}
")]
    extern "C" {
        #[wasm_bindgen(js_name = __opengrid_define_element)]
        fn define_element_js(
            name: &str,
            observed: &Array,
            connected: &Function,
            disconnected: &Function,
            attribute_changed: &Function,
        );
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{attach_open_shadow_root, define, element_in_root};

#[cfg(test)]
mod tests {
    use super::*;

    /// The host label becomes the inner `aria-label` (E8/R6).
    #[test]
    fn label_maps_to_aria_label() {
        assert_eq!(
            mirror_label(Some("Bestellungen")),
            Some(("aria-label", "Bestellungen"))
        );
    }

    /// No label and an empty label both mean "no accessible name".
    #[test]
    fn missing_or_empty_label_is_no_aria_label() {
        assert_eq!(mirror_label(None), None);
        assert_eq!(mirror_label(Some("")), None);
    }
}
