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
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use opengrid_datasource::DataSourceError;
use opengrid_datasource::wire::{ErrorCode, WireError, result_to_json};
use opengrid_query::Query;
use tower_http::cors::CorsLayer;

use crate::config::Config;
use crate::registry::{PrepareError, Registry};

/// Everything a request needs, shared by every handler.
pub struct AppState {
    pub registry: Registry,
    /// Token → the context that token stands for.
    pub tokens: BTreeMap<String, BTreeMap<String, String>>,
    pub max_payload_bytes: usize,
    pub timeout: Duration,
    /// Origins a browser may call from; empty means no CORS headers.
    pub allowed_origins: Vec<String>,
}

impl AppState {
    /// Builds the shared state from a checked configuration and registry.
    pub fn new(config: &Config, registry: Registry) -> Self {
        let tokens = config
            .tokens
            .iter()
            .map(|token| (token.value.clone(), token.context.clone()))
            .collect();
        Self {
            registry,
            tokens,
            max_payload_bytes: config.server.max_payload_bytes,
            timeout: Duration::from_millis(config.server.timeout_ms),
            allowed_origins: config.server.allowed_origins.clone(),
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
    let mut router = Router::new()
        .route("/query/{source}", post(query))
        .route("/source/{source}", get(describe))
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
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
        );
    }
    router
}

/// An error on its way out: the wire form plus the status that goes with it.
struct Failure(WireError);

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
            // The gateway is fine; the source behind it is not.
            ErrorCode::Backend => StatusCode::BAD_GATEWAY,
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
    body: Bytes,
) -> Result<Response, Failure> {
    let context = authorize(&state, &headers)?;

    if body.len() > state.max_payload_bytes {
        return Err(WireError::new(
            ErrorCode::LimitExceeded,
            format!(
                "request body is {} bytes, the limit is {}",
                body.len(),
                state.max_payload_bytes
            ),
        )
        .into());
    }

    let source = state.registry.get(&source_name).ok_or_else(|| {
        WireError::new(
            ErrorCode::UnknownSource,
            format!("unknown source {source_name:?}"),
        )
    })?;

    let query: Query = serde_json::from_slice(&body)
        .map_err(|error| WireError::new(ErrorCode::Malformed, format!("request body: {error}")))?;
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
        .prepare(query, &state.registry.limits, context)
        .map_err(|error| match error {
            PrepareError::Validation(error) => match error.path() {
                Some(path) => WireError::at(ErrorCode::Validation, error.to_string(), path),
                None => WireError::new(ErrorCode::Validation, error.to_string()),
            },
            // A configuration that does not fit the caller: never fall back to
            // running without the mandatory filter.
            PrepareError::Context(message) => WireError::new(ErrorCode::Backend, message),
        })?;

    let executed = tokio::time::timeout(state.timeout, source.data.execute(validated))
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

    let result = executed.map_err(|error| match error {
        DataSourceError::NoData => {
            WireError::new(ErrorCode::Backend, "the data source holds no data")
        }
        DataSourceError::Backend { message } => WireError::new(ErrorCode::Backend, message),
    })?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        result_to_json(&result),
    )
        .into_response())
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

    let body = serde_json::json!({
        "name": source.name,
        "schema": source.client_schema,
        "capabilities": source.data.capabilities(),
    });

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response())
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
    let refused = || -> Failure {
        WireError::new(ErrorCode::Unauthorized, "a valid bearer token is required").into()
    };

    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(refused)?;

    state
        .tokens
        .iter()
        .find(|(configured, _)| constant_time_eq(configured, presented))
        .map(|(_, context)| context)
        .ok_or_else(refused)
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
