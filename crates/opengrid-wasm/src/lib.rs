//! opengrid in the browser: the WASM face of the local engine.
//!
//! Point 10 of the plan. The JS side stays thin
//! (plan/spezifikation/08-rendering.md §JavaScript-Minimum: 5 % JS, 95 % Rust):
//! it hands bytes and JSON over and renders what comes back. Everything else —
//! ingestion, validation, filter, sort, group, aggregate, paging — happens in
//! Rust, and it happens **through the `DataSource` trait** (point 09), not by
//! calling the executor directly. The browser therefore exercises the same
//! contract the server will answer through.
//!
//! The JS API is the one the plan names:
//!
//! ```js
//! const engine = new Engine();            // wasm-pack / wasm-bindgen glue
//! engine.load_csv("orders", bytes, schemaJson);
//! const result = engine.execute(queryJson);
//! ```
//!
//! `load_csv` names the source; the query JSON carries the same name in its
//! `source` field, so one engine can hold several datasets (E6).
//!
//! No Arrow type crosses this boundary: both directions speak the
//! column-oriented JSON of E6, values in the wire notation of E13 (decimals as
//! strings, non-finite floats as `"NaN"`/`"Infinity"`/`"-Infinity"`).

pub mod schema_json;

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv as read_csv};
use opengrid_datasource::{DataSource, DataSourceCapabilities, QueryResult};
use opengrid_planner::ExecutionMode;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataSourceId, Schema};
use wasm_bindgen::prelude::*;

/// The engine the browser talks to: a set of in-memory sources.
///
/// One instance holds one [`LocalDataSource`] per name
/// (plan/spezifikation/03-datasource.md). The demo registers `orders` and runs
/// the example query of `spezifikation/02-query-modell.md` against it.
#[wasm_bindgen]
pub struct Engine {
    sources: HashMap<String, LocalDataSource>,
}

#[wasm_bindgen]
impl Engine {
    /// An engine without data. `load_csv` adds sources.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Engine {
        Engine {
            sources: HashMap::new(),
        }
    }

    /// Reads CSV bytes against an explicit schema and registers the source.
    ///
    /// The schema is required — ingest never guesses types, and an inferred
    /// proposal is a suggestion a UI may show
    /// (plan/spezifikation/04-local-engine.md §Ingest).
    #[wasm_bindgen(js_name = load_csv)]
    pub fn load_csv_js(
        &mut self,
        name: &str,
        bytes: &[u8],
        schema_json: &str,
    ) -> Result<(), JsError> {
        self.load_csv(name, bytes, schema_json)
            .map_err(|message| JsError::new(&message))
    }

    /// Runs a query JSON and answers with the result JSON.
    ///
    /// The result is the wire shape of a [`QueryResult`]: `total_count` for the
    /// number before paging, `row_count` for this page, and one
    /// `{ "name", "values" }` object per output column, in schema order.
    #[wasm_bindgen(js_name = execute)]
    pub fn execute_js(&self, query_json: &str) -> Result<String, JsError> {
        let result = self
            .execute_result(query_json)
            .map_err(|message| JsError::new(&message))?;
        Ok(result_json_of(&result))
    }

    /// The names of the registered sources, in unspecified order.
    #[wasm_bindgen(js_name = source_names)]
    pub fn source_names(&self) -> Vec<String> {
        self.sources.keys().cloned().collect()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// The Rust side of the JS API — the same paths, minus the `JsError` wrapping.
///
/// Kept public (and outside the `#[wasm_bindgen]` block) so the browser tests
/// can drive the engine and compare a typed [`QueryResult`] against the suite's
/// expectations, instead of comparing JSON.
impl Engine {
    /// Reads CSV bytes against an explicit schema and registers the source
    /// under `name`.
    pub fn load_csv(&mut self, name: &str, bytes: &[u8], schema_json: &str) -> Result<(), String> {
        let id = DataSourceId::new(name).map_err(|error| format!("source {name:?}: {error}"))?;
        let schema =
            schema_json::from_json(schema_json).map_err(|error| format!("schema: {error}"))?;
        let batches =
            read_csv(bytes, &schema, CsvOptions::default()).map_err(|error| error.to_string())?;
        let source = LocalDataSource::new(batches).map_err(|error| error.to_string())?;
        self.sources.insert(id.as_str().to_owned(), source);
        Ok(())
    }

    /// Runs a query JSON against the source it names and answers with the
    /// result.
    ///
    /// The path is the one the server takes: parse the JSON, read the schema of
    /// the named source, validate the query against it, execute. Validation is
    /// not optional — [`DataSource::execute`] accepts only a
    /// [`ValidatedQuery`](opengrid_query::ValidatedQuery).
    pub fn execute_result(&self, query_json: &str) -> Result<QueryResult, String> {
        let query: Query =
            serde_json::from_str(query_json).map_err(|error| format!("query JSON: {error}"))?;
        let source = self
            .sources
            .get(query.source.as_str())
            .ok_or_else(|| format!("unknown source {:?}", query.source.as_str()))?;

        let schema = block_on(DataSource::schema(source)).map_err(|error| error.to_string())?;
        let validated = query
            .validate(&schema, &Limits::default())
            .map_err(|error| error.to_string())?;
        block_on(DataSource::execute(source, validated)).map_err(|error| error.to_string())
    }
}

/// Drives a future that is ready on the first poll.
///
/// The local source never awaits — it holds batches and runs the executor
/// (plan/spezifikation/04-local-engine.md §DataSource-Adapter) — so no executor
/// is needed and none is pulled into the bundle. A future that does suspend is
/// a bug on our side: it panics with a diagnosis instead of hanging the tab.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the local engine awaited; it must not"),
    }
}

