//! `POST /query/{source}` — the endpoint that answers queries
//! (plan/spezifikation/07-server.md), and `GET /source/{source}` next to it.
//!
//! The request body is the query AST, never SQL (§Sicherheit). What happens to
//! it, in order:
//!
//! 1. **Token.** `Authorization: Bearer …` against the configured list (E15),
//!    compared in constant time. No token, no data — and no hint about which
//!    part was wrong.
//! 2. **Source.** The name comes from the path. A body that names a different
//!    source is a validation error rather than a silent choice between the two.
//! 3. **Validation against the client schema**, which is the schema narrowed to
//!    `allowed_fields`: a forbidden column is indistinguishable from a typo.
//! 4. **The mandatory row filter** is added (E16) and the result validated
//!    against the full schema — this is what runs.
//! 5. **Execution**, with the configured timeout, and the answer in the wire form
//!    of point 23.
//!
//! `POST /export/{source}` takes the same path through steps 1–4 (see
//! `admit`) and then streams every row of the answer as a file — the
//! `export` module.
//!
//! `GET /source/{source}` answers what a *planner* needs before it can ask
//! anything: the client schema and the capabilities of the backend (plan point
//! 28). It is the same token and the same narrowing — a column outside
//! `allowed_fields` is not in the answer, because it does not exist for this
//! caller.
//!
//! Every failure leaves through the same door: a [`WireError`] with a code a
//! client can branch on, a sentence a person can read, and — where the query is
//! at fault — the JSON path `QueryError` already carries.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use opengrid_datasource::DataSourceError;
use opengrid_datasource::wire::{ErrorCode, WireError, result_to_json};
use opengrid_pivot::PivotQuery;
use opengrid_query::{Limits, Query, ValidatedQuery};
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

use crate::registry::{PrepareError, Registry, Source};

/// A failure the operator should see. The caller was told less: for a backend
/// failure a fixed sentence and the `reference`, never `detail` (issue #219).
#[derive(Clone, Debug)]
pub struct ErrorReport {
    pub code: ErrorCode,
    /// The reference the caller got in its message; none when the caller was
    /// told the cause itself or nothing about it (a refused token).
    pub reference: Option<String>,
    /// The full cause. Never a token value.
    pub detail: String,
}

pub(crate) type Hook = Arc<dyn Fn(&ErrorReport) + Send + Sync>;

/// Where failures are written down for the operator, and the source of the
/// references a caller can quote.
#[derive(Clone)]
pub struct Reporter {
    hook: Option<Hook>,
    started: u64,
    counter: Arc<AtomicU64>,
}

impl Default for Reporter {
    fn default() -> Self {
        Self {
            hook: None,
            started: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
            counter: Arc::default(),
        }
    }
}

impl Reporter {
    pub(crate) fn with_hook(hook: Hook) -> Self {
        Self {
            hook: Some(hook),
            ..Self::default()
        }
    }

    fn report(&self, code: ErrorCode, reference: Option<String>, detail: String) {
        if let Some(hook) = &self.hook {
            hook(&ErrorReport {
                code,
                reference,
                detail,
            });
        }
    }

    /// A backend failure as the caller sees it: one fixed sentence and a
    /// reference that finds `detail` in the operator's records. What a
    /// database says can carry a table, a column or a tenant's value.
    pub(crate) fn backend(&self, detail: impl Into<String>) -> WireError {
        let number = self.counter.fetch_add(1, Ordering::Relaxed);
        let reference = format!("og-{:x}-{number:04x}", self.started);
        self.report(ErrorCode::Backend, Some(reference.clone()), detail.into());
        WireError::new(
            ErrorCode::Backend,
            format!("the data source failed (reference {reference})"),
        )
    }

    /// A failure the caller is told in full: written down, not hidden.
    fn told(&self, code: ErrorCode, message: String) -> WireError {
        self.report(code, None, message.clone());
        WireError::new(code, message)
    }
}

