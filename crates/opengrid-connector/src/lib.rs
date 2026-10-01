//! `opengrid-connector` — the contract a data source implements to be served by
//! `opengrid-server` (issue #45, E36).
//!
//! **The server knows no database.** An application implements [`Connector`]
//! for whatever holds its data — PostgreSQL, a file, another service, something
//! with no query language at all — and hands it to the server. opengrid keeps
//! no drivers; PostgreSQL and the local engine are reference implementations,
//! nothing more.
//!
//! # What a connector has to do
//!
//! | Method | Required | Without it |
//! |---|---|---|
//! | [`schema`](Connector::schema) | yes | — |
//! | [`capabilities`](Connector::capabilities) | yes | — |
//! | [`execute`](Connector::execute) | yes | — |
//! | [`pivot`](Connector::pivot) | no | one query per level (`opengrid-pivot`'s generic path) |
//! | [`export`](Connector::export) | no | counted with one query, then read with one more |
//! | [`concurrent_exports`](Connector::concurrent_exports) | no | the server picks its own bound |
//!
//! # What a connector never has to do
//!
//! Security. The server checks the token, narrows the schema to the allowed
//! fields and adds the mandatory row filter (E16) **before** a query reaches
//! the connector, and validates it against the full schema. A connector only
//! ever sees a [`ValidatedQuery`] that already carries every rule, so there is
//! nothing it could forget.
//!
//! # Why boxed futures
//!
//! The server holds its sources as `Arc<dyn Connector>`, one list for every
//! kind. A trait with `async fn` cannot be used that way, so the methods return
//! [`BoxFuture`] — written by hand, no new dependency. Implementing it is one
//! `Box::pin(async move { … })` per method.
//!
//! # Stability
//!
//! The contract is frozen with its first release (E36). It grows only by new
//! methods with a default, so a connector written today keeps compiling.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult};
pub use opengrid_pivot::{PivotResult, ValidatedPivotQuery};
pub use opengrid_query::ValidatedQuery;
pub use opengrid_types::{Schema, Value};

use opengrid_datasource::SendDataSource;

pub mod local;
mod rows;
pub use local::LocalConnector;
pub use rows::{RowSource, RowStream, Rows};

/// A future a connector returns: boxed, so the trait can be used as
/// `dyn Connector`, and `Send`, so the server can run it on any thread.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A source of tabular data the server can answer for.
pub trait Connector: Send + Sync {
    /// Every column the source has — the full schema, including columns only a
    /// row filter may name. The server narrows it for clients.
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>>;

    /// What this source answers by itself. A browser-side planner reads it
    /// through `GET /source/{name}` to decide what to push.
    fn capabilities(&self) -> DataSourceCapabilities;

    /// Runs a validated query and answers with the requested page, and
    /// `total_count` — the rows that matched **before** `offset` and `limit`.
    fn execute(&self, query: ValidatedQuery)
    -> BoxFuture<'_, Result<QueryResult, DataSourceError>>;

