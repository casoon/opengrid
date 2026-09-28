//! PostgreSQL as a [`Connector`] (issue #45): queries through the compiler,
//! the pivot as one `GROUPING SETS` statement, the export through a cursor.

use std::sync::Arc;
use std::time::Duration;

use opengrid_connector::{
    BoxFuture, Cancel, Connector, DataSourceCapabilities, DataSourceError, ExportRows, PivotResult,
    QueryResult, Schema, ValidatedPivotQuery, ValidatedQuery,
};
use opengrid_datasource::SendDataSource;

use crate::{ExportCanceller, PostgresDataSource, PostgresExport};

impl Connector for PostgresDataSource {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        Box::pin(SendDataSource::schema(self))
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        SendDataSource::capabilities(self)
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(SendDataSource::execute(self, query))
    }

    fn pivot<'a>(
        &'a self,
        pivot: &'a ValidatedPivotQuery,
    ) -> BoxFuture<'a, Result<PivotResult, DataSourceError>> {
        Box::pin(self.execute_pivot(pivot))
    }

    fn export<'a>(
        &'a self,
        query: &'a ValidatedQuery,
        idle_limit: Duration,
    ) -> BoxFuture<'a, Result<Box<dyn ExportRows + 'a>, DataSourceError>> {
        Box::pin(async move {
            let export: Box<dyn ExportRows + 'a> =
                Box::new(PostgresDataSource::export(self, query, idle_limit).await?);
            Ok(export)
        })
    }

    /// Every export holds one pooled connection for as long as its client
    /// downloads; half the pool stays for `/query` and `/pivot`.
    fn concurrent_exports(&self) -> Option<usize> {
        Some(self.pool_size() / 2)
    }
}

impl ExportRows for PostgresExport {
    fn count(&mut self) -> BoxFuture<'_, Result<u64, DataSourceError>> {
        Box::pin(PostgresExport::count(self))
    }

    fn next_piece(&mut self, rows: usize) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(PostgresExport::next_piece(self, rows))
    }

    fn canceller(&self) -> Option<Arc<dyn Cancel>> {
        Some(Arc::new(PostgresExport::canceller(self)))
    }

    fn close(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(self.roll_back())
    }
}

impl Cancel for ExportCanceller {
    fn cancel(&self) -> BoxFuture<'_, ()> {
        Box::pin(ExportCanceller::cancel(self))
    }
}
