//! The async data interface the custom elements consume.
//!
//! The plan pins the timing: the Worker arrives late (point 19), but the data
//! interface is asynchronous **from the start**, so moving the engine off the
//! main thread later only swaps the implementation
//! (plan/spezifikation/04-local-engine.md §Worker "Zeitpunkt"). This module is
//! that interface, one layer above the `DataSource` trait (plan point 09, E5):
//! it speaks JSON in and JSON out, so it stays engine-agnostic.
//!
//! * [`QueryExecutor`] is the portable, synchronous seam an engine implements —
//!   the local WASM engine today, a native test now.
//! * [`DataProvider`] is the Promise-based contract the components use. The
//!   local [`LocalProvider`] resolves on the spot; the Worker-backed provider of
//!   point 19 returns a promise that settles later, with the same signature.
//!
//! A provider can be attached to a host element with [`set_provider`] and read
//! back with [`provider`]; point 14 fills the element with the actual query.

use std::fmt;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

/// A query failed. The message is shown to the application unchanged, so it
/// should name the cause, not an internal type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderError {
    message: String,
}

impl ProviderError {
    /// Wraps a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

/// The portable engine seam: a query JSON in, the result JSON out.
///
/// Implemented by the local engine binding (point 10) and by test doubles. It is
/// deliberately synchronous — the local path never suspends
/// (plan/spezifikation/04-local-engine.md §Browser-Bindung) — and portable, so
/// it builds and tests on the host.
pub trait QueryExecutor {
    /// Runs a query and answers its result.
    fn execute(&self, query_json: &str) -> Result<String, ProviderError>;
}

/// The Promise-based provider the components consume (point 19 will implement it
/// over a Worker).
#[cfg(target_arch = "wasm32")]
pub trait DataProvider {
    /// Runs a query JSON and answers a promise that resolves with the result
    /// JSON, or rejects with an error message.
    fn execute(&self, query_json: &str) -> js_sys::Promise;
}

/// A [`DataProvider`] over a [`QueryExecutor`] that is ready on the spot.
#[cfg(target_arch = "wasm32")]
pub struct LocalProvider<E> {
    executor: E,
}

#[cfg(target_arch = "wasm32")]
impl<E> LocalProvider<E> {
    /// Wraps a synchronous executor.
    pub fn new(executor: E) -> Self {
        Self { executor }
    }

    /// The wrapped executor.
    pub fn executor(&self) -> &E {
        &self.executor
    }
}

#[cfg(target_arch = "wasm32")]
impl<E: QueryExecutor> DataProvider for LocalProvider<E> {
    fn execute(&self, query_json: &str) -> js_sys::Promise {
        match self.executor.execute(query_json) {
            Ok(result) => js_sys::Promise::resolve(&JsValue::from_str(&result)),
            Err(error) => js_sys::Promise::reject(&JsValue::from_str(&error.to_string())),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::{provider, set_provider};

/// Per-host storage of the data provider.
///
/// Keyed by a global symbol id on the host element instead of a side table
/// keyed by the JS object, because a `web_sys::HtmlElement` has no stable Rust
/// identity across callbacks. The provider itself is a Rust `Rc`, so the map is
/// thread-local; nothing here crosses a thread.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use wasm_bindgen::JsValue;
    use web_sys::HtmlElement;

    use super::DataProvider;

    thread_local! {
        static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
        static PROVIDERS: RefCell<HashMap<u32, Rc<dyn DataProvider>>> =
            RefCell::new(HashMap::new());
    }

    /// The global symbol the id is stored under; `Symbol.for` is stable across
    /// modules in the same realm, unlike a private `Symbol()`.
    fn id_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.provider_id")
    }

    /// Attaches a provider to `host`, replacing any previous one.
    pub fn set_provider(host: &HtmlElement, provider: Rc<dyn DataProvider>) {
        let id = NEXT_ID.with(|next| {
            let mut next = next.borrow_mut();
            let id = *next;
            *next += 1;
            id
        });
        PROVIDERS.with(|providers| providers.borrow_mut().insert(id, provider));
        let _ = js_sys::Reflect::set(
            host.as_ref(),
            id_symbol().as_ref(),
            &JsValue::from_f64(f64::from(id)),
        );
    }

    /// The provider attached to `host`, if any.
    pub fn provider(host: &HtmlElement) -> Option<Rc<dyn DataProvider>> {
        let id = js_sys::Reflect::get(host.as_ref(), id_symbol().as_ref())
            .ok()
            .and_then(|value| value.as_f64())? as u32;
        PROVIDERS.with(|providers| providers.borrow().get(&id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeExecutor;

    impl QueryExecutor for FakeExecutor {
        fn execute(&self, query_json: &str) -> Result<String, ProviderError> {
            if query_json == "{}" {
                Ok("{\"total_count\":0}".to_owned())
            } else {
                Err(ProviderError::new("bad query"))
            }
        }
    }

    /// The portable seam is testable on the host, without a browser.
    #[test]
    fn the_executor_seam_is_engine_agnostic() {
        let executor = FakeExecutor;
        assert_eq!(
            executor.execute("{}").unwrap(),
            "{\"total_count\":0}".to_owned()
        );
        let error = executor.execute("nope").unwrap_err();
        assert_eq!(error.message(), "bad query");
        assert_eq!(error.to_string(), "bad query");
    }
}
