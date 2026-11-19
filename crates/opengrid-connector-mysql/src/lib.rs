//! `opengrid-connector-mysql` — MySQL 8.0 and later as a [`Connector`] for
//! `opengrid-server` (issue #501).
//!
//! Queries go through the one SQL compiler (`opengrid-sql`) in its MySQL
//! dialect; the driver is `mysql_async`, and only this crate depends on it.
//!
//! # The connection
//!
//! A URL, `mysql://user:password@host:3306/database`. Every connection the
//! pool opens runs with `ANSI_QUOTES` added to its `sql_mode` (the compiler
//! quotes identifiers with `"`), the time zone `+00:00` (a `TIMESTAMP` is read
//! as the instant in UTC, rule S9) and `utf8mb4`. TLS is the URL's:
//! `?require_ssl=true` makes it required, verified against the system's roots
//! through rustls.
//!
//! # What the database answers, and what the engine does
//!
//! MySQL answers filters, sorts, groups, aggregates and pages. Where it cannot
//! answer exactly as the engine would — `icontains` and `istarts_with` (its
//! `LOWER` maps one character to one, rule S5a), a sort by a locale it has no
//! collation for (S4a) — the connector asks MySQL for the rows the filter
//! matches and the engine answers the query over them, at most
//! [`MAX_ENGINE_ROWS`] of them. A tree is always answered that way (MySQL
//! cannot read a temporary table twice in one statement), as is a pivot level
//! by level (MySQL has no `GROUPING SETS`).
//!
//! # The table
//!
//! A string column is `utf8mb4` (the compiler compares under
//! `utf8mb4_0900_bin`); a timestamp is a `TIMESTAMP`, the instant; a list is a
//! `JSON` array of strings or of integers. A `DOUBLE` holds no NaN, so rule S7
//! never comes up.

mod introspect;

use mysql_async::prelude::Queryable;
use mysql_async::{Opts, OptsBuilder, Params, Pool};
use opengrid_connector::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, QueryResult, Schema,
    ValidatedQuery,
};
use opengrid_sql::{
    CompileError, CompiledQuery, Compiler, MySql, QueryCompiler, mysql_read_as_text, quote_ident,
};
use opengrid_types::{DataType, Value};

pub use introspect::read_tables;

/// The shared SQL compiler in its MySQL dialect.
pub type MySqlCompiler = Compiler<MySql>;

/// The most rows the engine answers a query over when MySQL cannot answer it
/// by itself: about 300 MiB for ten columns. More is an error that says so.
pub const MAX_ENGINE_ROWS: u64 = 1_000_000;

/// What every connection runs when it is opened, and again after the pool
/// resets it.
const SETUP: &str = "SET SESSION sql_mode = CONCAT(@@SESSION.sql_mode, ',ANSI_QUOTES'), \
                     time_zone = '+00:00', NAMES utf8mb4";

/// A table in a MySQL database, behind the [`Connector`] contract.
pub struct MySqlConnector {
    pool: Pool,
    compiler: MySqlCompiler,
    schema: Schema,
    max_engine_rows: u64,
}

impl MySqlConnector {
    /// Connects to `url` (`mysql://…`) for `table`. Nothing is opened until the
    /// first query; a URL that does not parse is refused here.
    ///
    /// The schema comes from the configuration, not from the catalogue: it is
    /// the contract queries are validated against.
    pub fn connect(url: &str, table: &str, schema: Schema) -> Result<Self, DataSourceError> {
        let opts = Opts::from_url(url).map_err(backend)?;
        let opts = OptsBuilder::from_opts(opts).setup(vec![SETUP]);
        Ok(Self {
            pool: Pool::new(opts),
            compiler: MySqlCompiler::new(table, schema.clone()),
            schema,
            max_engine_rows: MAX_ENGINE_ROWS,
        })
    }

    /// The same for a table in another database than the URL's:
    /// `"database"."table"`.
    pub fn connect_in_database(
        url: &str,
        database: &str,
        table: &str,
        schema: Schema,
    ) -> Result<Self, DataSourceError> {
        let mut connector = Self::connect(url, table, schema)?;
        connector.compiler = connector.compiler.in_namespace(database);
        Ok(connector)
    }

    /// The most rows the engine answers one query over ([`MAX_ENGINE_ROWS`]).
    pub fn max_engine_rows(mut self, rows: u64) -> Self {
        self.max_engine_rows = rows;
        self
    }

