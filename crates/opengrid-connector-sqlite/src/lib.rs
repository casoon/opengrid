//! `opengrid-connector-sqlite` — SQLite as a reference [`Connector`] (issue #51,
//! E36): a file, no installation, queries answered by SQLite itself.
//!
//! A reference, not a promise: it shows how a SQL database meets the query
//! semantics, and it is what the conformance suite runs against to prove the
//! point. The server does not depend on it.
//!
//! # The table
//!
//! SQLite has five storage classes and no decimal, date or NaN. The connector
//! reads a table laid out like this — [`create_table`] and [`insert`] write one:
//!
//! | opengrid type | Stored as |
//! |---|---|
//! | `bool` | `INTEGER` 0 or 1 |
//! | `int64` | `INTEGER` |
//! | `float64` | `REAL`; **NaN as the text `'NaN'`** (SQLite would store NaN as NULL) |
//! | `decimal(p, s)` | `INTEGER`, the coefficient: `12.34` in `decimal(12,2)` is `1234` — exact up to 18 digits |
//! | `utf8` | `TEXT` |
//! | `date` | `INTEGER`, days since 1970-01-01 |
//! | `timestamp` | `INTEGER`, microseconds since 1970-01-01 UTC |
//! | `list<utf8>`, `list<int64>` | `TEXT`, the JSON array; `has`, `has_all` and `is_empty` read it with `json_each` |
//!
//! Derived columns (`"from": { "part": "year", … }`) have no column; they are
//! computed in SQL.
//!
//! # Where SQLite meets the rules
//!
//! - **S7, NaN**: text sorts after every number in SQLite, so `'NaN'` does what
//!   the rule says — greater than any number, equal to itself, not NULL. `sum`
//!   and `avg` of a float column answer NaN when a NaN is in the set; SQLite
//!   alone would add the text as 0.
//! - **S3, NULL order**: `NULLS FIRST`/`LAST` written out.
//! - **S4/S13, strings**: SQLite's default `BINARY` collation is a byte
//!   comparison of UTF-8 — the order the engine uses. `contains` and
//!   `starts_with` use `instr`, which is case-sensitive; `LIKE` would not be.
//!   `icontains` and `istarts_with` (S5a) fold both sides with `og_fold`, a
//!   function the connector registers on its connection: SQLite's `lower` folds
//!   ASCII only.
//! - **S4a, a locale's collation**: SQLite has none. A sort key with a locale is
//!   `COLLATE "og_<locale>"`, and the connector registers that collation the
//!   first time a statement names it — the engine's own comparison (ICU4X,
//!   then bytes), so the order is the engine's by construction. No index
//!   serves such a sort.
//! - **S8, decimals**: integers throughout; a sum stays exact, an average is
//!   divided as a float (S12).
//!
//! # Blocking
//!
//! SQLite is a library, not a server: a query runs on the thread that calls it.
//! The connector keeps its connection on a thread of its own and sends each
//! query there (`worker`), so a slow query holds neither an async worker nor
//! the server's timeout. **A request that ends ends its query:** when the
//! future is dropped (the timeout, a client that left) a query that has not
//! started is skipped and one that runs is interrupted (`sqlite3_interrupt`).
//! Queries of one connector still run one after the other — one connection —
//! so a file next to the server and moderate traffic is what it is for.

use std::fmt::Write as _;
use std::sync::Arc;

mod tree;
mod worker;

use opengrid_connector::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, QueryResult, Schema,
    ValidatedQuery, Value,
};
use opengrid_sql::{CompileError, CompiledQuery, Dialect, QueryCompiler, Sqlite};
use opengrid_types::{DataType, Date, Decimal, Timestamp};
use rusqlite::types::Value as Sql;

pub use rusqlite::Connection;

/// A table in a SQLite database, answered by SQLite.
pub struct SqliteConnector {
    worker: worker::Worker,
    compiler: Arc<SqlCompiler>,
    schema: Schema,
    /// A tree's level in SQL rather than in the engine (issue #132); off
    /// unless asked for, see [`SqliteConnector::with_tree_pushdown`].
    tree_pushdown: bool,
}

/// The shared SQL compiler in its SQLite dialect (issue #485).
pub(crate) type SqlCompiler = opengrid_sql::Compiler<Sqlite>;

use opengrid_sql::{COLLATION, FOLD};

impl SqliteConnector {
    /// The table `table` in the database file at `path`, laid out for `schema`
    /// (module docs).
    pub fn open(
        path: impl AsRef<std::path::Path>,
        table: &str,
        schema: Schema,
    ) -> Result<Self, DataSourceError> {
        let connection = Connection::open(path).map_err(sqlite)?;
        Self::new(connection, table, schema)
    }