/// Everything a request needs, shared by every handler.
pub struct AppState {
    pub registry: Registry,
    /// Token → the context that token stands for.
    pub tokens: BTreeMap<String, BTreeMap<String, String>>,
    pub max_payload_bytes: usize,
    pub timeout: Duration,
    /// Origins a browser may call from; empty means no CORS headers.
    pub allowed_origins: Vec<String>,
    /// The most rows one export may have (issue #2).
    pub max_export_rows: u64,
    /// The most rows a tree's scope may have (see `ServerBuilder::max_tree_rows`).
    pub max_tree_rows: u64,
    /// The most rows an Excel export may have.
    pub max_xlsx_rows: u64,
    /// The exports that may run at once, and the places they take.
    pub max_concurrent_exports: usize,
    pub exports: Arc<Semaphore>,
    /// Where the failures a caller is not told in full are written down.
    pub reporter: Reporter,
}

impl AppState {
    /// The shared state from its parts — what the builder and the
    /// configuration both come down to.
    pub(crate) fn from_parts(
        registry: Registry,
        tokens: BTreeMap<String, BTreeMap<String, String>>,
        settings: &crate::server::Settings,
    ) -> Self {
        let concurrent = settings
            .max_concurrent_exports
            .unwrap_or_else(|| registry.default_concurrent_exports());
        Self {
            registry,
            tokens,
            max_payload_bytes: settings.max_payload_bytes,
            timeout: settings.timeout,
            allowed_origins: settings.allowed_origins.clone(),
            max_export_rows: settings.max_export_rows,
            max_tree_rows: settings.max_tree_rows,
            max_xlsx_rows: settings.max_xlsx_rows,
            max_concurrent_exports: concurrent,
            exports: Arc::new(Semaphore::new(concurrent)),
            reporter: Reporter::default(),
        }
    }
}

/// The router: one endpoint, one state.
///
/// `allowed_origins` adds CORS for exactly those origins — a browser page served
/// from somewhere else is the normal case for a gateway. Empty means no CORS
/// headers at all, which is the safe default: with a bearer token, a wildcard
/// origin would mean every site the user visits can spend that token.
pub fn router(state: Arc<AppState>) -> Router {
    let origins = state.allowed_origins.clone();
    let max_payload_bytes = state.max_payload_bytes;
    let mut router = Router::new()
        .route("/query/{source}", post(query))
        .route("/pivot/{source}", post(pivot))
        .route("/source/{source}", get(describe))
        .route("/export/{source}", post(crate::export::export))
        // axum buffers a body under its own 2 MiB limit before a handler runs,
        // so a configured size above that was capped and an unauthenticated
        // client could make the server buffer 2 MiB a request (issue #222).
        .layer(DefaultBodyLimit::max(max_payload_bytes))
        // Every answer holds one tenant's rows: nothing may keep it, and a
        // browser must not guess another type for it.
        .layer(axum::middleware::map_response(keep_private))
        .with_state(state);

    if !origins.is_empty() {
        let parsed: Vec<HeaderValue> = origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        router = router.layer(
            CorsLayer::new()
                .allow_origin(parsed)
                .allow_methods([Method::GET, Method::POST])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                // A page reads the export's file name and its row count; a
                // browser hides every other header of a cross-origin answer.
                .expose_headers([
                    header::CONTENT_DISPOSITION,
                    header::HeaderName::from_static(crate::export::ROWS_HEADER),
                ]),
        );
    }
    router
}

/// `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every answer.
async fn keep_private(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// The request body, or the wire error for one that is over the payload limit
/// (the limit is the router's `DefaultBodyLimit`) or could not be read.
pub(crate) fn payload(
    body: Result<Bytes, BytesRejection>,
    state: &AppState,
) -> Result<Bytes, Failure> {
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            WireError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "request body is over the limit of {} bytes",
                    state.max_payload_bytes
                ),
            )
            .into()
        } else {
            WireError::new(
                ErrorCode::Malformed,
                format!("request body: {}", rejection.body_text()),
            )
            .into()
        }
    })
}