    /// The compiler this connector uses — for tests and for inspecting the SQL.
    pub fn compiler(&self) -> &MySqlCompiler {
        &self.compiler
    }

    /// Closes the pool's connections.
    pub async fn disconnect(self) -> Result<(), DataSourceError> {
        self.pool.disconnect().await.map_err(backend)
    }

    /// Runs one compiled statement and answers its cells as text, or NULL.
    async fn rows(
        &self,
        compiled: &CompiledQuery,
        output: &[(String, DataType)],
    ) -> Result<Vec<Vec<Option<String>>>, DataSourceError> {
        let sql = wrap_for_reading(&compiled.sql, output).map_err(compile)?;
        let params = if compiled.params.is_empty() {
            Params::Empty
        } else {
            Params::Positional(compiled.params.iter().map(bind).collect())
        };
        let mut connection = self.pool.get_conn().await.map_err(backend)?;
        // Dropped before the answer (the server's timeout, a client that
        // left), the statement is killed, not left running.
        let guard = KillOnDrop {
            pool: self.pool.clone(),
            connection: connection.id(),
            armed: true,
        };
        let rows: Vec<mysql_async::Row> = connection.exec(sql, params).await.map_err(backend)?;
        guard.disarm();
        rows.into_iter()
            .map(|row| row.unwrap().into_iter().map(text).collect())
            .collect()
    }

    async fn execute_with_count(
        &self,
        query: ValidatedQuery,
        known_count: Option<u64>,
    ) -> Result<QueryResult, DataSourceError> {
        if query.tree.is_some() {
            return Connector::tree(self, &query).await;
        }
        let compiled = match self.compiler.compile(&query) {
            Ok(compiled) => compiled,
            Err(CompileError::Unsupported { .. }) => return self.engine_answers(&query).await,
            Err(error) => return Err(compile(error)),
        };
        let output = output_of(&query);
        let rows = self.rows(&compiled, &output).await?;
        // The rows before paging: a second statement, for the reasons in
        // `CompiledQuery::count_of`.
        let total_count = match known_count {
            Some(known) => known,
            None => {
                let counting = CompiledQuery::count_of(&query, &self.compiler).map_err(compile)?;
                let counted = self
                    .rows(&counting, &[("total_count".to_owned(), DataType::Int64)])
                    .await?;
                counted
                    .first()
                    .and_then(|row| row.first().cloned().flatten())
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(rows.len() as u64)
            }
        };
        Ok(QueryResult::new(
            query.output_schema.clone(),
            columns_of(rows, &output)?,
            total_count,
        ))
    }

    /// A query MySQL cannot answer as the engine would: MySQL sends the rows
    /// its filter matches — every row, if the filter is the part it cannot
    /// say — and the engine answers the query over them.
    async fn engine_answers(&self, query: &ValidatedQuery) -> Result<QueryResult, DataSourceError> {
        let schema = self.schema.materialized();
        let mut rows = ValidatedQuery {
            source: query.source.clone(),
            select: schema
                .fields()
                .iter()
                .map(|field| field.name.clone())
                .collect(),
            filter: query.filter.clone(),
            group: Vec::new(),
            aggregate: Vec::new(),
            sort: Vec::new(),
            offset: None,
            // One more than may be held, to know that there are more.
            limit: Some(self.max_engine_rows.saturating_add(1)),
            tree: None,
            output_schema: schema.clone(),
        };
        let compiled = match self.compiler.compile(&rows) {
            Err(CompileError::Unsupported { .. }) => {
                rows.filter = None;
                self.compiler.compile(&rows)
            }
            other => other,
        }
        .map_err(compile)?;
        let output = output_of(&rows);
        let cells = self.rows(&compiled, &output).await?;
        if cells.len() as u64 > self.max_engine_rows {
            return Err(DataSourceError::LimitExceeded {
                message: format!(
                    "more than {} rows for the engine to answer this query over \
                     (MySQL cannot answer it by itself); narrow the filter",
                    self.max_engine_rows
                ),
            });
        }
        let table = opengrid_columns::Table::from_values(&schema, &columns_of(cells, &output)?)
            .map_err(|message| DataSourceError::Backend { message })?;
        let answer =
            opengrid_engine::execute::execute(&table, query).map_err(DataSourceError::from)?;
        Ok(QueryResult::new(
            query.output_schema.clone(),
            answer.table.to_values(),
            answer.total_count,
        ))
    }
}