    /// The same over a connection the caller opened — an in-memory database,
    /// say.
    pub fn new(
        connection: Connection,
        table: &str,
        schema: Schema,
    ) -> Result<Self, DataSourceError> {
        quote(table)?;
        schema.check().map_err(|error| backend(error.to_string()))?;
        // SQLite's own `lower` folds ASCII only; the engine's rule (S5a) is
        // Unicode's, so the connector brings the engine's function.
        connection
            .create_scalar_function(
                FOLD,
                1,
                rusqlite::functions::FunctionFlags::SQLITE_UTF8
                    | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
                |context| {
                    Ok(context
                        .get::<Option<String>>(0)?
                        .map(|text| opengrid_query::fold_case(&text)))
                },
            )
            .map_err(sqlite)?;
        // A locale's collation (S4a) is registered when a statement first
        // names it: which locales a page will ask for is not known here.
        connection
            .collation_needed(|connection, name| {
                let Some(locale) = name.strip_prefix(COLLATION) else {
                    return Ok(());
                };
                let compare = opengrid_engine::collation::Icu::comparison(locale)
                    .map_err(|message| rusqlite::Error::UserFunctionError(message.into()))?;
                connection.create_collation(name, compare)
            })
            .map_err(sqlite)?;
        Ok(Self {
            worker: worker::Worker::start(connection),
            compiler: Arc::new(SqlCompiler::sqlite(table, schema.clone())),
            schema,
            tree_pushdown: false,
        })
    }

    /// Answers a tree's level in SQLite itself, with `WITH RECURSIVE`
    /// (issue #132), instead of handing the scope's rows to the engine.
    ///
    /// **Off by default, because it is slower** where the rows are close: the
    /// rules of a tree (T1, T3, T5) need the whole tree for every level, and
    /// the engine does that in one pass over rows it has read once. Measured
    /// on 100 000 nodes, a level takes about 75 ms the engine's way and
    /// 850–960 ms in SQL (`docs/guides/connectors.md`). Worth it only when
    /// moving the rows costs more than that.
    pub fn with_tree_pushdown(mut self, on: bool) -> Self {
        self.tree_pushdown = on;
        self
    }
}

/// One level of a tree, answered by SQLite (issue #132).
fn answer_tree(
    compiler: &SqlCompiler,
    connection: &Connection,
    query: &ValidatedQuery,
) -> Result<QueryResult, DataSourceError> {
    let tree = query.tree.as_ref().expect("a tree query");
    tree::level(compiler, connection, query, tree)
}

fn answer(
    compiler: &SqlCompiler,
    connection: &Connection,
    query: &ValidatedQuery,
    known_count: Option<u64>,
) -> Result<QueryResult, DataSourceError> {
    let statement = compiler.compile(query).map_err(compile)?;

    let fields = query.output_schema.fields();
    let mut columns: Vec<Vec<Value>> = vec![Vec::new(); fields.len()];
    let mut prepared = connection.prepare(&statement.sql).map_err(sqlite)?;
    let mut rows = prepared
        .query(rusqlite::params_from_iter(params(&statement)?.iter()))
        .map_err(sqlite)?;
    while let Some(row) = rows.next().map_err(sqlite)? {
        for (index, field) in fields.iter().enumerate() {
            let cell: Sql = row.get(index).map_err(sqlite)?;
            columns[index].push(read(cell, field.data_type, field.name.as_str())?);
        }
    }
    drop(rows);
    drop(prepared);

    let total = match known_count {
        Some(known) => known,
        None => {
            let counting = CompiledQuery::count_of(query, compiler).map_err(compile)?;
            let total: i64 = connection
                .query_row(
                    &counting.sql,
                    rusqlite::params_from_iter(params(&counting)?.iter()),
                    |row| row.get(0),
                )
                .map_err(sqlite)?;
            u64::try_from(total).unwrap_or(0)
        }
    };
    Ok(QueryResult::new(
        query.output_schema.clone(),
        columns,
        total,
    ))
}

impl Connector for SqliteConnector {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        let schema = self.schema.clone();
        Box::pin(async move { Ok(schema) })
    }

    /// Everything but a pivot in one statement, which takes the generic path —
    /// and, when asked for, a tree's level with `WITH RECURSIVE` (issue #132).
    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            filter: true,
            sort: true,
            group: true,
            aggregate: true,
            paging: true,
            tree: self.tree_pushdown,
            // The engine's collation, registered on the connection (S4a).
            collation: true,
            ..DataSourceCapabilities::default()
        }
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        self.execute_counted(query, None)
    }

    fn execute_counted(
        &self,
        query: ValidatedQuery,
        known_count: Option<u64>,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        match &query.tree {
            // A flat tree is the whole visible tree anyway (T8), and without
            // the pushdown every tree is the engine's: it answers from the
            // scope's rows, as for any source.
            Some(tree) if tree.flat || !self.tree_pushdown => {
                Box::pin(async move { Connector::tree(self, &query).await })
            }
            Some(_) => {
                let compiler = Arc::clone(&self.compiler);
                Box::pin(
                    self.worker
                        .submit(move |connection| answer_tree(&compiler, connection, &query)),
                )
            }
            None => {
                let compiler = Arc::clone(&self.compiler);
                Box::pin(
                    self.worker.submit(move |connection| {
                        answer(&compiler, connection, &query, known_count)
                    }),
                )
            }
        }
    }
}

