//! One id per host element, and the moment its state may go (point 74).
//!
//! Everything a component keeps per element — the provider, the grid's
//! runtime, the column layout, the texts, the formats — lives in Rust tables
//! keyed by this id, because a `web_sys::HtmlElement` has no stable Rust
//! identity across callbacks. The id is kept in a `WeakMap` of this module
//! instance, keyed by the host.
//!
//! **One id, not one per table.** Each table used to hand out its own, and two
//! of them shared a symbol with two counters: a grid configured before it was
//! connected and a later grid without could be given the same number — and
//! one column layout between them.
//!
//! **Released when the host is collected, not when it is disconnected.** A
//! keyed list moves an element with one insert (disconnect, connect, same
//! task); a cache like Vue's `<KeepAlive>` takes it out and puts it back much
//! later. Both expect the same element back. Only the garbage collector knows
//! that an element will never return, so a `FinalizationRegistry` reports it,
//! and every table registered with [`on_release`] drops the id.
//!
//! That only works if no table holds the host — or anything in its shadow
//! tree — past the moment it leaves the document. A reference from WASM is a
//! root the collector cannot see through, so the host would stay reachable for
//! ever and the registry would never fire. The grid therefore lets go of its
//! DOM when it is disconnected for longer than a task (`grid_element.rs`).

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::{Cell, RefCell};

    use wasm_bindgen::JsValue;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::*;
    use web_sys::HtmlElement;

    thread_local! {
        /// Every table's way of forgetting an id.
        static RELEASES: RefCell<Vec<fn(u32)>> = const { RefCell::new(Vec::new()) };
        /// Whether the registry has its callback yet.
        static WATCHING: Cell<bool> = const { Cell::new(false) };
    }

    /// The id of `host`, if it has one yet.
    pub fn existing_id(host: &HtmlElement) -> Option<u32> {
        match known_id(host) {
            0 => None,
            id => Some(id),
        }
    }

    /// The id of `host`, given on first use; from then on the host is watched,
    /// and its id released once it is collected.
    pub fn id(host: &HtmlElement) -> u32 {
        if !WATCHING.with(Cell::get) {
            WATCHING.with(|watching| watching.set(true));
            let release = Closure::<dyn FnMut(u32)>::new(release_all).into_js_value();
            watch_with(&release);
        }
        host_id(host)
    }

    /// Registers how a table forgets an id. Idempotent, so a table may call it
    /// on every insert instead of needing an initialisation step.
    pub fn on_release(release: fn(u32)) {
        RELEASES.with(|releases| {
            let mut releases = releases.borrow_mut();
            if !releases
                .iter()
                .any(|known| std::ptr::fn_addr_eq(*known, release))
            {
                releases.push(release);
            }
        });
    }

    fn release_all(id: u32) {
        let releases = RELEASES.with(|releases| releases.borrow().clone());
        for release in releases {
            release(id);
        }
    }

    // The ids live in a `WeakMap` of this module instance, not under a global
    // symbol on the host: a second copy of the module (two bundles, a hot
    // reload) keeps its own tables and its own counter, and must not read an
    // id the first one gave out.
    #[wasm_bindgen(inline_js = "
const ids = new WeakMap();
let next = 1;
let registry = null;

export function __opengrid_watch_with(release) {
    registry = new FinalizationRegistry(release);
}

export function __opengrid_known_id(host) {
    return ids.get(host) ?? 0;
}

export function __opengrid_host_id(host) {
    let id = ids.get(host);
    if (id === undefined) {
        id = next++;
        ids.set(host, id);
        registry.register(host, id);
    }
    return id;
}
")]
    extern "C" {
        #[wasm_bindgen(js_name = __opengrid_watch_with)]
        fn watch_with(release: &JsValue);
        #[wasm_bindgen(js_name = __opengrid_known_id)]
        fn known_id(host: &HtmlElement) -> u32;
        #[wasm_bindgen(js_name = __opengrid_host_id)]
        fn host_id(host: &HtmlElement) -> u32;
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{existing_id, id, on_release};