/// An error on its way out: the wire form plus the status that goes with it.
pub(crate) struct Failure(WireError);

impl From<WireError> for Failure {
    fn from(error: WireError) -> Self {
        Self(error)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::Malformed => StatusCode::BAD_REQUEST,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::UnknownSource => StatusCode::NOT_FOUND,
            ErrorCode::Validation => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::LimitExceeded => StatusCode::PAYLOAD_TOO_LARGE,
            // Trying again later helps: an export turned away while others run.
            ErrorCode::Busy => StatusCode::SERVICE_UNAVAILABLE,
            // The gateway is fine; the source behind it is not.
            ErrorCode::Backend => StatusCode::BAD_GATEWAY,
            // The request is fine and the gateway too; the data breaks a rule.
            ErrorCode::Data => StatusCode::UNPROCESSABLE_ENTITY,
        };
        (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            self.0.to_json(),
        )
            .into_response()
    }
}

async fn query(
    State(state): State<Arc<AppState>>,
    Path(source_name): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, Failure> {
    let body = payload(body, &state)?;
    let (source, validated) = admit(
        &state,
        &source_name,
        &headers,
        &body,
        &state.registry.limits,
        Some(state.registry.limits.max_limit),
    )?;

    // A tree goes to a connector that answers trees as it is, and to any
    // other through its tree path (plan point 122).
    let tree = validated.tree.is_some();
    if tree {
        within_tree_bound(&state, &source, &validated).await?;
    }
    let answering = async {
        if tree && !source.data.capabilities().tree {
            source.data.tree(&validated).await
        } else {
            source.data.execute(validated.clone()).await
        }
    };
    let executed = tokio::time::timeout(state.timeout, answering)
        .await
        .map_err(|_| {
            WireError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "the query took longer than {} ms",
                    state.timeout.as_millis()
                ),
            )
        })?;

    let result = executed.map_err(|error| source_failed(error, &state.reporter))?;

    if wants_columns(&headers) {
        let table = opengrid_columns::Table::from_values(&result.schema, &result.columns)
            .map_err(|message| state.reporter.backend(message))?;
        // A tree's level carries its part in the binary form too (E38), and
        // its subtree aggregates (T7).
        return Ok(columns_response(match &result.tree {
            None => opengrid_columns::wire::encode_result(&table, result.total_count),
            Some(tree) => {
                let aggregates = match tree.aggregates.is_empty() {
                    true => None,
                    false => Some(
                        opengrid_columns::Table::from_values(
                            &tree.aggregate_schema,
                            &tree.aggregates,
                        )
                        .map_err(|message| state.reporter.backend(message))?,
                    ),
                };
                opengrid_columns::wire::encode_tree_result(
                    &table,
                    result.total_count,
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
                        aggregates,
                    },
                )
            }
        }));
    }
    Ok(json_response(result_to_json(&result)))
}

/// Whether the client asked for the binary result form (E35): its media type
/// in `Accept`, with any parameters, and not refused with `q=0`. Everything
/// else — no `Accept`, `*/*`, `application/json` — gets JSON, so curl and
/// other clients see what they always saw.
fn wants_columns(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|entry| {
            let mut parts = entry.split(';').map(str::trim);
            let media = parts.next().unwrap_or_default();
            let refused = parts.any(|parameter| {
                parameter
                    .strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
                    .is_some_and(|q| q == 0.0)
            });
            media.eq_ignore_ascii_case(opengrid_columns::wire::MEDIA_TYPE) && !refused
        })
}

/// A successful answer in JSON. `Vary: Accept`, because the same request can
/// be answered in two forms and a cache must not hand one out for the other.
fn json_response(body: String) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::VARY, "Accept"),
        ],
        body,
    )
        .into_response()
}

