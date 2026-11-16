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
//! No engine type crosses this boundary: both directions speak the
//! column-oriented JSON of E6, values in the wire notation of E13 (decimals as
//! strings, non-finite floats as `"NaN"`/`"Infinity"`/`"-Infinity"`).

pub mod schema_json;

use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use opengrid_datasource::{DataSource, DataSourceCapabilities, QueryResult};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{
    ColumnsLoader, CsvLoader, CsvOptions, load_csv as read_csv, load_json as read_json,
};
use opengrid_planner::ExecutionMode;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataSourceId, Schema};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// The function of the collation module, `sort_keys(locale, texts, ends)`
    /// (`crates/opengrid-collation`): a JavaScript function, called as one.
    pub type CollatorFunction;

    #[wasm_bindgen(method, catch, js_name = call)]
    fn call(
        this: &CollatorFunction,
        receiver: &JsValue,
        locale: &str,
        texts: &[u8],
        ends: &[u32],
    ) -> Result<Vec<u8>, JsValue>;
}

/// The collation module behind the engine's [`Collator`]: the strings of a
/// column go out in one buffer and their keys come back in one.
struct ModuleCollator(CollatorFunction);

impl opengrid_engine::collation::Collator for ModuleCollator {
    fn sort_keys(
        &self,
        locale: &str,
        texts: &mut dyn Iterator<Item = &str>,
        keys: &mut opengrid_columns::sort::SortKeys,
    ) -> Result<(), String> {
        let mut bytes = Vec::new();
        let mut ends = Vec::new();
        for text in texts {
            bytes.extend_from_slice(text.as_bytes());
            ends.push(
                u32::try_from(bytes.len())
                    .map_err(|_| "a column of more than four gigabytes of text".to_owned())?,
            );
        }
        let packed = self
            .0
            .call(&JsValue::UNDEFINED, locale, &bytes, &ends)
            .map_err(|error| {
                let message = js_message(&error);
                format!("the collator refused {locale:?}: {message}")
            })?;
        let head = 4 * ends.len();
        let wrong = || "the collator answered something that is not its keys".to_owned();
        let body = packed.get(head..).ok_or_else(wrong)?;
        let mut start = 0;
        for end in packed[..head].as_chunks::<4>().0 {
            let end = u32::from_le_bytes(*end) as usize;
            keys.push(body.get(start..end).ok_or_else(wrong)?);
            start = end;
        }
        Ok(())
    }
}

/// The message of what JavaScript threw, where it has one.
fn js_message(error: &JsValue) -> String {
    #[wasm_bindgen]
    extern "C" {
        type Thrown;
        #[wasm_bindgen(method, getter)]
        fn message(this: &Thrown) -> JsValue;
    }
    if error.is_object()
        && let Some(message) = error.unchecked_ref::<Thrown>().message().as_string()
    {
        return message;
    }
    error.as_string().unwrap_or_else(|| "no message".to_owned())
}

/// The collator of every engine and planner of this module: they sort on one
/// thread, and the collator belongs to that.
fn install_collator(sort_keys: CollatorFunction) {
    opengrid_engine::collation::install(std::rc::Rc::new(ModuleCollator(sort_keys)));
}

/// The engine the browser talks to: a set of in-memory sources.
///
/// One instance holds one [`LocalDataSource`] per name
/// (plan/spezifikation/03-datasource.md). The demo registers `orders` and runs
/// the example query of `spezifikation/02-query-modell.md` against it.
#[wasm_bindgen]
pub struct Engine {
    sources: HashMap<String, LocalDataSource>,
    /// The CSV file being read in pieces (`begin_csv`), with the name it will be registered under.
    pending: Option<(String, CsvLoader)>,
    /// The rows being read as packs of columns (`begin_columns`), with the name they will be
    /// registered under.
    pending_columns: Option<(String, ColumnsLoader)>,
}

