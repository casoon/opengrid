//! The rows tier (issue #46, E36): a source that can only hand out rows.
//!
//! A file, a list in memory, a service without a query language — anything
//! that can say which columns it has and hand out its rows is a source.
//! [`Rows`] turns it into a full [`Connector`]: it reads the rows into a table
//! and lets `opengrid-engine` answer the query — filter, sort, group,
//! aggregate, paging, `total_count` — natively, on the server.
//!
//! # The bound
//!
//! The table exists for the length of one answer, in memory, per request.
//! [`Rows::max_scan_rows`] bounds it: a source with more rows to read is an
//! error that says so ([`DataSourceError::LimitExceeded`]), not a server out of
//! memory. The default is [`Rows::DEFAULT_MAX_SCAN_ROWS`], one million — about
//! 300 MiB for ten columns. More wants a source that answers queries itself.
//!
//! # The filter is a hint
//!
//! [`RowSource::scan`] gets the query's filter — the mandatory row filter
//! already in it. A source may use it to hand out fewer rows; it does not have
//! to. The engine applies the filter again either way, so a source that
//! ignores it, or applies it wrongly, cannot let a row through that the filter
//! excludes.

use std::sync::OnceLock;
use std::time::Duration;

use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::PieceBuilder;
use opengrid_query::ValidatedFilter;

use crate::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, ExportRows, QueryResult, Schema,
    SendDataSource, ValidatedQuery,
};

/// What a source of the rows tier implements.
pub trait RowSource: Send + Sync {
    /// Every column: the stored ones, and derived ones (`"from": { "part":
    /// "year", … }`), which the server computes.
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>>;

    /// Hands out the rows, a piece at a time. Each piece holds the **stored**
    /// columns of the schema, in order. `filter` is a hint (module docs).
    fn scan<'a>(
        &'a self,
        filter: Option<&'a ValidatedFilter>,
    ) -> BoxFuture<'a, Result<Box<dyn RowStream + 'a>, DataSourceError>>;
}

/// The rows of one scan.
pub trait RowStream: Send {
    /// The next piece, or `None` after the last one.
    fn next_piece(&mut self) -> BoxFuture<'_, Result<Option<QueryResult>, DataSourceError>>;
}

/// A [`RowSource`] as a [`Connector`], answered by the engine.
pub struct Rows<R> {
    source: R,
    max_scan_rows: u64,
    schema: OnceLock<Schema>,
}

impl<R: RowSource> Rows<R> {
    /// One million rows: about 300 MiB for ten columns, per request.
    pub const DEFAULT_MAX_SCAN_ROWS: u64 = 1_000_000;

    pub fn new(source: R) -> Self {
        Self {
            source,
            max_scan_rows: Self::DEFAULT_MAX_SCAN_ROWS,
            schema: OnceLock::new(),
        }
    }

    /// The most rows one answer may read from the source.
    pub fn max_scan_rows(mut self, rows: u64) -> Self {
        self.max_scan_rows = rows;
        self
    }

    /// The schema, asked once.
    async fn full_schema(&self) -> Result<Schema, DataSourceError> {
        if let Some(schema) = self.schema.get() {
            return Ok(schema.clone());
        }
        let schema = self.source.schema().await?;
        Ok(self.schema.get_or_init(|| schema).clone())
    }

    /// Reads what the source hands out for `filter` into the engine.
    async fn load(
        &self,
        filter: Option<&ValidatedFilter>,
    ) -> Result<LocalDataSource, DataSourceError> {
        let schema = self.full_schema().await?;
        let mut builder = PieceBuilder::new(&schema);
        let mut stream = self.source.scan(filter).await?;
        while let Some(piece) = stream.next_piece().await? {
            builder.push(&piece).map_err(ingest)?;
            if builder.rows() > self.max_scan_rows {
                return Err(DataSourceError::LimitExceeded {
                    message: format!(
                        "the source has more than {} rows to read for this query \
                         (max_scan_rows); narrow the filter",
                        self.max_scan_rows
                    ),
                });
            }
        }
        Ok(LocalDataSource::new(builder.finish().map_err(ingest)?))
    }
}

impl<R: RowSource> Connector for Rows<R> {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        Box::pin(self.full_schema())
    }

    /// The engine answers everything once the rows are read.
    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities::ALL
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(async move {
            let local = self.load(query.filter.as_ref()).await?;
            SendDataSource::execute(&local, query).await
        })
    }

    /// Reads the rows once and hands out slices of the one answer; the
    /// default would scan twice, once to count and once to read.
    fn export<'a>(
        &'a self,
        query: &'a ValidatedQuery,
        _idle_limit: Duration,
    ) -> BoxFuture<'a, Result<Box<dyn ExportRows + 'a>, DataSourceError>> {
        Box::pin(async move {
            let local = self.load(query.filter.as_ref()).await?;
            let pieces: Box<dyn ExportRows + 'a> = Box::new(Pieces(local.pieces(query)?));
            Ok(pieces)
        })
    }
}

struct Pieces(opengrid_engine::datasource::LocalPieces);

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

fn ingest(error: opengrid_engine::ingest::IngestError) -> DataSourceError {
    DataSourceError::Backend {
        message: format!("the source's rows: {error}"),
    }
}