/// A successful answer in the binary form.
fn columns_response(body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, opengrid_columns::wire::MEDIA_TYPE),
            (header::VARY, "Accept"),
        ],
        body,
    )
        .into_response()
}

/// Everything a query goes through before it may run: the token, the payload
/// limit, the source from the path, a body that is a query for that source,
/// validation against the client schema, the mandatory row filter (E16) and
/// validation against the full schema. `POST /query` and `POST /export` both
/// come through here (issue #2), so an export cannot be a way around a rule a
/// query keeps; they differ only in `limits` — an export's `max_limit` is
/// `max_export_rows` — and in `default_limit`: a query that names no `limit`
/// gets it (issue #216), so a page is never more than `max_limit` rows. An
/// export passes `None` and reads up to its row bound, which it counts first.
pub(crate) fn admit(
    state: &AppState,
    source_name: &str,
    headers: &HeaderMap,
    body: &Bytes,
    limits: &Limits,
    default_limit: Option<u64>,
) -> Result<(Arc<Source>, ValidatedQuery), Failure> {
    let context = authorize(state, headers)?;

    let source = state.registry.get(source_name).ok_or_else(|| {
        WireError::new(
            ErrorCode::UnknownSource,
            format!("unknown source {source_name:?}"),
        )
    })?;

    let mut query: Query = request_body(body)?;
    if query.limit.is_none() {
        query.limit = default_limit;
    }
    if query.source.as_str() != source_name {
        return Err(WireError::at(
            ErrorCode::Validation,
            format!(
                "the body names source {:?}, the path names {source_name:?}",
                query.source.as_str()
            ),
            "source",
        )
        .into());
    }

    let validated = source
        .prepare(query, limits, context)
        .map_err(|error| prepare_failed(error, &state.reporter))?;
    Ok((source, validated))
}

/// A tree is built from all the rows of its scope - by the engine, for a
/// source that does not answer trees itself, and for every flat tree. Counted
/// first, so a scope past `max_tree_rows` is refused before one row is read
/// (issue #218).
pub(crate) async fn within_tree_bound(
    state: &AppState,
    source: &Source,
    query: &ValidatedQuery,
) -> Result<(), Failure> {
    let Some(tree) = &query.tree else {
        return Ok(());
    };
    if source.data.capabilities().tree && !tree.flat {
        // The database builds the level; the engine does not hold the rows.
        return Ok(());
    }
    let alias = opengrid_types::FieldName::new("scope_rows").expect("a valid name");
    let count = ValidatedQuery {
        source: query.source.clone(),
        select: vec![alias.clone()],
        filter: tree.scope.clone(),
        group: Vec::new(),
        aggregate: vec![opengrid_query::Aggregate {
            field: None,
            function: opengrid_query::AggregateFn::Count,
            alias: alias.clone(),
        }],
        sort: Vec::new(),
        offset: None,
        limit: None,
        tree: None,
        output_schema: opengrid_types::Schema::new(vec![opengrid_types::Field::required(
            alias,
            opengrid_types::DataType::Int64,
        )]),
    };
    let counted = tokio::time::timeout(state.timeout, source.data.execute(count))
        .await
        .map_err(|_| {
            WireError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "counting the tree took longer than {} ms",
                    state.timeout.as_millis()
                ),
            )
        })?
        .map_err(|error| source_failed(error, &state.reporter))?;
    let rows = match counted.columns.first().and_then(|column| column.first()) {
        Some(opengrid_types::Value::Int64(rows)) => u64::try_from(*rows).unwrap_or(0),
        _ => 0,
    };
    if rows > state.max_tree_rows {
        return Err(WireError::new(
            ErrorCode::LimitExceeded,
            format!(
                "the tree has {rows} rows, more than the {} a tree may have (max_tree_rows); \
                 narrow its scope",
                state.max_tree_rows
            ),
        )
        .into());
    }
    Ok(())
}