#[wasm_bindgen]
impl Engine {
    /// An engine without data. `load_csv` adds sources.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Engine {
        Engine {
            sources: HashMap::new(),
            pending: None,
            pending_columns: None,
        }
    }

    /// Gives the engine its collator (rule S4a, issue #391): `sort_keys` of
    /// the collation module, which a page loads when a query first sorts by a
    /// locale — the providers of the package do. Until then such a sort is
    /// refused; a sort without a locale never needs it. It holds for every
    /// engine and planner of this module.
    pub fn set_collator(&self, sort_keys: CollatorFunction) {
        install_collator(sort_keys);
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

    /// Starts reading a CSV file in pieces (issue #427): `push_csv` for each piece, then
    /// `end_csv`. Only a piece and the columns are in memory, not the whole file as well. The
    /// pieces may end anywhere. `size_hint` is the file's size in bytes, 0 when unknown; it only
    /// sizes the columns. A file begun before and not ended is dropped.
    #[wasm_bindgen(js_name = begin_csv)]
    pub fn begin_csv_js(
        &mut self,
        name: &str,
        schema_json: &str,
        size_hint: f64,
    ) -> Result<(), JsError> {
        self.begin_csv(name, schema_json, size_hint)
            .map_err(|message| JsError::new(&message))
    }

    /// The next piece of the file `begin_csv` started. A piece that cannot be read ends the
    /// file: nothing is registered, and the error names the line.
    #[wasm_bindgen(js_name = push_csv)]
    pub fn push_csv_js(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        self.push_csv(bytes)
            .map_err(|message| JsError::new(&message))
    }

    /// Drops the file `begin_csv` started, nothing registered: for a page whose reading of the
    /// file failed. Nothing to drop is not an error.
    pub fn abort_csv(&mut self) {
        self.pending = None;
    }

    /// The end of the file: registers the source, replacing one of that name.
    #[wasm_bindgen(js_name = end_csv)]
    pub fn end_csv_js(&mut self) -> Result<(), JsError> {
        self.end_csv().map_err(|message| JsError::new(&message))
    }

    /// Starts reading the rows a page holds as packs of typed columns (issue #445): `push_columns`
    /// for each pack, then `end_columns`. `rows` is the number of rows of the whole table; the
    /// columns are sized by it. The format of a pack is in `opengrid_engine::ingest`. Rows begun
    /// before and not ended are dropped. An error here or later sends the page to `load_rows`,
    /// which owns every message.
    #[wasm_bindgen(js_name = begin_columns)]
    pub fn begin_columns_js(
        &mut self,
        name: &str,
        schema_json: &str,
        rows: f64,
    ) -> Result<(), JsError> {
        self.begin_columns(name, schema_json, rows)
            .map_err(|message| JsError::new(&message))
    }

    /// The next pack of the rows `begin_columns` started. A pack that does not hold ends the
    /// reading: nothing is registered.
    #[wasm_bindgen(js_name = push_columns)]
    pub fn push_columns_js(&mut self, pack: &[u8]) -> Result<(), JsError> {
        self.push_columns(pack)
            .map_err(|message| JsError::new(&message))
    }

    /// Drops the rows `begin_columns` started, nothing registered. Nothing to drop is not an
    /// error.
    pub fn abort_columns(&mut self) {
        self.pending_columns = None;
    }

    /// The end of the rows: registers the source, replacing one of that name.
    #[wasm_bindgen(js_name = end_columns)]
    pub fn end_columns_js(&mut self) -> Result<(), JsError> {
        self.end_columns().map_err(|message| JsError::new(&message))
    }

    /// Registers a source from rows the page already holds (issue #182): a
    /// JSON array of objects keyed by field name, values in the wire notation
    /// of E13. The schema is required, as for CSV. A name that is already
    /// loaded is **replaced** — all or nothing: a row that does not fit is
    /// refused with its row and field named, and the old source stays.
    #[wasm_bindgen(js_name = load_rows)]
    pub fn load_rows_js(
        &mut self,
        name: &str,
        rows_json: &str,
        schema_json: &str,
    ) -> Result<(), JsError> {
        self.load_rows(name, rows_json, schema_json)
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

    /// Runs a query JSON and answers with the result in the binary form
    /// (decision E35, `opengrid_columns::wire`): the same answer as
    /// [`execute`](Engine::execute_js), but as bytes a worker can hand over
    /// without copying and an element can read without parsing.
    #[wasm_bindgen(js_name = execute_columns)]
    pub fn execute_columns_js(&self, query_json: &str) -> Result<Vec<u8>, JsError> {
        self.execute_columns(query_json)
            .map_err(|message| JsError::new(&message))
    }

    /// What the engine holds (issue #70), as JSON:
    /// `{ "memory": bytes, "sources": [{ "name", "rows", "columns", "bytes" }] }`.
    ///
    /// `memory` is the module's whole linear memory — what the browser gave
    /// it, which only grows; `bytes` is what a source's columns take inside
    /// it. Sources by name.
    #[wasm_bindgen(js_name = stats)]
    pub fn stats_js(&self) -> String {
        self.stats().to_string()
    }

    /// The query of a view (issue #144): what `get_query()` answers on an
    /// `<opengrid-grid datasource=source columns=…>` once `view_json` is
    /// applied — without an element, so a server can count or export a saved
    /// view. `columns` is the grid's `columns` attribute, as a JSON array.
    ///
    /// A view naming a column the grid does not have is refused as a whole,
    /// every problem named; so is a filter value that is not a value of its
    /// column.
    #[wasm_bindgen(js_name = view_query)]
    pub fn view_query_js(
        &self,
        source: &str,
        columns_json: &str,
        view_json: &str,
    ) -> Result<String, JsError> {
        self.view_query(source, columns_json, view_json)
            .map_err(|message| JsError::new(&message))
    }

    /// Runs a pivot JSON over the source it names and answers the pivot's JSON
    /// form (issue #28) — what `POST /pivot/{source}` answers, computed here.
    #[wasm_bindgen(js_name = pivot)]
    pub fn pivot_js(&self, pivot_json: &str) -> Result<String, JsError> {
        let (result, rows) = self
            .pivot_result(pivot_json)
            .map_err(|message| JsError::new(&message))?;
        Ok(opengrid_pivot::pivot_to_json(&result, &rows))
    }

    /// The same pivot in the binary form (E35) — what the tab and the worker
    /// hand the element.
    #[wasm_bindgen(js_name = pivot_columns)]
    pub fn pivot_columns_js(&self, pivot_json: &str) -> Result<Vec<u8>, JsError> {
        let (result, rows) = self
            .pivot_result(pivot_json)
            .map_err(|message| JsError::new(&message))?;
        Ok(opengrid_pivot::pivot_to_bytes(&result, &rows))
    }

    /// The names of the registered sources, in unspecified order.
    #[wasm_bindgen(js_name = source_names)]
    pub fn source_names(&self) -> Vec<String> {
        self.sources.keys().cloned().collect()
    }

    /// Frees the source `name` (issue #459); `true` when there was one. The
    /// module's memory does not shrink — WebAssembly memory only grows — but
    /// the next load takes the room the table had, so a page that unloads
    /// before it loads again needs one table's room, not two.
    #[wasm_bindgen(js_name = unload)]
    pub fn unload(&mut self, name: &str) -> bool {
        self.sources.remove(name).is_some()
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
        let table =
            read_csv(bytes, &schema, CsvOptions::default()).map_err(|error| error.to_string())?;
        let source = LocalDataSource::new(table);
        self.sources.insert(id.as_str().to_owned(), source);
        Ok(())
    }

    /// [`Engine::begin_csv_js`] without the `JsError`.
    pub fn begin_csv(
        &mut self,
        name: &str,
        schema_json: &str,
        size_hint: f64,
    ) -> Result<(), String> {
        self.pending = None;
        let id = DataSourceId::new(name).map_err(|error| format!("source {name:?}: {error}"))?;
        let schema =
            schema_json::from_json(schema_json).map_err(|error| format!("schema: {error}"))?;
        let loader = CsvLoader::new(&schema, CsvOptions::default())
            .map_err(|error| error.to_string())?
            .with_size_hint(size_hint as usize);
        self.pending = Some((id.as_str().to_owned(), loader));
        Ok(())
    }

    /// [`Engine::push_csv_js`] without the `JsError`.
    pub fn push_csv(&mut self, bytes: &[u8]) -> Result<(), String> {
        let Some((_, loader)) = &mut self.pending else {
            return Err("push_csv without begin_csv".to_owned());
        };
        loader.feed(bytes).map_err(|error| {
            self.pending = None;
            error.to_string()
        })
    }

    /// [`Engine::end_csv_js`] without the `JsError`.
    pub fn end_csv(&mut self) -> Result<(), String> {
        let Some((name, loader)) = self.pending.take() else {
            return Err("end_csv without begin_csv".to_owned());
        };
        let table = loader.finish().map_err(|error| error.to_string())?;
        self.sources.insert(name, LocalDataSource::new(table));
        Ok(())
    }

    /// [`Engine::begin_columns_js`] without the `JsError`.
    pub fn begin_columns(
        &mut self,
        name: &str,
        schema_json: &str,
        rows: f64,
    ) -> Result<(), String> {
        self.pending_columns = None;
        let id = DataSourceId::new(name).map_err(|error| format!("source {name:?}: {error}"))?;
        let schema =
            schema_json::from_json(schema_json).map_err(|error| format!("schema: {error}"))?;
        if !(0.0..=4_294_967_295.0).contains(&rows) || rows.fract() != 0.0 {
            return Err(format!("{rows} is not a number of rows"));
        }
        let loader =
            ColumnsLoader::new(&schema, rows as usize).map_err(|error| error.to_string())?;
        self.pending_columns = Some((id.as_str().to_owned(), loader));
        Ok(())
    }

    /// [`Engine::push_columns_js`] without the `JsError`.
    pub fn push_columns(&mut self, pack: &[u8]) -> Result<(), String> {
        let Some((_, loader)) = &mut self.pending_columns else {
            return Err("push_columns without begin_columns".to_owned());
        };
        loader.feed(pack).map_err(|error| {
            self.pending_columns = None;
            error.to_string()
        })
    }

    /// [`Engine::end_columns_js`] without the `JsError`.
    pub fn end_columns(&mut self) -> Result<(), String> {
        let Some((name, loader)) = self.pending_columns.take() else {
            return Err("end_columns without begin_columns".to_owned());
        };
        let table = loader.finish().map_err(|error| error.to_string())?;
        self.sources.insert(name, LocalDataSource::new(table));
        Ok(())
    }

    /// Reads a JSON array of objects against an explicit schema and registers
    /// the source under `name`, replacing one of that name.
    pub fn load_rows(
        &mut self,
        name: &str,
        rows_json: &str,
        schema_json: &str,
    ) -> Result<(), String> {
        let id = DataSourceId::new(name).map_err(|error| format!("source {name:?}: {error}"))?;
        let schema =
            schema_json::from_json(schema_json).map_err(|error| format!("schema: {error}"))?;
        let table = read_json(rows_json.as_bytes(), &schema).map_err(|error| error.to_string())?;
        self.sources
            .insert(id.as_str().to_owned(), LocalDataSource::new(table));
        Ok(())
    }

    /// [`view_query`](Engine::view_query_js) without the `JsError` wrapping.
    pub fn view_query(
        &self,
        source: &str,
        columns_json: &str,
        view_json: &str,
    ) -> Result<String, String> {
        use opengrid_json::Json;
        let table = self
            .sources
            .get(source)
            .ok_or_else(|| format!("unknown source {source:?}"))?
            .table();
        let declared: Vec<String> = match Json::parse(columns_json) {
            Ok(Json::Array(columns)) => columns
                .iter()
                .map(|column| column.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or("columns: an array of names")?,
            _ => return Err("columns: an array of names".to_owned()),
        };
        let view = Json::parse(view_json).map_err(|error| format!("view: {error}"))?;
        let view = opengrid_grid::grid_view::GridView::from_json(&view, &declared).map_err(
            |problems| {
                problems
                    .iter()
                    .map(|problem| format!("{}: {}", problem.field, problem.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            },
        )?;
        opengrid_grid::view_query::view_query(source, &declared, &view, table.schema())
            .map_err(|problems| problems.join("; "))
    }

    /// [`stats`](Engine::stats_js) as a JSON value.
    pub fn stats(&self) -> opengrid_json::Json {
        use opengrid_json::Json;
        let mut names: Vec<&String> = self.sources.keys().collect();
        names.sort();
        let sources: Vec<Json> = names
            .into_iter()
            .map(|name| {
                let table = self.sources[name].table();
                Json::object([
                    ("name", Json::from(name.as_str())),
                    ("rows", Json::from(table.num_rows())),
                    ("columns", Json::from(table.schema().len())),
                    ("bytes", Json::from(table.byte_size())),
                ])
            })
            .collect();
        Json::object([
            ("memory", Json::from(linear_memory())),
            ("sources", Json::from(sources)),
        ])
    }

    /// Runs a query JSON against the source it names and answers with the
    /// result.
    ///
    /// The path is the one the server takes: parse the JSON, read the schema of
    /// the named source, validate the query against it, execute. Validation is
    /// not optional — [`DataSource::execute`] accepts only a
    /// [`ValidatedQuery`](opengrid_query::ValidatedQuery).
    pub fn execute_result(&self, query_json: &str) -> Result<QueryResult, String> {
        let (source, validated) = self.admit(query_json)?;
        block_on(DataSource::execute(source, validated)).map_err(|error| error.to_string())
    }

    /// The same run as [`execute_result`](Engine::execute_result), answered in
    /// the binary form.
    pub fn execute_columns(&self, query_json: &str) -> Result<Vec<u8>, String> {
        let (source, validated) = self.admit(query_json)?;
        let answer = source.run(&validated).map_err(|error| error.to_string())?;
        Ok(encode_answer(&answer))
    }

    /// A pivot over the source it names, validated against that source's schema
    /// with the server's default limits, and its row dimensions.
    pub fn pivot_result(
        &self,
        pivot_json: &str,
    ) -> Result<(opengrid_pivot::PivotResult, Vec<opengrid_types::FieldName>), String> {
        let pivot: opengrid_pivot::PivotQuery =
            opengrid_json::from_str(pivot_json).map_err(|error| format!("pivot JSON: {error}"))?;
        let source = self
            .sources
            .get(pivot.source.as_str())
            .ok_or_else(|| format!("unknown source {:?}", pivot.source.as_str()))?;
        let schema = block_on(DataSource::schema(source)).map_err(|error| error.to_string())?;
        let rows = pivot.rows.clone();
        let validated = pivot
            .validate(
                &schema,
                &opengrid_pivot::PivotLimits::default(),
                &Limits::default(),
            )
            .map_err(|error| error.to_string())?;
        let result = block_on(opengrid_pivot::execute(source, &validated))
            .map_err(|error| error.to_string())?;
        Ok((result, rows))
    }

    /// Parses a query JSON, finds its source and validates the query against
    /// that source's schema.
    fn admit(
        &self,
        query_json: &str,
    ) -> Result<(&LocalDataSource, opengrid_query::ValidatedQuery), String> {
        let query: Query =
            opengrid_json::from_str(query_json).map_err(|error| format!("query JSON: {error}"))?;
        let source = self
            .sources
            .get(query.source.as_str())
            .ok_or_else(|| format!("unknown source {:?}", query.source.as_str()))?;

        let schema = block_on(DataSource::schema(source)).map_err(|error| error.to_string())?;
        let validated = query
            .validate(&schema, &Limits::default())
            .map_err(|error| error.to_string())?;
        Ok((source, validated))
    }
}

/// Drives a future that is ready on the first poll.
///
/// The local source never awaits — it holds a table and runs the executor
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

    /// The collator for the half of a query the planner finishes here, as
    /// [`Engine::set_collator`].
    pub fn set_collator(&self, sort_keys: CollatorFunction) {
        install_collator(sort_keys);
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
    /// a table again and the engine finishes the query over them, which
    /// includes recounting `total_count` — filtering or grouping changed it.
    pub fn finish(&self, client_query_json: &str, result_json: &str) -> Result<String, JsError> {
        self.finish_result(client_query_json, result_json)
            .map(|result| result_json_of(&result))
            .map_err(|message| JsError::new(&message))
    }

    /// [`finish`](Planner::finish) for a source that answered in the binary
    /// form: the bytes in, the finished result as bytes out.
    #[wasm_bindgen(js_name = finish_columns)]
    pub fn finish_columns_js(
        &self,
        client_query_json: &str,
        result: &[u8],
    ) -> Result<Vec<u8>, JsError> {
        self.finish_columns(client_query_json, result)
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
        let capabilities: DataSourceCapabilities = opengrid_json::from_str(capabilities_json)
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
            opengrid_json::from_str(query_json).map_err(|error| format!("query JSON: {error}"))?;
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

        let body = opengrid_json::json!({
            "mode": mode.as_str(),
            "describe": plan.describe(),
            "steps": plan.client_steps.iter().map(|step| step.as_str()).collect::<Vec<_>>(),
            "source": Query::from(&plan.source_query),
            "client": plan.client_query.as_ref().map(Query::from),
        });
        Ok(body.to_string())
    }

    /// The client half over a source answer in the binary form: decoded
    /// straight into the engine's columns, no value by value transposition.
    pub fn finish_columns(
        &self,
        client_query_json: &str,
        result: &[u8],
    ) -> Result<Vec<u8>, String> {
        let (table, _, _) =
            opengrid_columns::wire::decode_answer(result).map_err(|error| error.to_string())?;
        let query: Query = opengrid_json::from_str(client_query_json)
            .map_err(|error| format!("client query JSON: {error}"))?;
        let validated = query
            .validate(table.schema(), &Limits::default())
            .map_err(|error| error.to_string())?;
        let source = LocalDataSource::new(table);
        let answer = source.run(&validated).map_err(|error| error.to_string())?;
        Ok(encode_answer(&answer))
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

        let query: Query = opengrid_json::from_str(client_query_json)
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

/// The module's linear memory in bytes; none outside WASM.
fn linear_memory() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) * 65_536
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

/// An engine answer in the binary form: a tree's level with its part (E38),
/// anything else as a plain result.
fn encode_answer(answer: &opengrid_engine::execute::QueryResult) -> Vec<u8> {
    match &answer.tree {
        None => opengrid_columns::wire::encode_result(&answer.table, answer.total_count),
        Some(tree) => opengrid_columns::wire::encode_tree_result(
            &answer.table,
            answer.total_count,
            &opengrid_columns::wire::TreeSection {
                children: tree.children.clone(),
                matched: tree.matched.clone(),
                matches: tree.matches,
                orphans: tree.orphans,
                flat: tree
                    .flat
                    .as_ref()
                    .map(|flat| opengrid_columns::wire::FlatSection {
                        levels: flat.levels.clone(),
                        paths: flat.paths.clone(),
                        key_type: flat.key_type,
                        filtered: flat.filtered,
                    }),
                // The subtree aggregates (T7): the engine typed them, so they
                // fit their schema.
                aggregates: (!tree.aggregates.is_empty()).then(|| {
                    opengrid_columns::Table::from_values(&tree.aggregate_schema, &tree.aggregates)
                        .expect("the engine's aggregates fit their schema")
                }),
            },
        ),
    }
}

#[cfg(test)]
mod stats_tests {
    use super::Engine;

    /// Every source with its rows, columns and bytes, by name (issue #70).
    #[test]
    fn stats_name_every_source_and_what_it_holds() {
        let mut engine = Engine::new();
        let schema = r#"{"fields":[{"name":"n","type":"int64","nullable":false},{"name":"s","type":"utf8","nullable":true}]}"#;
        engine.load_csv("b", b"n,s\n1,ab\n2,cde\n", schema).unwrap();
        engine.load_csv("a", b"n,s\n7,x\n", schema).unwrap();
        let stats = engine.stats();
        let sources = stats["sources"].as_array().unwrap();
        assert_eq!(sources[0]["name"], "a");
        assert_eq!(sources[1]["name"], "b");
        assert_eq!(sources[1]["rows"], 2);
        assert_eq!(sources[1]["columns"], 2);
        // 2 ints of a byte each (issue #447); the text a dictionary of two (#451): 2 one-byte codes,
        // 3 offsets and 5 bytes of text; no NULL, no bitmap.
        assert_eq!(sources[1]["bytes"], 2 + 2 + 3 * 4 + 5);
        assert!(stats["memory"].as_u64().is_some());
    }

    /// Issue #459: `unload` frees one source and leaves the others; a query on
    /// it is then an unknown source, and the name can be loaded again.
    #[test]
    fn unload_frees_one_source_and_keeps_the_rest() {
        let mut engine = Engine::new();
        let schema = r#"{"fields":[{"name":"n","type":"int64","nullable":false}]}"#;
        engine.load_csv("a", b"n\n1\n", schema).unwrap();
        engine.load_csv("b", b"n\n2\n", schema).unwrap();
        assert!(engine.unload("a"));
        assert!(!engine.unload("a"));
        let sources = engine.stats()["sources"].as_array().unwrap().clone();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0]["name"], "b");
        let query = r#"{"source":"a","select":["n"]}"#;
        assert!(engine.execute_result(query).is_err());
        engine.load_csv("a", b"n\n3\n", schema).unwrap();
        assert_eq!(engine.execute_result(query).unwrap().row_count(), 1);
    }
}

#[cfg(test)]
mod chunked_load_tests {
    use super::Engine;

    const SCHEMA: &str = r#"{"fields":[{"name":"n","type":"int64","nullable":false},{"name":"s","type":"utf8","nullable":true}]}"#;

    /// Issue #427: a file pushed in pieces is the source `load_csv` makes of it.
    #[test]
    fn pieces_register_the_source_the_whole_file_does() {
        let file = "n,s\n1,\"a\nb\"\n2,cde\n3,ä\n";
        let mut whole = Engine::new();
        whole.load_csv("t", file.as_bytes(), SCHEMA).unwrap();
        let mut pieces = Engine::new();
        pieces.begin_csv("t", SCHEMA, file.len() as f64).unwrap();
        for byte in file.as_bytes().chunks(3) {
            pieces.push_csv(byte).unwrap();
        }
        assert!(pieces.stats()["sources"].as_array().unwrap().is_empty());
        pieces.end_csv().unwrap();
        assert_eq!(whole.stats()["sources"], pieces.stats()["sources"]);
    }

    /// A piece that cannot be read ends the file, and nothing is registered; the calls out of order
    /// are errors, not panics.
    #[test]
    fn a_broken_piece_registers_nothing_and_the_calls_keep_their_order() {
        let mut engine = Engine::new();
        assert!(engine.push_csv(b"x").is_err());
        assert!(engine.end_csv().is_err());
        engine.begin_csv("t", SCHEMA, 0.0).unwrap();
        engine.push_csv(b"n,s\n1,a\n").unwrap();
        let error = engine.push_csv(b"2\n").unwrap_err();
        assert!(error.contains("line 3"), "{error}");
        assert!(engine.end_csv().is_err());
        assert!(engine.stats()["sources"].as_array().unwrap().is_empty());
    }
}