/// The wire shape of a result (E6/E14), column-oriented.
///
/// Since point 23 the form lives in `opengrid_datasource::wire`, so the engine,
/// the server and the browser write and read the same bytes — and a column now
/// carries its **type** next to its values. Values keep the notation of E13: a
/// decimal is a string, `NaN` is `"NaN"`.
fn result_json_of(result: &QueryResult) -> String {
    opengrid_datasource::wire::result_to_json(result)
}

/// The planner in the browser: who answers which half of a query (point 28).
///
/// The page holds a remote source it cannot change and an engine it can. This
/// object decides between them and does the local half:
///
/// 1. [`plan`](Planner::plan) turns a query into the query for the source, the
///    query for the client, and a sentence describing the split.
/// 2. The page sends the source query wherever it goes and gets an answer back.
/// 3. [`finish`](Planner::finish) runs the client query over that answer.
///
/// It holds no state between the two calls on purpose: the plan travels as JSON
/// through the page, which is also what makes it readable from outside
/// (plan/spezifikation/05-planner.md: "Offengelegt für Debugging").
#[wasm_bindgen]
pub struct Planner {
    schema: Schema,
    capabilities: DataSourceCapabilities,
    mode: ExecutionMode,
}

#[wasm_bindgen]
impl Planner {
    /// A planner for a source with `schema_json` and `capabilities_json`, in
    /// `mode` (`local`, `remote`, `hybrid` or `auto`).
    ///
    /// Both JSON documents are what `GET /source/{name}` answers, so a page can
    /// hand them straight through without knowing their shape.
    #[wasm_bindgen(constructor)]
    pub fn new(schema_json: &str, capabilities_json: &str, mode: &str) -> Result<Planner, JsError> {
        Planner::build(schema_json, capabilities_json, mode).map_err(|error| JsError::new(&error))
    }

    /// Splits `query_json`, answering the plan as JSON.
    ///
    /// `mode` overrides the planner's own mode when it is not empty — that is
    /// how the element's `mode` attribute reaches the planner without rebuilding
    /// it on every attribute change.
    ///
    /// The answer is `{ mode, describe, steps, source, client }`: `source` is the
    /// query to send on, `client` the query to hand back to
    /// [`finish`](Planner::finish), or `null` when the source answers it all.
    pub fn plan(&self, query_json: &str, mode: &str) -> Result<String, JsError> {
        self.plan_json(query_json, mode)
            .map_err(|error| JsError::new(&error))
    }

    /// Runs `client_query_json` over `result_json` — the source's answer.
    ///
    /// This is the hybrid return path E14 names: the partial result becomes
    /// Arrow batches again and the engine finishes the query over them, which
    /// includes recounting `total_count` — filtering or grouping changed it.
    pub fn finish(&self, client_query_json: &str, result_json: &str) -> Result<String, JsError> {
        self.finish_result(client_query_json, result_json)
            .map(|result| result_json_of(&result))
            .map_err(|message| JsError::new(&message))
    }
}

/// The Rust side of the JS API — the same paths, minus the `JsError` wrapping,
/// which cannot even be constructed off the browser.
impl Planner {
    /// A planner over a schema and a capability declaration, both as JSON.
    pub fn build(
        schema_json: &str,
        capabilities_json: &str,
        mode: &str,
    ) -> Result<Planner, String> {
        let schema =
            schema_json::from_json(schema_json).map_err(|error| format!("schema: {error}"))?;
        let capabilities: DataSourceCapabilities = serde_json::from_str(capabilities_json)
            .map_err(|error| format!("capabilities: {error}"))?;
        Ok(Planner {
            schema,
            capabilities,
            mode: parse_mode(mode)?,
        })
    }

    /// Splits a query and writes the plan as JSON.
    pub fn plan_json(&self, query_json: &str, mode: &str) -> Result<String, String> {
        let query: Query =
            serde_json::from_str(query_json).map_err(|error| format!("query JSON: {error}"))?;
        let validated = query
            .validate(&self.schema, &Limits::default())
            .map_err(|error| error.to_string())?;
        let mode = if mode.is_empty() {
            self.mode
        } else {
            parse_mode(mode)?
        };

        let plan = opengrid_planner::plan(&validated, &self.schema, self.capabilities, mode)
            .map_err(|error| error.to_string())?;

        let body = serde_json::json!({
            "mode": mode.as_str(),
            "describe": plan.describe(),
            "steps": plan.client_steps.iter().map(|step| step.as_str()).collect::<Vec<_>>(),
            "source": Query::from(&plan.source_query),
            "client": plan.client_query.as_ref().map(Query::from),
        });
        Ok(body.to_string())
    }

    /// Runs the client half over the source's answer.
    pub fn finish_result(
        &self,
        client_query_json: &str,
        result_json: &str,
    ) -> Result<QueryResult, String> {
        let partial = opengrid_datasource::wire::result_from_json(result_json)
            .map_err(|error| format!("result JSON: {error}"))?;
        let source = LocalDataSource::from_result(&partial).map_err(|error| error.to_string())?;

        let query: Query = serde_json::from_str(client_query_json)
            .map_err(|error| format!("client query JSON: {error}"))?;
        let validated = query
            .validate(&partial.schema, &Limits::default())
            .map_err(|error| error.to_string())?;
        block_on(DataSource::execute(&source, validated)).map_err(|error| error.to_string())
    }
}

/// Reads a `mode` attribute; an unknown spelling is an error, never a fallback.
fn parse_mode(mode: &str) -> Result<ExecutionMode, String> {
    ExecutionMode::parse(mode)
        .ok_or_else(|| format!("unknown mode {mode:?}: expected local, remote, hybrid or auto"))
}