impl Connector for MySqlConnector {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        let schema = self.schema.clone();
        Box::pin(async move { Ok(schema) })
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            filter: true,
            sort: true,
            group: true,
            aggregate: true,
            paging: true,
            // Level by level, each one a statement.
            pivot: true,
            // A locale MySQL has no collation for is the engine's (S4a).
            collation: true,
            // The engine answers a tree over the scope's rows.
            tree: false,
            ..DataSourceCapabilities::default()
        }
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(self.execute_with_count(query, None))
    }

    fn execute_counted(
        &self,
        query: ValidatedQuery,
        known_count: Option<u64>,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        Box::pin(self.execute_with_count(query, known_count))
    }
}

/// Kills the statement running on `connection` when it is dropped armed: a
/// future dropped mid-statement would otherwise leave MySQL working on it,
/// holding the connection and its CPU.
struct KillOnDrop {
    pool: Pool,
    connection: u32,
    armed: bool,
}

impl KillOnDrop {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let pool = self.pool.clone();
        let kill = format!("KILL QUERY {}", self.connection);
        runtime.spawn(async move {
            if let Ok(mut connection) = pool.get_conn().await {
                let _ = connection.query_drop(kill).await;
            }
        });
    }
}

/// The output columns of a query, with their types.
fn output_of(query: &ValidatedQuery) -> Vec<(String, DataType)> {
    query
        .output_schema
        .fields()
        .iter()
        .map(|field| (field.name.as_str().to_owned(), field.data_type))
        .collect()
}

/// Wraps a compiled statement so every column comes back as text or NULL, in
/// the form the wire format reads.
fn wrap_for_reading(sql: &str, output: &[(String, DataType)]) -> Result<String, CompileError> {
    let mut projection = Vec::with_capacity(output.len());
    for (name, data_type) in output {
        let column = quote_ident(name)?;
        projection.push(format!(
            "{} AS {column}",
            mysql_read_as_text(&column, *data_type)
        ));
    }
    Ok(format!(
        "SELECT {} FROM ({sql}) AS \"result\"",
        projection.join(", ")
    ))
}

/// A parameter as MySQL takes it: numbers as numbers, everything else as the
/// text the compiler's cast reads (`CAST(? AS DECIMAL(12,2))`).
fn bind(value: &Value) -> mysql_async::Value {
    use mysql_async::Value as My;
    match value {
        Value::Null => My::NULL,
        Value::Bool(flag) => My::Int(i64::from(*flag)),
        Value::Int64(number) => My::Int(*number),
        Value::Float64(number) => My::Double(*number),
        Value::Decimal(decimal) => My::Bytes(decimal.to_string().into_bytes()),
        Value::Utf8(text) => My::Bytes(text.clone().into_bytes()),
        Value::Date(date) => My::Bytes(date.to_string().into_bytes()),
        // `2026-01-01T00:00:00.5Z` is read by `CAST(? AS DATETIME(6))` without
        // its zone; the session's zone is UTC.
        Value::Timestamp(timestamp) => My::Bytes(
            timestamp
                .to_string()
                .trim_end_matches('Z')
                .replacen('T', " ", 1)
                .into_bytes(),
        ),
        Value::List(_) => My::Bytes(
            opengrid_json::ToJson::to_json(value)
                .to_string()
                .into_bytes(),
        ),
    }
}

/// A cell MySQL sent as text, or NULL.
fn text(value: mysql_async::Value) -> Result<Option<String>, DataSourceError> {
    use mysql_async::Value as My;
    match value {
        My::NULL => Ok(None),
        My::Bytes(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|error| backend(format!("a cell is not UTF-8: {error}"))),
        // Every column is cast to text; a number here is still its digits.
        My::Int(number) => Ok(Some(number.to_string())),
        My::UInt(number) => Ok(Some(number.to_string())),
        My::Double(number) => Ok(Some(number.to_string())),
        My::Float(number) => Ok(Some(number.to_string())),
        other => Err(backend(format!(
            "a cell came back as {other:?}, not as text"
        ))),
    }
}

/// Rows of text, turned column-oriented in output order.
fn columns_of(
    rows: Vec<Vec<Option<String>>>,
    output: &[(String, DataType)],
) -> Result<Vec<Vec<Value>>, DataSourceError> {
    let mut columns = vec![Vec::with_capacity(rows.len()); output.len()];
    for row in rows {
        for (index, cell) in row.into_iter().enumerate() {
            let (name, data_type) = &output[index];
            columns[index].push(value_from_text(cell.as_deref(), *data_type, name)?);
        }
    }
    Ok(columns)
}

