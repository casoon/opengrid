//! A whole query out of PostgreSQL, read through a cursor (issue #2).
//!
//! # Why a cursor, and not `OFFSET` pieces
//!
//! Paging an export with `OFFSET` makes every piece more expensive than the one
//! before — the database produces and throws away every row in front of the
//! window — and between two statements the rows can move. A cursor runs the
//! statement once and hands its rows out as they are asked for, so the server
//! holds one piece at a time, whatever the export's length.
//!
//! # One snapshot for the count and the rows
//!
//! [`PostgresExport::count`] counts before a single row is read, so the server
//! can refuse an export that is too long **before the first byte** — with a
//! status code, not a file cut off in the middle. A limit in the cursor could
//! only notice after that many rows had already been sent. The count and the
//! cursor run in one read-only `REPEATABLE READ` transaction: both see the same
//! snapshot, so the number counted is the number of rows that come.
//!
//! # Ending early
//!
//! An export dropped before its last piece — the client went away, a fetch
//! took too long — must not hand a connection with an open transaction back to
//! the pool: the next request would run inside this snapshot, read-only. So
//! that connection is taken out of the pool and closed, and PostgreSQL rolls
//! the transaction back and closes the cursor as the backend goes. Between two
//! fetches the backend is idle and goes at once. A statement that is still
//! running is only stopped by a cancel request: [`ExportCanceller`], which a
//! caller that gives up in the middle of a step sends before it drops the
//! export.

use deadpool_postgres::Object;
use opengrid_datasource::{DataSourceError, QueryResult};
use opengrid_query::ValidatedQuery;
use opengrid_types::{DataType, Schema};
use tokio_postgres::types::ToSql;
use tokio_postgres::{CancelToken, NoTls};

use crate::compiler::{CompiledQuery, QueryCompiler};
use crate::source::{
    PostgresDataSource, backend, columns_of, compile, text_params, texts, wrap_for_reading,
};

/// The cursor's name. One export per connection, so one name is enough.
const CURSOR: &str = "opengrid_export";

/// An export in progress: a connection, its transaction and, after
/// [`count`](Self::count), a cursor over the query.
pub struct PostgresExport {
    /// `Some` until the export is dropped; only `Drop` takes it.
    client: Option<Object>,
    cancel: CancelToken,
    /// The statement `/query` would run, wrapped to read every column as text.
    statement: CompiledQuery,
    counting: CompiledQuery,
    output: Vec<(String, DataType)>,
    schema: Schema,
    offset: u64,
    limit: Option<u64>,
    rows: u64,
    /// The transaction has ended, and the connection may go back to the pool.
    finished: bool,
}

/// Stops the statement an export is running, from outside it.
pub struct ExportCanceller(CancelToken);

impl ExportCanceller {
    /// Asks PostgreSQL to cancel whatever the export's connection is running.
    /// Best effort by nature: a statement that has just finished has nothing
    /// left to cancel, and a failed request changes nothing about the export,
    /// which is being dropped anyway.
    pub async fn cancel(&self) {
        let _ = self.0.cancel_query(NoTls).await;
    }
}

impl PostgresDataSource {
    /// Starts an export of `query`: a connection from the pool and a read-only
    /// `REPEATABLE READ` transaction. Nothing is counted or read yet, so the
    /// caller holds the [`ExportCanceller`] before anything long runs.
    ///
    /// The statement is the one [`execute`](opengrid_datasource::SendDataSource::execute)
    /// runs for the same query — the same compiler, the same parameters, the
    /// same text form of every value — only read through a cursor.
    pub async fn export(&self, query: &ValidatedQuery) -> Result<PostgresExport, DataSourceError> {
        let output: Vec<(String, DataType)> = query
            .output_schema
            .fields()
            .iter()
            .map(|field| (field.name.as_str().to_owned(), field.data_type))
            .collect();
        let compiled = self.compiler.compile(query).map_err(compile)?;
        let statement = CompiledQuery {
            sql: wrap_for_reading(&compiled.sql, &output),
            params: compiled.params,
        };
        let counting = CompiledQuery::count_of(query, &self.compiler).map_err(compile)?;

        let client = self.pool.get().await.map_err(backend)?;
        let cancel = client.cancel_token();
        // Built before `BEGIN`, so a failure from here on drops an export that
        // is not finished — and the connection with it, not back to the pool.
        let export = PostgresExport {
            client: Some(client),
            cancel,
            statement,
            counting,
            output,
            schema: query.output_schema.clone(),
            offset: query.offset.unwrap_or(0),
            limit: query.limit,
            rows: 0,
            finished: false,
        };
        export
            .client()
            .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(backend)?;
        Ok(export)
    }
}

impl PostgresExport {
    fn client(&self) -> &Object {
        self.client
            .as_ref()
            .expect("the connection is held until drop")
    }

    /// What stops this export's running statement from outside.
    pub fn canceller(&self) -> ExportCanceller {
        ExportCanceller(self.cancel.clone())
    }

    /// Counts the rows the export will have — after the query's `offset` and
    /// `limit` — and opens the cursor over them, in the same snapshot. Call it
    /// once, before the first [`next_piece`](Self::next_piece).
    pub async fn count(&mut self) -> Result<u64, DataSourceError> {
        let params = text_params(&self.counting.params);
        let refs = as_refs(&params);
        let counted = self
            .client()
            .query(self.counting.sql.as_str(), &refs)
            .await
            .map_err(backend)?;
        let matched: i64 = counted
            .first()
            .map(|row| row.get(0))
            .ok_or_else(|| backend("the count answered no row"))?;
        let after_offset = u64::try_from(matched)
            .unwrap_or(0)
            .saturating_sub(self.offset);
        self.rows = self
            .limit
            .map_or(after_offset, |limit| after_offset.min(limit));

        let params = text_params(&self.statement.params);
        let refs = as_refs(&params);
        let declare = format!(
            "DECLARE \"{CURSOR}\" NO SCROLL CURSOR FOR {}",
            self.statement.sql
        );
        self.client()
            .execute(declare.as_str(), &refs)
            .await
            .map_err(backend)?;
        Ok(self.rows)
    }

    /// The next piece of at most `rows` rows. A piece shorter than `rows` is
    /// the last one: the transaction ends with it, and the connection goes back
    /// to the pool when the export is dropped. After it, every piece is empty.
    pub async fn next_piece(&mut self, rows: usize) -> Result<QueryResult, DataSourceError> {
        if self.finished {
            return Ok(QueryResult::new(
                self.schema.clone(),
                vec![Vec::new(); self.output.len()],
                self.rows,
            ));
        }
        let fetch = format!("FETCH FORWARD {rows} FROM \"{CURSOR}\"");
        let fetched = self
            .client()
            .query(fetch.as_str(), &[])
            .await
            .map_err(backend)?;
        let length = fetched.len();
        let columns = columns_of(texts(&fetched, self.output.len()), &self.output)?;
        drop(fetched);
        if length < rows {
            // Ends the transaction and closes the cursor with it.
            self.client()
                .batch_execute("COMMIT")
                .await
                .map_err(backend)?;
            self.finished = true;
        }
        Ok(QueryResult::new(self.schema.clone(), columns, self.rows))
    }
}

impl Drop for PostgresExport {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // Out of the pool and closed: see the module docs, "Ending early".
        if let Some(client) = self.client.take() {
            drop(Object::take(client));
        }
    }
}

fn as_refs(params: &[Option<String>]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|param| param as &(dyn ToSql + Sync))
        .collect()
}
