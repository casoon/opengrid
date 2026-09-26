//! What the server knows about its sources, built once at startup.
//!
//! A registered source carries four things the endpoint needs on every request:
//! the data itself, the schema a **client** may see, the full schema the query
//! actually runs against, and the mandatory row filter (E16).
//!
//! # Why two schemas
//!
//! `allowed_fields` is enforced by *narrowing the schema* the client's query is
//! validated against, instead of walking the query afterwards looking for
//! forbidden names. A field that is not allowed then simply does not exist:
//! validation rejects it with the same error and the same JSON path as a typo,
//! and there is no path through the AST — a filter, a sort key, an aggregate, a
//! group key — that could be forgotten in a hand-written check.
//!
//! The mandatory filter, though, usually names a column the client must *not*
//! see (`tenant_id`). So the query is validated twice: once against the client
//! schema, to decide whether the caller may ask this, and once against the full
//! schema after the filter has been added, to produce what actually runs.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use opengrid_arrow_engine::datasource::{LocalDataSource, LocalPieces};
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_datasource::SendDataSource;
use opengrid_datasource_postgres::{ExportCanceller, PostgresDataSource, PostgresExport};
use opengrid_pivot::{PivotError, PivotLimits, PivotQuery, PivotResult, ValidatedPivotQuery};
use opengrid_query::{CmpOp, FilterExpr, Limits, Query, ValidatedQuery};
use opengrid_types::{FieldName, Schema};

use crate::config::{Config, RowFilterConfig, SourceConfig};

/// One source the server will answer for.
pub struct Source {
    pub name: String,
    /// The schema a client's query is validated against — `allowed_fields` only.
    pub client_schema: Schema,
    /// Every column, including the ones only the row filter may name.
    pub full_schema: Schema,
    pub row_filter: Option<RowFilterConfig>,
    pub data: Backend,
}

/// What actually answers a query.
///
/// Both are `SendDataSource`; the endpoint does not care which one it has, which
/// is the point of the trait. A third one (MySQL, Mongo) would be another
/// variant and nothing else would change.
pub enum Backend {
    /// The local engine over a CSV file (point 24) — the gateway is testable
    /// without a database.
    LocalCsv(LocalDataSource),
    /// A PostgreSQL table (point 26).
    Postgres(PostgresDataSource),
}

impl Backend {
    /// What this backend can answer by itself — the planner's input.
    pub fn capabilities(&self) -> opengrid_datasource::DataSourceCapabilities {
        match self {
            Backend::LocalCsv(source) => SendDataSource::capabilities(source),
            Backend::Postgres(source) => SendDataSource::capabilities(source),
        }
    }

    /// Runs a whole pivot, pushed down where the backend can do it.
    ///
    /// PostgreSQL answers one `GROUPING SETS` statement (point 31); anything
    /// else gets the generic path, which is `n+1` ordinary queries against the
    /// very same trait. Same answer either way — that is what the differential
    /// test in `opengrid-datasource-postgres` is for.
    pub async fn execute_pivot(
        &self,
        pivot: &ValidatedPivotQuery,
    ) -> Result<PivotResult, opengrid_datasource::DataSourceError> {
        match self {
            Backend::LocalCsv(source) => {
                opengrid_pivot::execute(source, pivot)
                    .await
                    .map_err(|error| opengrid_datasource::DataSourceError::Backend {
                        message: error.to_string(),
                    })
            }
            Backend::Postgres(source) => source.execute_pivot(pivot).await,
        }
    }

    /// Starts an export of `query` (issue #2): the rows of the whole answer,
    /// handed out a piece at a time. Nothing heavy has run when this returns —
    /// [`ExportRows::count`] is the first step that can take long, so a caller
    /// holds the [`ExportRows::canceller`] before it.
    ///
    /// `idle_backstop` bounds how long PostgreSQL lets the export's transaction
    /// sit idle; the local engine has no transaction.
    pub(crate) async fn export(
        &self,
        query: &ValidatedQuery,
        idle_backstop: Duration,
    ) -> Result<ExportRows, opengrid_datasource::DataSourceError> {
        match self {
            Backend::LocalCsv(source) => Ok(ExportRows::Local(source.pieces(query)?)),
            Backend::Postgres(source) => Ok(ExportRows::Postgres(Box::new(
                source.export(query, idle_backstop).await?,
            ))),
        }
    }

