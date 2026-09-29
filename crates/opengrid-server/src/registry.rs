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
use std::sync::Arc;

use opengrid_connector::Connector;
use opengrid_pivot::{PivotError, PivotLimits, PivotQuery, ValidatedPivotQuery};
use opengrid_query::{CmpOp, FilterExpr, Limits, Query, ValidatedQuery};
use opengrid_types::{FieldName, Schema};

/// One source the server will answer for.
pub struct Source {
    pub name: String,
    /// The schema a client's query is validated against — `allowed_fields` only.
    pub client_schema: Schema,
    /// Every column, including the ones only the row filter may name.
    pub full_schema: Schema,
    pub row_filter: Option<RowFilter>,
    /// What actually answers (issue #45): any [`Connector`]. The endpoint does
    /// not know which one it has, and does not need to.
    pub data: Arc<dyn Connector>,
}

/// What the server enforces for a source, whatever connector answers it.
#[derive(Clone, Debug, Default)]
pub struct SourcePolicy {
    /// The columns a client may name. Empty means every column.
    pub allowed_fields: Vec<String>,
    /// The filter the server adds to every query (E16).
    pub row_filter: Option<RowFilter>,
}

/// A mandatory row filter: `field op value`, where `value` may be `:key` to
/// take the caller's context value — the tenant a token stands for.
#[derive(Clone, Debug)]
pub struct RowFilter {
    pub field: String,
    pub op: String,
    pub value: String,
}

impl RowFilter {
    /// `field op value`, e.g. `RowFilter::new("tenant_id", "eq", ":tenant")`.
    pub fn new(field: impl Into<String>, op: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            op: op.into(),
            value: value.into(),
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
    pub(crate) fn new(message: impl Into<String>) -> Self {
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
    /// A registry over sources that are already checked ([`Source::new`]).
    pub fn new(sources: Vec<Source>, limits: Limits, pivot_limits: PivotLimits) -> Self {
        Self {
            sources: sources
                .into_iter()
                .map(|source| (source.name.clone(), Arc::new(source)))
                .collect(),
            limits,
            pivot_limits,
        }
    }

    /// `max_concurrent_exports` when nobody set it: the smallest bound a
    /// connector names ([`Connector::concurrent_exports`] — PostgreSQL says half
    /// its pool), at least 1. Every export holds its source's resources for as
    /// long as its client downloads, and the bound is server-wide, so the
    /// smallest one is the one that holds. Without any, the machine's
    /// parallelism: an in-memory source holds a whole answer per export instead.
    pub fn default_concurrent_exports(&self) -> usize {
        self.sources
            .values()
            .filter_map(|source| source.data.concurrent_exports())
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

impl Source {
    /// A source the server may answer for: the connector, its full schema and
    /// the policy. Everything that can be wrong with them is a startup error
    /// here, never a surprise on the first request.
    pub fn new(
        name: &str,
        data: Arc<dyn Connector>,
        full_schema: Schema,
        policy: SourcePolicy,
    ) -> Result<Self, RegistryError> {
        // A broken derivation is a startup failure like every other
        // configuration mistake (plan point 54): a gateway that came up with
        // one would serve a column full of NULL and nobody would notice.
        full_schema
            .check()
            .map_err(|error| RegistryError::new(format!("datasource {name:?}: {error}")))?;
        let client_schema = narrow(&full_schema, &policy.allowed_fields, name)?;
        if let Some(filter) = &policy.row_filter {
            check_row_filter(filter, &full_schema, name)?;
        }
        Ok(Self {
            name: name.to_owned(),
            client_schema,
            full_schema,
            row_filter: policy.row_filter,
            data,
        })
    }
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

fn check_row_filter(filter: &RowFilter, schema: &Schema, name: &str) -> Result<(), RegistryError> {
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
    filter: &RowFilter,
    context: &BTreeMap<String, String>,
) -> Result<FilterExpr, PrepareError> {
    let field = FieldName::new(&filter.field)
        .map_err(|error| PrepareError::Context(format!("row_filter field: {error}")))?;
    let op = CmpOp::parse(&filter.op)
        .ok_or_else(|| PrepareError::Context("row_filter operator".to_owned()))?;

    let value = match filter.value.strip_prefix(':') {
        // A literal, read as JSON so a number stays a number.
        None => opengrid_json::Json::parse(&filter.value)
            .unwrap_or_else(|_| opengrid_json::Json::from(filter.value.as_str())),
        Some(key) => {
            let resolved = context.get(key).ok_or_else(|| {
                // The caller's token carries no such context value. That is a
                // configuration mismatch, and it must not fall back to "no
                // filter" — that would hand out every row.
                PrepareError::Context(format!(
                    "the token carries no context value {key:?} for the mandatory row filter"
                ))
            })?;
            opengrid_json::Json::from(resolved.as_str())
        }
    };

    Ok(FilterExpr::Cmp { field, op, value })
}
