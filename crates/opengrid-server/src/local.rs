//! The local engine over an in-memory table, as a [`Connector`] (issue #45).
//!
//! What a `local-csv` source in the configuration becomes, and a reference for
//! a connector that holds its data itself: the engine answers every query, the
//! pivot takes the generic path, and an export runs the query **once** and
//! hands out slices of the answer — paging it instead would sort the whole
//! table again for every piece.

use std::time::Duration;

use opengrid_connector::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, ExportRows, QueryResult, Schema,
    ValidatedQuery,
};
use opengrid_datasource::SendDataSource;
use opengrid_engine::datasource::{LocalDataSource, LocalPieces};

/// A table in memory, answered by `opengrid-engine`.
pub struct LocalConnector(LocalDataSource);

impl LocalConnector {
    pub fn new(source: LocalDataSource) -> Self {
        Self(source)
    }
}

impl Connector for LocalConnector {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        Box::pin(SendDataSource::schema(&self.0))
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        SendDataSource::capabilities(&self.0)
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(SendDataSource::execute(&self.0, query))
    }

    fn export<'a>(
        &'a self,
        query: &'a ValidatedQuery,
        _idle_limit: Duration,
    ) -> BoxFuture<'a, Result<Box<dyn ExportRows + 'a>, DataSourceError>> {
        Box::pin(async move {
            let pieces: Box<dyn ExportRows + 'a> = Box::new(Pieces(self.0.pieces(query)?));
            Ok(pieces)
        })
    }
}

/// The answer of one run, handed out in slices.
struct Pieces(LocalPieces);

impl ExportRows for Pieces {
    fn count(&mut self) -> BoxFuture<'_, Result<u64, DataSourceError>> {
        let rows = self.0.rows();
        Box::pin(async move { Ok(rows) })
    }

    fn next_piece(&mut self, rows: usize) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        let piece = self.0.next_piece(rows);
        Box::pin(async move { piece })
    }
}