    /// Runs a query against whichever backend this is.
    pub async fn execute(
        &self,
        query: ValidatedQuery,
    ) -> Result<opengrid_datasource::QueryResult, opengrid_datasource::DataSourceError> {
        match self {
            Backend::LocalCsv(source) => SendDataSource::execute(source, query).await,
            Backend::Postgres(source) => SendDataSource::execute(source, query).await,
        }
    }
}

/// An export in progress, whichever backend runs it.
///
/// The local engine has answered by the time this exists — its data is in
/// memory, and one run beats paging it — and hands out slices of that answer.
/// PostgreSQL counts and then reads through a cursor in one snapshot
/// ([`PostgresExport`]).
pub(crate) enum ExportRows {
    Local(LocalPieces),
    /// Boxed: a connection and two compiled statements, once per export.
    Postgres(Box<PostgresExport>),
}

impl ExportRows {
    /// The rows the export will have, before the first of them is read.
    pub(crate) async fn count(&mut self) -> Result<u64, opengrid_datasource::DataSourceError> {
        match self {
            ExportRows::Local(pieces) => Ok(pieces.rows()),
            ExportRows::Postgres(export) => export.count().await,
        }
    }

    /// The next piece of at most `rows` rows; a shorter one is the last.
    pub(crate) async fn next_piece(
        &mut self,
        rows: usize,
    ) -> Result<opengrid_datasource::QueryResult, opengrid_datasource::DataSourceError> {
        match self {
            ExportRows::Local(pieces) => pieces.next_piece(rows),
            ExportRows::Postgres(export) => export.next_piece(rows).await,
        }
    }

    /// What stops a statement that is still running, where there is one to
    /// stop: the local engine answers synchronously and has none.
    pub(crate) fn canceller(&self) -> Option<ExportCanceller> {
        match self {
            ExportRows::Local(_) => None,
            ExportRows::Postgres(export) => Some(export.canceller()),
        }
    }

    /// Ends an export cleanly before its last piece — refused, or failed with
    /// a status — so a PostgreSQL connection goes back to the pool.
    pub(crate) async fn close(self) {
        match self {
            ExportRows::Local(_) => {}
            ExportRows::Postgres(export) => export.close().await,
        }
    }
}

/// The sources, by name.
pub struct Registry {
    sources: BTreeMap<String, Arc<Source>>,
    pub limits: Limits,
    /// The bounds a pivot must stay inside (plan point 30).
    pub pivot_limits: PivotLimits,
}

/// Why a registry could not be built. Always a startup failure.
#[derive(Debug)]
pub struct RegistryError {
    message: String,
}

impl RegistryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RegistryError {}

impl Registry {
    /// Loads every configured source. Any problem stops the server.
    pub fn build(config: &Config, base: &Path) -> Result<Self, RegistryError> {
        let mut sources = BTreeMap::new();
        for source in &config.datasources {
            let loaded = load_source(source, base)?;
            sources.insert(source.name.clone(), Arc::new(loaded));
        }

        let mut limits = Limits::default();
        if let Some(max_limit) = config.server.max_limit {
            limits.max_limit = max_limit;
        }
        if let Some(max_depth) = config.server.max_depth {
            limits.max_depth = max_depth;
        }
        let mut pivot_limits = PivotLimits::default();
        if let Some(max_columns) = config.server.max_pivot_columns {
            pivot_limits.max_columns = max_columns;
        }
        if let Some(max_rows) = config.server.max_pivot_rows {
            pivot_limits.max_rows = max_rows;
        }
        Ok(Self {
            sources,
            limits,
            pivot_limits,
        })
    }

    /// `max_concurrent_exports` when the configuration leaves it out: half the
    /// smallest PostgreSQL pool, at least 1. Every export holds one pooled
    /// connection for as long as its client downloads, and the bound is
    /// server-wide, so even if all of them hit the same source, half of its
    /// pool stays for `/query` and `/pivot`. Without a PostgreSQL source, the
    /// same number the default pool would give — the machine's parallelism —
    /// since the local engine holds a whole answer per export instead.
    pub fn default_concurrent_exports(&self) -> usize {
        self.sources
            .values()
            .filter_map(|source| match &source.data {
                Backend::Postgres(postgres) => Some(postgres.pool_size() / 2),
                Backend::LocalCsv(_) => None,
            })
            .min()
            .unwrap_or_else(|| {
                std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
            })
            .max(1)
    }

    /// The source of that name, if it is configured.
    pub fn get(&self, name: &str) -> Option<Arc<Source>> {
        self.sources.get(name).cloned()
    }