/// The tables of the database `connection` holds and, for each, the columns
/// opengrid can serve (issue #486): `pragma table_info`, and a declared type
/// mapped only where the connector reads the column back exactly —
/// `INTEGER` as `int64`, `REAL` as `float64`, `TEXT` as `utf8`, `BOOLEAN` as
/// `bool`. Every other declared type is named and left out, not guessed:
/// `DECIMAL`, `DATE` and `DATETIME` hold text or numbers in a plain file, and
/// the connector's own layout (module docs) stores them as scaled integers —
/// a file written that way reads as `int64`, and its owner edits the type.
pub fn read_tables(
    connection: &Connection,
) -> Result<Vec<opengrid_connector::introspect::TableProposal>, DataSourceError> {
    use opengrid_connector::introspect::TableProposal;

    let mut tables = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(sqlite)?;
    let names: Vec<String> = tables
        .query_map([], |row| row.get(0))
        .map_err(sqlite)?
        .collect::<Result<_, _>>()
        .map_err(sqlite)?;

    let mut out = Vec::new();
    for name in names {
        let mut table = TableProposal::new(None, &name);
        let mut columns = connection
            .prepare(&format!("PRAGMA table_info({})", quote(&name)?))
            .map_err(sqlite)?;
        let rows = columns
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, i64>(5)? != 0,
                ))
            })
            .map_err(sqlite)?;
        for row in rows {
            let (column, declared, not_null, primary_key) = row.map_err(sqlite)?;
            table.column(
                &column,
                &declared,
                declared_type(&declared),
                !(not_null || primary_key),
            );
        }
        out.push(table);
    }
    Ok(out)
}

/// The opengrid type of a declared SQLite type, where the connector reads it
/// back as exactly that.
fn declared_type(declared: &str) -> Option<DataType> {
    let upper = declared.to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or("").trim();
    match base {
        "INTEGER" | "INT" | "BIGINT" | "SMALLINT" | "TINYINT" | "MEDIUMINT" | "INT8" | "INT2" => {
            Some(DataType::Int64)
        }
        "REAL" | "FLOAT" | "DOUBLE" | "DOUBLE PRECISION" => Some(DataType::Float64),
        "TEXT" | "VARCHAR" | "CHAR" | "CHARACTER" | "NVARCHAR" | "NCHAR" | "CLOB" => {
            Some(DataType::Utf8)
        }
        "BOOLEAN" | "BOOL" => Some(DataType::Bool),
        _ => None,
    }
}

/// Creates `table` for the **stored** columns of `schema`, in the layout the
/// connector reads (module docs).
pub fn create_table(
    connection: &Connection,
    table: &str,
    schema: &Schema,
) -> Result<(), DataSourceError> {
    let mut sql = format!("CREATE TABLE {} (", quote(table)?);
    for (index, field) in schema.stored().fields().iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        let storage = match field.data_type {
            DataType::Float64 => "REAL",
            // A list is its JSON array, as text (issue #185).
            DataType::Utf8 | DataType::ListUtf8 | DataType::ListInt64 => "TEXT",
            _ => "INTEGER",
        };
        let _ = write!(sql, "{} {storage}", quote(field.name.as_str())?);
    }
    sql.push(')');
    connection.execute(&sql, []).map_err(sqlite)?;
    Ok(())
}

/// Writes rows into `table`: `rows` holds the stored columns of `schema`.
pub fn insert(
    connection: &mut Connection,
    table: &str,
    schema: &Schema,
    rows: &QueryResult,
) -> Result<(), DataSourceError> {
    let stored = schema.stored();
    let placeholders = vec!["?"; stored.len()].join(", ");
    let sql = format!("INSERT INTO {} VALUES ({placeholders})", quote(table)?);
    let transaction = connection.transaction().map_err(sqlite)?;
    {
        let mut statement = transaction.prepare(&sql).map_err(sqlite)?;
        for row in 0..rows.row_count() {
            let values: Vec<Sql> = stored
                .fields()
                .iter()
                .zip(&rows.columns)
                .map(|(field, column)| bind(&column[row], field.data_type))
                .collect::<Result<_, _>>()?;
            statement
                .execute(rusqlite::params_from_iter(values.iter()))
                .map_err(sqlite)?;
        }
    }
    transaction.commit().map_err(sqlite)
}