/// A source that could not answer, as the wire error it becomes.
///
/// A backend failure is a fixed sentence and a reference (the cause stays with
/// the operator: it can name a table, a column or a tenant's value). A
/// `LimitExceeded` is the caller's to act on. A `Data` error (a tree with a
/// cycle) is told in full too: it names the rows of the caller's own data, and
/// travels as `data` (issue #219), not as an outage.
pub(crate) fn source_failed(error: DataSourceError, reporter: &Reporter) -> WireError {
    match error {
        DataSourceError::NoData => {
            WireError::new(ErrorCode::Backend, "the data source holds no data")
        }
        DataSourceError::Backend { message } => reporter.backend(message),
        DataSourceError::LimitExceeded { message } => {
            WireError::new(ErrorCode::LimitExceeded, message)
        }
        DataSourceError::Data { message } => reporter.told(ErrorCode::Data, message),
    }
}

/// A failed `prepare`, as the wire error it becomes.
fn prepare_failed(error: PrepareError, reporter: &Reporter) -> Failure {
    match error {
        PrepareError::Validation(error) => match error.path() {
            Some(path) => WireError::at(ErrorCode::Validation, error.to_string(), path),
            None => WireError::new(ErrorCode::Validation, error.to_string()),
        },
        // A pivot that does not fit — too wide, too long, no measure. The
        // caller can act on it, so it is a validation error and not a 502.
        PrepareError::Pivot(error) => WireError::new(ErrorCode::Validation, error.to_string()),
        // A configuration that does not fit the caller: never fall back to
        // running without the mandatory filter.
        PrepareError::Context(message) => reporter.backend(message),
    }
    .into()
}

/// `POST /pivot/{source}` — a whole pivot in one request (plan point 53).
///
/// The same door as `/query`: token, payload limit, the source from the path,
/// validation against the client schema, then the mandatory row filter (E16) —
/// which here lands on **every** grouping set, the grand total included.
///
/// The answer is the pivot wire form: the ordinary result of E17 under
/// `result`, plus the level of each row and what each generated column stands
/// for. Without those two a client could not tell a subtotal from a real NULL
/// group (S10/P2).
///
/// `?format=xlsx` answers the same pivot as a workbook instead (issue #331,
/// `crate::export`): everything up to the answer is this door, unchanged.
async fn pivot(
    State(state): State<Arc<AppState>>,
    Path(source_name): Path<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, Failure> {
    let body = payload(body, &state)?;
    let context = authorize(&state, &headers)?;

    let source = state.registry.get(&source_name).ok_or_else(|| {
        WireError::new(
            ErrorCode::UnknownSource,
            format!("unknown source {source_name:?}"),
        )
    })?;

    let query: PivotQuery = request_body(&body)?;
    if query.source.as_str() != source_name {
        return Err(WireError::at(
            ErrorCode::Validation,
            format!(
                "the body names source {:?}, the path names {source_name:?}",
                query.source.as_str()
            ),
            "source",
        )
        .into());
    }

    let rows = query.rows.clone();
    let prepared = source
        .prepare_pivot(
            query,
            &state.registry.limits,
            &state.registry.pivot_limits,
            context,
        )
        .map_err(|error| prepare_failed(error, &state.reporter))?;
    // A workbook is an export: its parameters and its place among the running
    // exports are settled before the source is asked anything.
    let workbook = match crate::export::pivot_words(&uri)? {
        Some(words) => Some((words, crate::export::place(&state)?)),
        None => None,
    };

    let executed = tokio::time::timeout(state.timeout, source.data.pivot(&prepared))
        .await
        .map_err(|_| {
            WireError::new(
                ErrorCode::LimitExceeded,
                format!(
                    "the pivot took longer than {} ms",
                    state.timeout.as_millis()
                ),
            )
        })?;

    let result = executed.map_err(|error| source_failed(error, &state.reporter))?;

    if let Some((words, permit)) = workbook {
        return crate::export::pivot_workbook(&state, &source_name, result, words, permit).await;
    }
    if wants_columns(&headers) {
        return Ok(columns_response(opengrid_pivot::pivot_to_bytes(
            &result, &rows,
        )));
    }
    Ok(json_response(opengrid_pivot::pivot_to_json(&result, &rows)))
}

/// `GET /source/{source}` — the schema and capabilities of one source.
///
/// This is what a browser-side planner needs to split a query: the schema to
/// validate against, the capabilities to decide what may be pushed. The schema
/// is the **client** schema, narrowed by `allowed_fields` — the same view the
/// query endpoint validates against, so what is describable is what is askable.
/// The mandatory row filter (E16) is not in the answer; it is the server's
/// business and may name columns the caller never sees.
async fn describe(
    State(state): State<Arc<AppState>>,
    Path(source_name): Path<String>,
    headers: HeaderMap,
) -> Result<Response, Failure> {
    authorize(&state, &headers)?;

    let source = state.registry.get(&source_name).ok_or_else(|| {
        WireError::new(
            ErrorCode::UnknownSource,
            format!("unknown source {source_name:?}"),
        )
    })?;

    let body = opengrid_json::json!({
        "name": source.name,
        "schema": source.client_schema,
        "capabilities": source.capabilities(),
        // A planner needs the bounds before it asks, not after it is refused.
        "pivot_limits": {
            "max_column_dimensions": state.registry.pivot_limits.max_column_dimensions,
            "max_columns": state.registry.pivot_limits.max_columns,
            "max_rows": state.registry.pivot_limits.max_rows,
            "max_cells": state.registry.pivot_limits.max_cells,
        },
    });

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response())
}