    /// The configured names, for diagnostics at startup.
    pub fn names(&self) -> Vec<&str> {
        self.sources.keys().map(String::as_str).collect()
    }
}

fn load_source(config: &SourceConfig, base: &Path) -> Result<Source, RegistryError> {
    if !matches!(config.kind.as_str(), "local-csv" | "postgres") {
        return Err(RegistryError::new(format!(
            "datasource {:?}: type {:?} is not supported (\"local-csv\" or \"postgres\")",
            config.name, config.kind
        )));
    }

    let schema_path = base.join(&config.schema);
    let schema_json = std::fs::read_to_string(&schema_path).map_err(|error| {
        RegistryError::new(format!(
            "datasource {:?}: {}: {error}",
            config.name,
            schema_path.display()
        ))
    })?;
    let full_schema: Schema = serde_json::from_str(&schema_json).map_err(|error| {
        RegistryError::new(format!(
            "datasource {:?}: {}: {error}",
            config.name,
            schema_path.display()
        ))
    })?;
    // A broken derivation is a startup failure like every other configuration
    // mistake (plan point 54): a gateway that came up with one would serve a
    // column full of NULL and nobody would notice.
    full_schema
        .check()
        .map_err(|error| RegistryError::new(format!("datasource {:?}: {error}", config.name)))?;

    let data = match config.kind.as_str() {
        "postgres" => {
            let url = config.connection.as_deref().ok_or_else(|| {
                RegistryError::new(format!(
                    "datasource {:?}: a postgres source needs a `connection`",
                    config.name
                ))
            })?;
            let url = crate::config::interpolate_public(url).map_err(|error| {
                RegistryError::new(format!("datasource {:?}: {error}", config.name))
            })?;
            let table = config.table.as_deref().unwrap_or(&config.name);
            let source =
                PostgresDataSource::connect(&url, table, full_schema.clone()).map_err(|error| {
                    RegistryError::new(format!("datasource {:?}: {error}", config.name))
                })?;
            Backend::Postgres(source)
        }
        _ => {
            let data_path = base.join(config.path.as_deref().ok_or_else(|| {
                RegistryError::new(format!(
                    "datasource {:?}: a local-csv source needs a `path`",
                    config.name
                ))
            })?);
            let bytes = std::fs::read(&data_path).map_err(|error| {
                RegistryError::new(format!(
                    "datasource {:?}: {}: {error}",
                    config.name,
                    data_path.display()
                ))
            })?;
            let batches =
                load_csv(&bytes, &full_schema, CsvOptions::default()).map_err(|error| {
                    RegistryError::new(format!("datasource {:?}: {error}", config.name))
                })?;
            Backend::LocalCsv(LocalDataSource::new(batches).map_err(|error| {
                RegistryError::new(format!("datasource {:?}: {error}", config.name))
            })?)
        }
    };

    let client_schema = narrow(&full_schema, &config.allowed_fields, &config.name)?;
    if let Some(filter) = &config.row_filter {
        check_row_filter(filter, &full_schema, &config.name)?;
    }

    Ok(Source {
        name: config.name.clone(),
        client_schema,
        full_schema,
        row_filter: config.row_filter.clone(),
        data,
    })
}

/// The schema reduced to `allowed_fields`, in the schema's own order.
///
/// **A derivation never leaves the server** (plan point 54), the same way the
/// mandatory row filter does not (E16): the client schema says `ordered_year` is
/// an `int64`, which is all a client can do anything with, and where the values
/// come from stays here. That also keeps the narrowed schema sound on its own —
/// a derivation whose source column is not in `allowed_fields` would otherwise
/// dangle.
fn narrow(schema: &Schema, allowed: &[String], name: &str) -> Result<Schema, RegistryError> {
    if allowed.is_empty() {
        return Ok(schema.materialized());
    }
    for field in allowed {
        if schema.field(field).is_none() {
            return Err(RegistryError::new(format!(
                "datasource {name:?}: allowed_fields names {field:?}, which the schema does not have"
            )));
        }
    }
    let fields = schema
        .fields()
        .iter()
        .filter(|field| allowed.iter().any(|name| name == field.name.as_str()))
        .cloned()
        .collect();
    Ok(Schema::new(fields).materialized())
}