/// The values of a compiled statement as SQLite binds them.
pub(crate) fn params(statement: &CompiledQuery) -> Result<Vec<Sql>, DataSourceError> {
    statement.params.iter().map(stored).collect()
}

/// A compile error as the connector's own (the same words as before the
/// compiler was shared).
pub(crate) fn compile(error: CompileError) -> DataSourceError {
    match error {
        CompileError::Identifier { name } => backend(format!(
            "{name:?} is not an identifier this connector quotes"
        )),
        CompileError::UnknownField { name } => backend(format!("no column {name:?}")),
        CompileError::Value { message } => backend(message),
        other => backend(other.to_string()),
    }
}

/// A value as SQLite stores it for a column of `data_type` (module docs).
///
/// A decimal that does not fit SQLite's 64-bit integer is a backend error,
/// not a wrapped number (issue #222).
fn bind(value: &Value, data_type: DataType) -> Result<Sql, DataSourceError> {
    stored(&Sqlite.param_value(value, data_type).map_err(compile)?)
}

/// A value the dialect has already brought to the column's scale.
fn stored(value: &Value) -> Result<Sql, DataSourceError> {
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(value) => Sql::Integer(i64::from(*value)),
        Value::Int64(value) => Sql::Integer(*value),
        Value::Float64(value) if value.is_nan() => Sql::Text("NaN".to_owned()),
        Value::Float64(value) => Sql::Real(*value),
        Value::Decimal(value) => Sql::Integer(
            i64::try_from(value.value())
                .map_err(|_| backend("a decimal does not fit SQLite's 64-bit integer"))?,
        ),
        Value::Utf8(value) => Sql::Text(value.clone()),
        Value::Date(value) => Sql::Integer(i64::from(value.days_since_epoch())),
        Value::Timestamp(value) => Sql::Integer(value.micros()),
        Value::List(_) => Sql::Text(opengrid_json::ToJson::to_json(value).to_string()),
    })
}

/// A cell back into the type the query declared.
fn read(cell: Sql, data_type: DataType, name: &str) -> Result<Value, DataSourceError> {
    let wrong = |cell: &Sql| backend(format!("column {name:?} ({data_type}) holds {cell:?}"));
    Ok(match (data_type, cell) {
        (_, Sql::Null) => Value::Null,
        (DataType::Bool, Sql::Integer(value)) => Value::Bool(value != 0),
        (DataType::Int64, Sql::Integer(value)) => Value::Int64(value),
        (DataType::Float64, Sql::Real(value)) => Value::Float64(value),
        (DataType::Float64, Sql::Integer(value)) => Value::Float64(value as f64),
        (DataType::Float64, Sql::Text(text)) => Value::Float64(match text.as_str() {
            "NaN" => f64::NAN,
            "Infinity" | "Inf" => f64::INFINITY,
            "-Infinity" | "-Inf" => f64::NEG_INFINITY,
            _ => return Err(wrong(&Sql::Text(text))),
        }),
        (DataType::Decimal { scale, .. }, Sql::Integer(value)) => {
            Value::Decimal(Decimal::new(i128::from(value), scale))
        }
        (DataType::Utf8, Sql::Text(text)) => Value::Utf8(text),
        (DataType::Date, Sql::Integer(days)) => Value::Date(Date::from_days_since_epoch(
            i32::try_from(days)
                .map_err(|_| backend(format!("column {name:?}: day {days} out of range")))?,
        )),
        (DataType::Timestamp, Sql::Integer(micros)) => {
            Value::Timestamp(Timestamp::from_micros(micros))
        }
        (DataType::ListUtf8 | DataType::ListInt64, Sql::Text(text)) => {
            let json =
                opengrid_json::Json::parse(&text).map_err(|_| wrong(&Sql::Text(text.clone())))?;
            Value::from_json_typed(&json, &data_type)
                .map_err(|error| backend(format!("column {name:?} ({data_type}): {error}")))?
        }
        (_, other) => return Err(wrong(&other)),
    })
}

/// Quotes an identifier; identifiers come from the schema and the caller,
/// never from a request.
fn quote(name: &str) -> Result<String, DataSourceError> {
    opengrid_sql::quote_ident(name).map_err(compile)
}

fn sqlite(error: rusqlite::Error) -> DataSourceError {
    backend(format!("SQLite: {error}"))
}

/// The data breaks a tree rule: the caller is told, it is no outage.
fn data(message: impl Into<String>) -> DataSourceError {
    DataSourceError::Data {
        message: message.into(),
    }
}

fn backend(message: impl Into<String>) -> DataSourceError {
    DataSourceError::Backend {
        message: message.into(),
    }
}
