use opengrid_query::ValidatedQuery;
use opengrid_types::Schema;

use crate::{DataSourceCapabilities, DataSourceError, QueryResult};

/// A queryable source of tabular data.
///
/// The single interface the UI and the planner work against
/// (plan/spezifikation/03-datasource.md). Implementations: the local engine
/// ([`LocalDataSource`], point 09 — in `opengrid-arrow-engine`, because Arrow
/// lives there), PostgreSQL (point 24/26), REST (point 27).
///
/// # `Send` and no `Send`
///
/// This is the browser variant: its futures may hold state that is not `Send`
/// (a `RefCell`, a JS handle), which is what a WASM client needs. On the server
/// every future has to be `Send`, so `trait-variant` generates
/// [`SendDataSource`] from this same definition — one definition, both worlds
/// (decision E5), with no `async-trait` boxing.
///
/// A type that implements [`SendDataSource`] also implements `DataSource`: the
/// macro generates that blanket impl. Generic code can therefore be written
/// against `DataSource` and accept either kind. Where a concrete type is known to
/// implement both and both traits are in scope, name the trait explicitly —
/// `DataSource::execute(&source, query)` — to keep the call unambiguous.
///
/// # Validated queries only
///
/// [`execute`](Self::execute) takes a [`ValidatedQuery`], never the raw query:
/// the source may assume that field names, operator suitability and literal types
/// are already checked, and no implementation can skip validation
/// (plan/spezifikation/02-query-modell.md).
#[trait_variant::make(SendDataSource: Send)]
pub trait DataSource {
    /// The columns a query may name.
    async fn schema(&self) -> Result<Schema, DataSourceError>;

    /// Runs a validated query and answers with the requested page.
    ///
    /// The result carries `total_count`, the size of the filtered result *before*
    /// paging.
    async fn execute(&self, query: ValidatedQuery) -> Result<QueryResult, DataSourceError>;

    /// What this source can answer by itself.
    fn capabilities(&self) -> DataSourceCapabilities;
}
