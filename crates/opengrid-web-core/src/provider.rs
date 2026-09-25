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
//!   local [`LocalProvider`] resolves on the spot; the page-owned engine is
//!   wrapped by [`JsProvider`] (point 14), and the Worker-backed provider of
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
    ///
    /// `mode` is the element's `mode` attribute, or `""` when it has none. A
    /// provider that runs the whole query in one place ignores it; a provider
    /// that splits the work between a source and the engine (plan point 28)
    /// reads it, and that is the only reason it travels this far.
    fn execute(&self, query_json: &str, mode: &str) -> js_sys::Promise;
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
    /// The mode is ignored: there is only one place for the work to happen.
    fn execute(&self, query_json: &str, _mode: &str) -> js_sys::Promise {
        match self.executor.execute(query_json) {
            Ok(result) => js_sys::Promise::resolve(&JsValue::from_str(&result)),
            Err(error) => js_sys::Promise::reject(&JsValue::from_str(&error.to_string())),
        }
    }
}

/// A [`DataProvider`] over a JavaScript object with an `execute(queryJson)`
/// method.
///
/// The page owns the engine: it hands the component a plain JS object whose
/// `execute` answers a Promise (or a value, which is wrapped in a resolved
/// Promise). This keeps `opengrid-web-components` engine-agnostic — the fixture
/// wraps the local engine, point 19 wraps the Worker, and both look identical
/// from here.
#[cfg(target_arch = "wasm32")]
pub struct JsProvider {
    object: JsValue,
}

#[cfg(target_arch = "wasm32")]
impl JsProvider {
    /// Wraps a JS object exposing `execute(queryJson)`.
    pub fn new(object: JsValue) -> Self {
        Self { object }
    }

    /// The wrapped object.
    pub fn object(&self) -> &JsValue {
        &self.object
    }
}

#[cfg(target_arch = "wasm32")]
impl DataProvider for JsProvider {
    /// The mode goes along as a second argument. A provider that does not take
    /// one simply ignores it — that is how JavaScript calls work, and it is why
    /// the seam did not have to change shape for point 28.
    fn execute(&self, query_json: &str, mode: &str) -> js_sys::Promise {
        use wasm_bindgen::JsCast;

        let called =
            js_sys::Reflect::get(&self.object, &JsValue::from_str("execute")).and_then(|method| {
                match method.dyn_into::<js_sys::Function>() {
                    Ok(function) => function.call2(
                        &self.object,
                        &JsValue::from_str(query_json),
                        &JsValue::from_str(mode),
                    ),
                    Err(_) => Err(JsValue::from_str(
                        "provider has no execute(queryJson) method",
                    )),
                }
            });

        match called {
            // A synchronous value is a resolved promise with that value.
            Ok(value) => match value.dyn_ref::<js_sys::Promise>() {
                Some(promise) => promise.clone(),
                None => js_sys::Promise::resolve(&value),
            },
            Err(error) => js_sys::Promise::reject(&error),
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::{provider, set_provider};

/// Per-host storage of the data provider.
///
/// **On the host, not in a Rust table** (point 74). The provider is the
/// page's object, and a page's object may well reach the host — a closure over
/// a component's ref is the everyday case. Held from WASM, that would be a
/// cycle through a root the garbage collector cannot see through, and a grid
/// removed for good would never be collected. On the host, the cycle lives in
/// the JS heap alone, where it is collected like any other.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::rc::Rc;

    use wasm_bindgen::JsValue;
    use web_sys::HtmlElement;

    use super::{DataProvider, JsProvider};

    /// The global symbol the provider is stored under; `Symbol.for` is stable
    /// across modules in the same realm, unlike a private `Symbol()`.
    fn provider_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.provider")
    }

    /// Attaches a provider object to `host`, replacing any previous one.
    pub fn set_provider(host: &HtmlElement, provider: &JsValue) {
        let _ = js_sys::Reflect::set(host.as_ref(), provider_symbol().as_ref(), provider);
    }

    /// The provider attached to `host`, if any.
    pub fn provider(host: &HtmlElement) -> Option<Rc<dyn DataProvider>> {
        let object = js_sys::Reflect::get(host.as_ref(), provider_symbol().as_ref()).ok()?;
        if object.is_undefined() || object.is_null() {
            return None;
        }
        Some(Rc::new(JsProvider::new(object)))
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