    /// Runs a whole pivot. The default asks one query per level through
    /// [`execute`](Self::execute); a source that can answer every level in one
    /// statement (PostgreSQL's `GROUPING SETS`) overrides it.
    fn pivot<'a>(
        &'a self,
        pivot: &'a ValidatedPivotQuery,
    ) -> BoxFuture<'a, Result<PivotResult, DataSourceError>> {
        Box::pin(async move {
            opengrid_pivot::execute(&AsSource(self), pivot)
                .await
                .map_err(|error| DataSourceError::Backend {
                    message: error.to_string(),
                })
        })
    }

    /// Answers one level of a tree (E38) for a source that cannot by itself
    /// (`capabilities().tree` is false): the rows of the tree's scope — the
    /// mandatory row filter is in it — are asked for in one query, and the
    /// engine answers the level over them (plan point 122). A source that
    /// answers trees gets the query through [`execute`](Self::execute).
    fn tree<'a>(
        &'a self,
        query: &'a ValidatedQuery,
    ) -> BoxFuture<'a, Result<QueryResult, DataSourceError>> {
        Box::pin(async move {
            let Some(tree) = &query.tree else {
                return self.execute(query.clone()).await;
            };
            let schema = self.schema().await?.materialized();
            let rows = ValidatedQuery {
                source: query.source.clone(),
                select: schema
                    .fields()
                    .iter()
                    .map(|field| field.name.clone())
                    .collect(),
                // Only the scope here: the ancestors of T5 do not pass the
                // filter and still belong to the answer.
                filter: tree.scope.clone(),
                group: Vec::new(),
                aggregate: Vec::new(),
                sort: Vec::new(),
                offset: None,
                limit: None,
                tree: None,
                output_schema: schema.clone(),
            };
            let answer = self.execute(rows).await?;
            let table = opengrid_columns::Table::from_values(&answer.schema, &answer.columns)
                .map_err(|message| DataSourceError::Backend { message })?;
            let result = opengrid_engine::execute::execute(&table, query).map_err(|error| {
                DataSourceError::Backend {
                    message: error.to_string(),
                }
            })?;
            let mut level = QueryResult::new(
                query.output_schema.clone(),
                result.table.to_values(),
                result.total_count,
            );
            level.tree = result.tree;
            Ok(level)
        })
    }

    /// Starts an export of `query`: every row of the answer, a piece at a time.
    /// Nothing heavy may have run when this returns — the server holds the
    /// [`ExportRows::canceller`] before it asks for the count.
    ///
    /// `idle_limit` is how long the export may sit between two pieces before
    /// the source may end it on its own — a backstop behind the server's own
    /// timeouts, for a source that holds a transaction or a cursor open.
    ///
    /// The default counts with one query (`limit` 0, so only `total_count`
    /// comes back) and reads the rows with one more when the first piece is
    /// asked for. A source with cursors, or one that holds its data in memory,
    /// overrides it.
    fn export<'a>(
        &'a self,
        query: &'a ValidatedQuery,
        idle_limit: Duration,
    ) -> BoxFuture<'a, Result<Box<dyn ExportRows + 'a>, DataSourceError>> {
        let _ = idle_limit;
        Box::pin(async move {
            let rows: Box<dyn ExportRows + 'a> = Box::new(PagedExport {
                connector: self,
                query: query.clone(),
                rows: None,
                next: 0,
            });
            Ok(rows)
        })
    }

    /// How many exports this source can carry at once, if it knows — a pool
    /// of connections does. The server takes the smallest answer over its
    /// sources as its default bound; `None` leaves it to the server.
    fn concurrent_exports(&self) -> Option<usize> {
        None
    }
}

/// An export in progress.
pub trait ExportRows: Send {
    /// The rows the export will have, before the first of them is read —
    /// after the query's own `offset` and `limit`.
    fn count(&mut self) -> BoxFuture<'_, Result<u64, DataSourceError>>;

    /// The next piece of at most `rows` rows. A shorter piece is the last one.
    fn next_piece(&mut self, rows: usize) -> BoxFuture<'_, Result<QueryResult, DataSourceError>>;

    /// What stops a statement that is still running, where there is one to
    /// stop. The server calls it when the client leaves or a step takes too long.
    fn canceller(&self) -> Option<Arc<dyn Cancel>> {
        None
    }

    /// Ends the export before its last piece — refused, or failed — so a
    /// connection can go back where it came from. The server drops the export
    /// right after; dropping without this must be safe too, this is the clean
    /// way.
    fn close(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// Stops what an export is running, from outside it.
pub trait Cancel: Send + Sync {
    fn cancel(&self) -> BoxFuture<'_, ()>;
}

/// Any [`SendDataSource`] as a connector: the source answers queries, and pivot
/// and export take the defaults. The bridge for sources written against the
/// older trait.
pub struct FromSource<S>(pub S);

impl<S: SendDataSource + Send + Sync> Connector for FromSource<S> {
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
}

/// A connector seen as a `DataSource`: how `opengrid-pivot` asks it one query
/// per level, and how `opengrid_conformance::check_source` runs the suite
/// against it (issue #47).
pub struct AsSource<'a, C: ?Sized>(pub &'a C);

impl<C: Connector + ?Sized> SendDataSource for AsSource<'_, C> {
    fn schema(&self) -> impl Future<Output = Result<Schema, DataSourceError>> + Send {
        self.0.schema()
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> impl Future<Output = Result<QueryResult, DataSourceError>> + Send {
        self.0.execute(query)
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        self.0.capabilities()
    }
}

/// The default export: counted with `limit` 0, read with one query.
struct PagedExport<'a, C: ?Sized> {
    connector: &'a C,
    query: ValidatedQuery,
    /// The whole answer, once the first piece was asked for.
    rows: Option<QueryResult>,
    next: usize,
}

