//! The local engine as a [`DataSource`](opengrid_datasource::DataSource).
//!
//! Point 09 binds data and engine in one place: [`LocalDataSource`] holds the
//! table and answers with the column-free
//! [`QueryResult`](opengrid_datasource::QueryResult) (E14). Before this point the
//! conformance runner needed a test-local adapter (`LocalEngine`), because the
//! orphan rule kept this crate from implementing the suite's trait — that
//! workaround is gone with the real trait.
//!
//! The source lives here rather than in `opengrid-datasource` because that crate
//! holds the contract only, not an engine (plan/spezifikation/11-crates.md
//! §Portabilität, the dependency points `engine → datasource`, never back).

use std::future::Future;

use opengrid_columns::Table;
use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult, SendDataSource};
use opengrid_query::ValidatedQuery;
use opengrid_types::Schema;

use crate::execute::execute;

/// The in-memory source: a table plus the local executor.
///
/// This is the implementation the browser runs in WASM. It reports every
/// capability, because the engine answers filter, sort, group, aggregate and
/// paging itself (plan/spezifikation/03-datasource.md §Capabilities).
pub struct LocalDataSource {
    table: Table,
}

impl LocalDataSource {
    /// Builds a source over the rows of a [`QueryResult`].
    ///
    /// This is how a partial answer from a remote source becomes something the
    /// local engine can finish (plan point 28): the hybrid path of 05-planner.md,
    /// over the coercion path E14 names. An empty result keeps its columns, so
    /// the remaining steps still know what they are working on.
    pub fn from_result(result: &QueryResult) -> Result<Self, DataSourceError> {
        let table =
            crate::ingest::load_result(result).map_err(|error| DataSourceError::Backend {
                message: error.to_string(),
            })?;
        Ok(Self::new(table))
    }

    /// Binds a table to the engine. The schema comes from the table — ingest
    /// produced it against an explicit schema, so the data is the authority
    /// here. An empty table (no rows) is still a table with its columns.
    pub fn new(table: Table) -> Self {
        Self { table }
    }

    /// The table it answers from — for what it costs (issue #70).
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Runs `query` and answers with the result table and its `total_count`,
    /// without turning a single cell into a value — the path of the binary
    /// result form (E35), which serialises the columns as they are.
    pub fn run(&self, query: &ValidatedQuery) -> Result<(Table, u64), DataSourceError> {
        let result = execute(&self.table, query).map_err(backend)?;
        // The binary form (E35) has no place yet for what a tree's rows are;
        // dropping it would answer a tree as a plain list. Plan point 123.
        if result.tree.is_some() {
            return Err(DataSourceError::Backend {
                message: "tree: a tree query answers in JSON, not yet in the binary form"
                    .to_owned(),
            });
        }
        Ok((result.table, result.total_count))
    }

    /// Runs `query` **once** and hands its rows out in pieces (issue #2, the
    /// server's export).
    ///
    /// Paging the query instead — one `offset`/`limit` run per piece — would
    /// sort the whole table again for every piece, so a million rows would be
    /// a hundred full sorts. Here the answer stays one table, which is as
    /// compact as the data it came from, and only the piece being written is
    /// turned into values.
    pub fn pieces(&self, query: &ValidatedQuery) -> Result<LocalPieces, DataSourceError> {
        let result = execute(&self.table, query).map_err(backend)?;
        Ok(LocalPieces {
            schema: query.output_schema.clone(),
            table: result.table,
            next: 0,
        })
    }
}

/// The answer of [`LocalDataSource::pieces`], handed out a piece at a time.
pub struct LocalPieces {
    schema: Schema,
    table: Table,
    next: usize,
}

impl LocalPieces {
    /// How many rows the pieces hold together — the rows of the query's
    /// answer, after its `offset` and `limit`.
    pub fn rows(&self) -> u64 {
        self.table.num_rows() as u64
    }

    /// The next piece of at most `rows` rows. A piece shorter than `rows` is
    /// the last one; after it, every piece is empty.
    pub fn next_piece(&mut self, rows: usize) -> Result<QueryResult, DataSourceError> {
        let start = self.next.min(self.table.num_rows());
        let length = rows.min(self.table.num_rows() - start);
        self.next = start + length;
        let columns = self.table.slice(start, length).to_values();
        Ok(QueryResult::new(self.schema.clone(), columns, self.rows()))
    }
}

impl SendDataSource for LocalDataSource {
    /// The table is `Send` and nothing is awaited, so the local source
    /// satisfies the server variant as well as the browser one (E5). The base
    /// `DataSource` comes from the macro's blanket impl.
    fn schema(&self) -> impl Future<Output = Result<Schema, DataSourceError>> + Send {
        let schema = self.table.schema().clone();
        async move { Ok(schema) }
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> impl Future<Output = Result<QueryResult, DataSourceError>> + Send {
        let table = &self.table;
        async move {
            let result = execute(table, &query).map_err(backend)?;
            let mut answer = QueryResult::new(
                query.output_schema,
                result.table.to_values(),
                result.total_count,
            );
            answer.tree = result.tree;
            Ok(answer)
        }
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities::ALL
    }
}

fn backend(error: crate::execute::ExecuteError) -> DataSourceError {
    DataSourceError::Backend {
        message: format!("local engine: {error}"),
    }
}
