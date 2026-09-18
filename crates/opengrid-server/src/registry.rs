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

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_datasource::SendDataSource;
use opengrid_datasource_postgres::PostgresDataSource;
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

/// The sources, by name.
pub struct Registry {
    sources: BTreeMap<String, Arc<Source>>,
    pub limits: Limits,
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
        Ok(Self { sources, limits })
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
fn narrow(schema: &Schema, allowed: &[String], name: &str) -> Result<Schema, RegistryError> {
    if allowed.is_empty() {
        return Ok(schema.clone());
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
    Ok(Schema::new(fields))
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