impl<C: Connector + ?Sized> ExportRows for PagedExport<'_, C> {
    fn count(&mut self) -> BoxFuture<'_, Result<u64, DataSourceError>> {
        Box::pin(async move {
            let counting = ValidatedQuery {
                offset: None,
                limit: Some(0),
                ..self.query.clone()
            };
            let total = self.connector.execute(counting).await?.total_count;
            let after_offset = total.saturating_sub(self.query.offset.unwrap_or(0));
            Ok(self
                .query
                .limit
                .map_or(after_offset, |limit| after_offset.min(limit)))
        })
    }

    fn next_piece(&mut self, rows: usize) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(async move {
            if self.rows.is_none() {
                self.rows = Some(self.connector.execute(self.query.clone()).await?);
            }
            let all = self.rows.as_ref().expect("read above");
            let start = self.next.min(all.row_count());
            let end = (start + rows).min(all.row_count());
            self.next = end;
            let columns = all
                .columns
                .iter()
                .map(|column| column[start..end].to_vec())
                .collect();
            Ok(QueryResult::new(
                all.schema.clone(),
                columns,
                all.total_count,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_conformance::block_on;
    use opengrid_engine::datasource::LocalDataSource;
    use opengrid_engine::ingest::{CsvOptions, load_csv};
    use opengrid_query::{Limits, Query};

    const SCHEMA: &str = r#"{"fields":[
        {"name":"id","type":"int64","nullable":false},
        {"name":"country","type":"utf8","nullable":true},
        {"name":"qty","type":"int64","nullable":true}]}"#;
    const CSV: &str = "id,country,qty\n1,DE,3\n2,FR,1\n3,DE,2\n4,AT,5\n5,DE,4\n";

    fn connector() -> FromSource<LocalDataSource> {
        let schema: Schema = opengrid_json::from_str(SCHEMA).unwrap();
        let table = load_csv(CSV.as_bytes(), &schema, CsvOptions::default()).unwrap();
        FromSource(LocalDataSource::new(table))
    }

    fn validated(json: &str) -> ValidatedQuery {
        let schema: Schema = opengrid_json::from_str(SCHEMA).unwrap();
        let query: Query = opengrid_json::from_str(json).unwrap();
        query.validate(&schema, &Limits::default()).unwrap()
    }

    /// The default export counts after `offset` and `limit`, and its pieces
    /// together are the query's own answer, in order.
    #[test]
    fn the_default_export_counts_and_pieces_like_the_query() {
        let connector = connector();
        let query = validated(
            r#"{"source":"t","select":["id"],"sort":[{"field":"id","direction":"desc"}],"offset":1,"limit":3}"#,
        );
        let direct = block_on(connector.execute(query.clone())).unwrap();

        let mut export = block_on(connector.export(&query, Duration::from_secs(1))).unwrap();
        assert_eq!(block_on(export.count()).unwrap(), 3);
        let first = block_on(export.next_piece(2)).unwrap();
        let second = block_on(export.next_piece(2)).unwrap();
        let empty = block_on(export.next_piece(2)).unwrap();
        assert_eq!(first.row_count(), 2);
        assert_eq!(second.row_count(), 1, "a short piece is the last");
        assert_eq!(empty.row_count(), 0);
        let ids: Vec<Value> = first.columns[0]
            .iter()
            .chain(&second.columns[0])
            .cloned()
            .collect();
        assert_eq!(ids, direct.columns[0]);
    }

    /// Without `limit`, the count is every match after the offset.
    #[test]
    fn the_default_count_without_a_limit_is_every_match() {
        let connector = connector();
        let query = validated(
            r#"{"source":"t","select":["id"],"filter":{"field":"country","op":"eq","value":"DE"},"sort":[{"field":"id","direction":"asc"}],"offset":1}"#,
        );
        let mut export = block_on(connector.export(&query, Duration::from_secs(1))).unwrap();
        assert_eq!(block_on(export.count()).unwrap(), 2);
    }
}