fn check_row_filter(
    filter: &RowFilterConfig,
    schema: &Schema,
    name: &str,
) -> Result<(), RegistryError> {
    if schema.field(&filter.field).is_none() {
        return Err(RegistryError::new(format!(
            "datasource {name:?}: row_filter names {:?}, which the schema does not have",
            filter.field
        )));
    }
    if CmpOp::parse(&filter.op).is_none() {
        return Err(RegistryError::new(format!(
            "datasource {name:?}: row_filter operator {:?} is not a comparison operator",
            filter.op
        )));
    }
    Ok(())
}

/// Why a request could not be turned into something to run.
pub enum PrepareError {
    /// The caller asked for something they may not have, or that does not exist.
    Validation(opengrid_query::QueryError),
    /// The pivot itself does not work — a limit or a shape, not a column.
    Pivot(PivotError),
    /// The server's own configuration and the caller's context do not fit.
    Context(String),
}

impl Source {
    /// Turns a client's query into the query that actually runs.
    ///
    /// Two validations on purpose (see the module docs): the first decides
    /// whether the caller may ask this at all, the second builds what runs — with
    /// the mandatory row filter added, which no request can remove.
    pub fn prepare(
        &self,
        query: Query,
        limits: &Limits,
        context: &BTreeMap<String, String>,
    ) -> Result<ValidatedQuery, PrepareError> {
        query
            .validate(&self.client_schema, limits)
            .map_err(PrepareError::Validation)?;

        let query = match &self.row_filter {
            None => query,
            Some(filter) => {
                let clause = build_row_filter(filter, context)?;
                let combined = match query.filter {
                    None => clause,
                    Some(existing) => FilterExpr::And(vec![existing, clause]),
                };
                Query {
                    filter: Some(combined),
                    ..query
                }
            }
        };

        query
            .validate(&self.full_schema, limits)
            .map_err(PrepareError::Validation)
    }
}

impl Source {
    /// The same two validations for a pivot (plan point 53).
    ///
    /// The mandatory row filter (E16) is added **before** the grouping sets are
    /// built, so it lands on every level — the grand total included. A filter
    /// that only reached the detail rows would leak the rest of the table into
    /// the totals, which is the exact thing E16 exists to prevent.
    pub fn prepare_pivot(
        &self,
        pivot: PivotQuery,
        limits: &Limits,
        pivot_limits: &PivotLimits,
        context: &BTreeMap<String, String>,
    ) -> Result<ValidatedPivotQuery, PrepareError> {
        pivot
            .validate(&self.client_schema, pivot_limits, limits)
            .map_err(pivot_validation)?;

        let pivot = match &self.row_filter {
            None => pivot,
            Some(filter) => {
                let clause = build_row_filter(filter, context)?;
                let combined = match pivot.filter {
                    None => clause,
                    Some(existing) => FilterExpr::And(vec![existing, clause]),
                };
                PivotQuery {
                    filter: Some(combined),
                    ..pivot
                }
            }
        };

        pivot
            .validate(&self.full_schema, pivot_limits, limits)
            .map_err(pivot_validation)
    }
}

/// A pivot's own failures carry the query validator's diagnosis where they have
/// one, and their own sentence where the fault is the pivot's shape.
fn pivot_validation(error: PivotError) -> PrepareError {
    match error {
        PivotError::Query(error) => PrepareError::Validation(error),
        other => PrepareError::Pivot(other),
    }
}

/// Builds the mandatory clause, resolving a `:key` value from the caller's
/// context.
fn build_row_filter(
    filter: &RowFilterConfig,
    context: &BTreeMap<String, String>,
) -> Result<FilterExpr, PrepareError> {
    let field = FieldName::new(&filter.field)
        .map_err(|error| PrepareError::Context(format!("row_filter field: {error}")))?;
    let op = CmpOp::parse(&filter.op)
        .ok_or_else(|| PrepareError::Context("row_filter operator".to_owned()))?;

    let value = match filter.value.strip_prefix(':') {
        // A literal, read as JSON so a number stays a number.
        None => serde_json::from_str(&filter.value)
            .unwrap_or_else(|_| serde_json::Value::String(filter.value.clone())),
        Some(key) => {
            let resolved = context.get(key).ok_or_else(|| {
                // The caller's token carries no such context value. That is a
                // configuration mismatch, and it must not fall back to "no
                // filter" — that would hand out every row.
                PrepareError::Context(format!(
                    "the token carries no context value {key:?} for the mandatory row filter"
                ))
            })?;
            serde_json::Value::String(resolved.clone())
        }
    };

    Ok(FilterExpr::Cmp { field, op, value })
}