/// Reads one cell through the parser the wire format uses.
fn value_from_text(
    text: Option<&str>,
    data_type: DataType,
    column: &str,
) -> Result<Value, DataSourceError> {
    let Some(text) = text else {
        return Ok(Value::Null);
    };
    let json = match data_type {
        // A boolean is a `TINYINT(1)`: MySQL writes 1 and 0.
        DataType::Bool => match text {
            "1" => "true".to_owned(),
            "0" => "false".to_owned(),
            other => other.to_owned(),
        },
        // Numbers and lists are their JSON already.
        DataType::Int64 | DataType::Float64 | DataType::ListUtf8 | DataType::ListInt64 => {
            text.to_owned()
        }
        _ => opengrid_json::Json::from(text).to_string(),
    };
    let invalid = |error: &dyn std::fmt::Display| DataSourceError::Backend {
        message: format!("column {column:?}: {text:?} is not a {data_type:?}: {error}"),
    };
    let parsed = opengrid_json::Json::parse(&json).map_err(|error| invalid(&error))?;
    Value::from_json_typed(&parsed, &data_type).map_err(|error| invalid(&error))
}

fn backend(error: impl std::fmt::Display) -> DataSourceError {
    DataSourceError::Backend {
        message: error.to_string(),
    }
}

fn compile(error: CompileError) -> DataSourceError {
    DataSourceError::Backend {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Date, Decimal, Timestamp};

    #[test]
    fn a_value_survives_the_text_form_both_ways() {
        let cases = [
            (Value::Int64(-42), DataType::Int64, "-42"),
            (Value::Bool(true), DataType::Bool, "1"),
            (Value::Bool(false), DataType::Bool, "0"),
            (Value::Float64(1.5), DataType::Float64, "1.5"),
            (
                Value::Decimal(Decimal::new(-1050, 2)),
                DataType::decimal(12, 2).unwrap(),
                "-10.50",
            ),
            (
                Value::Date(Date::from_ymd(2026, 1, 1).unwrap()),
                DataType::Date,
                "2026-01-01",
            ),
            (
                Value::Timestamp(Timestamp::from_micros(1_767_225_600_000_001)),
                DataType::Timestamp,
                "2026-01-01T00:00:00.000001Z",
            ),
        ];
        for (value, data_type, from_mysql) in cases {
            assert_eq!(
                value_from_text(Some(from_mysql), data_type, "x").unwrap(),
                value,
                "reading {from_mysql:?}"
            );
        }
    }

    /// A statement dropped mid-way is killed, not left running (the server's
    /// timeout drops the future). Needs `OPENGRID_TEST_MYSQL`.
    #[tokio::test]
    async fn a_dropped_statement_is_killed() {
        let Ok(url) = std::env::var("OPENGRID_TEST_MYSQL") else {
            return;
        };
        let connector = MySqlConnector::connect(&url, "unused", Schema::new(Vec::new())).unwrap();
        let slow = CompiledQuery {
            sql: "SELECT SLEEP(20) AS \"x\"".to_owned(),
            params: Vec::new(),
        };
        let output = [("x".to_owned(), DataType::Int64)];
        let dropped = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            connector.rows(&slow, &output),
        )
        .await;
        assert!(dropped.is_err(), "the statement should still have run");

        let mut watcher = connector.pool.get_conn().await.unwrap();
        // Within a second: a dropped connection alone ends a `SLEEP` only
        // after several, and a heavy statement not until it next writes.
        let mut running = true;
        for _ in 0..10 {
            let sleeping: Vec<u64> = watcher
                .query(
                    "SELECT ID FROM information_schema.PROCESSLIST \
                     WHERE INFO LIKE 'SELECT CAST(%SLEEP(20)%'",
                )
                .await
                .unwrap();
            if sleeping.is_empty() {
                running = false;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(!running, "the dropped statement still runs");
    }

    /// The timestamp goes in without its zone, as `CAST(? AS DATETIME(6))`
    /// reads it.
    #[test]
    fn a_timestamp_is_bound_as_mysql_reads_it() {
        let value = Value::Timestamp(Timestamp::from_micros(1_767_225_600_000_001));
        assert_eq!(
            bind(&value),
            mysql_async::Value::Bytes(b"2026-01-01 00:00:00.000001".to_vec())
        );
    }
}