/// A request body read as `T`: UTF-8 JSON, or a `malformed` error that says
/// what is wrong with it.
fn request_body<T: opengrid_json::FromJson>(body: &[u8]) -> Result<T, WireError> {
    let malformed = |message: String| WireError::new(ErrorCode::Malformed, message);
    let text =
        std::str::from_utf8(body).map_err(|_| malformed("request body: not UTF-8".to_owned()))?;
    opengrid_json::from_str(text).map_err(|error| malformed(format!("request body: {error}")))
}

/// Checks the bearer token and answers with the context it stands for.
///
/// One failure for every way of getting it wrong — missing header, wrong scheme,
/// unknown token — because telling them apart tells an attacker which half to
/// keep trying.
fn authorize<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
) -> Result<&'a BTreeMap<String, String>, Failure> {
    // The caller is told one thing; the operator's records say which it was.
    let refused = |why: &str| -> Failure {
        state
            .reporter
            .report(ErrorCode::Unauthorized, None, why.to_owned());
        WireError::new(ErrorCode::Unauthorized, "a valid bearer token is required").into()
    };

    let presented = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| refused("no Authorization header"))?
        .to_str()
        .ok()
        .and_then(bearer_token)
        .ok_or_else(|| refused("the Authorization header is not a Bearer token"))?;

    state
        .tokens
        .iter()
        .find(|(configured, _)| constant_time_eq(configured, presented))
        .map(|(_, context)| context)
        .ok_or_else(|| refused("an unknown token"))
}

/// The token after the `Bearer` scheme, which RFC 7235 matches without regard
/// to case.
fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("Bearer").then_some(token)
}

/// Compares two secrets without leaking where they differ.
///
/// The length is not secret (it leaks through the request anyway); the content
/// is, so every byte is looked at.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_is_still_an_equality() {
        assert!(constant_time_eq("s3cret", "s3cret"));
        assert!(!constant_time_eq("s3cret", "s3crey"));
        assert!(!constant_time_eq("s3cret", "s3cre"));
        assert!(!constant_time_eq("", "x"));
        assert!(constant_time_eq("", ""));
    }
}
