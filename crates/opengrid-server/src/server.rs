//! The server as a library (issue #45, E36): the application builds it and
//! hands it its sources.
//!
//! ```no_run
//! # async fn run(orders: impl opengrid_connector::Connector + 'static) -> Result<(), Box<dyn std::error::Error>> {
//! use opengrid_server::{RowFilter, Server, SourcePolicy};
//!
//! let server = Server::builder()
//!     .source("orders", orders, SourcePolicy {
//!         allowed_fields: vec!["id".into(), "customer".into(), "amount".into()],
//!         row_filter: Some(RowFilter::new("tenant_id", "eq", ":tenant")),
//!     })
//!     .token(std::env::var("ORDERS_TOKEN")?, [("tenant", "acme")])
//!     .build()
//!     .await?;
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:8081").await?;
//! axum::serve(listener, server.router()).await?;
//! # Ok(()) }
//! ```
//!
//! Every rule of the gateway holds for a source handed in this way exactly as
//! for one from the configuration file: the token, the narrowed schema, the
//! mandatory row filter, the limits. The connector answers; the server decides
//! what it gets asked.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use opengrid_connector::Connector;
use opengrid_pivot::PivotLimits;
use opengrid_query::Limits;

use crate::api::AppState;
use crate::registry::{Registry, RegistryError, Source, SourcePolicy};

/// A built server: its router, ready to serve.
pub struct Server {
    state: Arc<AppState>,
}

impl Server {
    /// Starts a server description with the defaults of the configuration
    /// file: 64 KiB bodies, a 10 s timeout, no CORS, a million rows per export.
    pub fn builder() -> ServerBuilder {
        ServerBuilder::default()
    }

    /// The router, to hand to `axum::serve` or to nest in an application's own.
    pub fn router(&self) -> Router {
        crate::api::router(Arc::clone(&self.state))
    }

    /// The shared state — the sources and the settings the server runs with.
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }
}

/// The settings that are not sources or tokens.
#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub max_payload_bytes: usize,
    pub timeout: Duration,
    pub allowed_origins: Vec<String>,
    pub max_export_rows: u64,
    pub max_concurrent_exports: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_payload_bytes: 64 * 1024,
            timeout: Duration::from_secs(10),
            allowed_origins: Vec::new(),
            max_export_rows: 1_000_000,
            max_concurrent_exports: None,
        }
    }
}

/// What [`Server::builder`] collects.
#[derive(Default)]
pub struct ServerBuilder {
    sources: Vec<(String, Arc<dyn Connector>, SourcePolicy)>,
    tokens: Vec<(String, BTreeMap<String, String>)>,
    settings: Settings,
    limits: Limits,
    pivot_limits: PivotLimits,
}

impl ServerBuilder {
    /// A source under `name`, answered by `connector`, with what the server
    /// enforces for it.
    pub fn source(
        self,
        name: impl Into<String>,
        connector: impl Connector + 'static,
        policy: SourcePolicy,
    ) -> Self {
        self.shared_source(name, Arc::new(connector), policy)
    }

    /// The same, for a connector the application keeps a handle to.
    pub fn shared_source(
        mut self,
        name: impl Into<String>,
        connector: Arc<dyn Connector>,
        policy: SourcePolicy,
    ) -> Self {
        self.sources.push((name.into(), connector, policy));
        self
    }

    /// A bearer token and the context it stands for — the values a row filter
    /// names as `:key` (E15, E16). A server without tokens refuses every request.
    pub fn token<K: Into<String>, V: Into<String>>(
        mut self,
        value: impl Into<String>,
        context: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        let context = context
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self.tokens.push((value.into(), context));
        self
    }

    /// Largest request body, in bytes.
    pub fn max_payload_bytes(mut self, bytes: usize) -> Self {
        self.settings.max_payload_bytes = bytes;
        self
    }

    /// How long a query may take; for an export, each step of it.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.settings.timeout = timeout;
        self
    }

    /// An origin a browser may call from. None means no CORS headers at all;
    /// never `*` — a wildcard plus a bearer token hands the token to every site.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.settings.allowed_origins.push(origin.into());
        self
    }

    /// The most rows one export may have.
    pub fn max_export_rows(mut self, rows: u64) -> Self {
        self.settings.max_export_rows = rows;
        self
    }

    /// How many exports may run at once. Unset: the smallest bound a
    /// connector names, or the machine's parallelism.
    pub fn max_concurrent_exports(mut self, exports: usize) -> Self {
        self.settings.max_concurrent_exports = Some(exports);
        self
    }

    /// The query limits every source is validated with.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The bounds a pivot must stay inside.
    pub fn pivot_limits(mut self, limits: PivotLimits) -> Self {
        self.pivot_limits = limits;
        self
    }

    /// Asks every connector for its schema and checks everything that can be
    /// wrong before the first request: no sources, a name twice, an empty
    /// token, an allowed field or a row filter the schema does not have.
    pub async fn build(self) -> Result<Server, RegistryError> {
        if self.sources.is_empty() {
            return Err(RegistryError::new(
                "no sources: a server needs at least one",
            ));
        }
        if self.settings.max_concurrent_exports == Some(0) {
            return Err(RegistryError::new(
                "max_concurrent_exports is 0, which would refuse every export",
            ));
        }
        if self.tokens.iter().any(|(value, _)| value.trim().is_empty()) {
            return Err(RegistryError::new("a token value is empty"));
        }

        let mut sources = Vec::with_capacity(self.sources.len());
        for (name, connector, policy) in self.sources {
            if sources.iter().any(|source: &Source| source.name == name) {
                return Err(RegistryError::new(format!(
                    "source {name:?} is handed in twice"
                )));
            }
            let schema = connector.schema().await.map_err(|error| {
                RegistryError::new(format!("source {name:?}: its schema: {error}"))
            })?;
            sources.push(Source::new(&name, connector, schema, policy)?);
        }

        let registry = Registry::new(sources, self.limits, self.pivot_limits);
        let state =
            AppState::from_parts(registry, self.tokens.into_iter().collect(), &self.settings);
        Ok(Server {
            state: Arc::new(state),
        })
    }
}
