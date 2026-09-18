//! Hybrid execution: the source does what it can, this engine finishes the rest
//! (plan point 28, plan/spezifikation/05-planner.md §Hybrid Execution).
//!
//! [`opengrid_planner`] decides *who does what* — it is a pure value and knows
//! nothing about Arrow. This is the other half: running that decision. The
//! source answers the part it was given, the partial answer comes back through
//! the coercion path E14 names ([`LocalDataSource::from_result`]), and the local
//! engine runs the plan's client query over it.
//!
//! The wrapper is itself a [`DataSource`](opengrid_datasource::DataSource) that
//! reports every capability, because together the two halves can answer any
//! query the engine can — which is what lets the conformance suite run over the
//! hybrid path unchanged.

use std::future::Future;

use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult, SendDataSource};
use opengrid_planner::{ExecutionMode, ExecutionPlan, plan};
use opengrid_query::ValidatedQuery;
use opengrid_types::Schema;

use crate::datasource::LocalDataSource;

/// A source plus the local engine, with the planner between them.
pub struct HybridSource<S> {
    source: S,
    schema: Schema,
    mode: ExecutionMode,
}

impl<S: SendDataSource> HybridSource<S> {
    /// Wraps `source`, whose data has `schema`.
    ///
    /// The schema is passed in rather than awaited from the source: planning is
    /// synchronous, and the caller has the schema already — it is what the
    /// query was validated against.
    pub fn new(source: S, schema: Schema) -> Self {
        Self {
            source,
            schema,
            mode: ExecutionMode::Auto,
        }
    }

    /// The same source in an explicit mode.
    ///
    /// The three explicit modes must stay enforceable even where `auto` would
    /// decide otherwise (plan point 28, step 5): `local` leaves the source with
    /// a plain scan, `remote` fails rather than quietly finishing the work here.
    pub fn with_mode(mut self, mode: ExecutionMode) -> Self {
        self.mode = mode;
        self
    }

    /// The plan for `query`, without running it — for developer tools.
    pub fn plan_for(&self, query: &ValidatedQuery) -> Result<ExecutionPlan, DataSourceError> {
        plan(query, &self.schema, self.source.capabilities(), self.mode).map_err(planning)
    }

    /// Runs `query` and answers with the result **and** the plan that produced
    /// it (05-planner.md: "offengelegt für Debugging").
    pub async fn run(
        &self,
        query: ValidatedQuery,
    ) -> Result<(QueryResult, ExecutionPlan), DataSourceError> {
        let plan = self.plan_for(&query)?;
        let partial = self.source.execute(plan.source_query.clone()).await?;

        let Some(client_query) = plan.client_query.clone() else {
            return Ok((partial, plan));
        };

        // The partial answer becomes the local engine's data, and the client
        // query is a complete query over it: the remaining steps, in pipeline
        // order, including the row count the grid shows — `total_count` is
        // recomputed here because filtering or grouping changed it.
        let local = LocalDataSource::from_result(&partial)?;
        let result = SendDataSource::execute(&local, client_query).await?;
        Ok((result, plan))
    }
}

impl<S: SendDataSource + Sync> SendDataSource for HybridSource<S> {
    fn schema(&self) -> impl Future<Output = Result<Schema, DataSourceError>> + Send {
        let schema = self.schema.clone();
        async move { Ok(schema) }
    }

    async fn execute(&self, query: ValidatedQuery) -> Result<QueryResult, DataSourceError> {
        self.run(query).await.map(|(result, _)| result)
    }

    /// Everything — whatever the inner source cannot do, the engine does.
    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities::ALL
    }
}

fn planning(error: opengrid_planner::PlanError) -> DataSourceError {
    DataSourceError::Backend {
        message: error.to_string(),
    }
}
