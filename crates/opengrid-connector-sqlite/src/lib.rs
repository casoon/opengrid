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
//! - **S8, decimals**: integers throughout; a sum stays exact, an average is
//!   divided as a float (S12).
//!
//! # Blocking
//!
//! SQLite is a library, not a server: a query runs on the calling thread,
//! behind a mutex. Fine for a file next to the server and moderate traffic; a
//! busy server puts its SQLite work on a blocking thread pool.

use std::fmt::Write as _;
use std::sync::Mutex;

use opengrid_connector::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, QueryResult, Schema,
    ValidatedQuery, Value,
};
use opengrid_query::{
    Aggregate, AggregateFn, CmpOp, NullsOrder, Sort, SortDirection, ValidatedFilter,
};
use opengrid_types::{DataType, Date, DatePart, Decimal, Timestamp};
use rusqlite::types::Value as Sql;

pub use rusqlite::Connection;

/// A table in a SQLite database, answered by SQLite.
pub struct SqliteConnector {
    connection: Mutex<Connection>,
    table: String,
    schema: Schema,
}

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
        Ok(Self {
            connection: Mutex::new(connection),
            table: table.to_owned(),
            schema,
        })
    }

    fn answer(&self, query: &ValidatedQuery) -> Result<QueryResult, DataSourceError> {
        let compiler = Compiler {
            table: &self.table,
            schema: &self.schema,
        };
        let statement = compiler.query(query)?;
        let counting = compiler.count(query)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| backend("the connection is poisoned"))?;

        let fields = query.output_schema.fields();
        let mut columns: Vec<Vec<Value>> = vec![Vec::new(); fields.len()];
        let mut prepared = connection.prepare(&statement.sql).map_err(sqlite)?;
        let mut rows = prepared
            .query(rusqlite::params_from_iter(statement.params.iter()))
            .map_err(sqlite)?;
        while let Some(row) = rows.next().map_err(sqlite)? {
            for (index, field) in fields.iter().enumerate() {
                let cell: Sql = row.get(index).map_err(sqlite)?;
                columns[index].push(read(cell, field.data_type, field.name.as_str())?);
            }
        }
        drop(rows);
        drop(prepared);

        let total: i64 = connection
            .query_row(
                &counting.sql,
                rusqlite::params_from_iter(counting.params.iter()),
                |row| row.get(0),
            )
            .map_err(sqlite)?;
        Ok(QueryResult::new(
            query.output_schema.clone(),
            columns,
            u64::try_from(total).unwrap_or(0),
        ))
    }
}

impl Connector for SqliteConnector {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        let schema = self.schema.clone();
        Box::pin(async move { Ok(schema) })
    }

    /// Everything but a pivot in one statement, which takes the generic path.
    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            filter: true,
            sort: true,
            group: true,
            aggregate: true,
            paging: true,
            ..DataSourceCapabilities::default()
        }
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        let answer = self.answer(&query);
        Box::pin(async move { answer })
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
            DataType::Utf8 => "TEXT",
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
                .collect();
            statement
                .execute(rusqlite::params_from_iter(values.iter()))
                .map_err(sqlite)?;
        }
    }
    transaction.commit().map_err(sqlite)
}

/// SQL plus the values it expects.
struct Statement {
    sql: String,
    params: Vec<Sql>,
}

struct Compiler<'a> {
    table: &'a str,
    schema: &'a Schema,
}

/// The statement under construction: values only ever as `?`.
#[derive(Default)]
struct Builder {
    sql: String,
    params: Vec<Sql>,
}

impl Builder {
    fn push(&mut self, text: &str) {
        self.sql.push_str(text);
    }

    fn param(&mut self, value: &Value, data_type: DataType) {
        self.params.push(bind(value, data_type));
        self.sql.push('?');
    }

    fn finish(self) -> Statement {
        Statement {
            sql: self.sql,
            params: self.params,
        }
    }
}

impl Compiler<'_> {
    fn query(&self, query: &ValidatedQuery) -> Result<Statement, DataSourceError> {
        let mut sql = Builder::default();
        sql.push("SELECT ");
        for (index, field) in query.output_schema.fields().iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            let name = field.name.as_str();
            match query
                .aggregate
                .iter()
                .find(|aggregate| aggregate.alias.as_str() == name)
            {
                Some(aggregate) => sql.push(&self.aggregate(aggregate)?),
                None => sql.push(&self.column(name)?),
            }
            sql.push(" AS ");
            sql.push(&quote(name)?);
        }
        self.table_and_filter(query, &mut sql)?;
        if !query.group.is_empty() {
            sql.push(" GROUP BY ");
            let keys = query
                .group
                .iter()
                .map(|field| self.column(field.as_str()))
                .collect::<Result<Vec<_>, _>>()?;
            sql.push(&keys.join(", "));
        }
        if !query.sort.is_empty() {
            sql.push(" ORDER BY ");
            let keys = query
                .sort
                .iter()
                .map(sort_key)
                .collect::<Result<Vec<_>, _>>()?;
            sql.push(&keys.join(", "));
        }
        // SQLite knows `OFFSET` only after a `LIMIT`; -1 is "no limit".
        if query.limit.is_some() || query.offset.is_some() {
            sql.push(" LIMIT ");
            sql.param(
                &Value::Int64(query.limit.map_or(-1, |limit| limit as i64)),
                DataType::Int64,
            );
            if let Some(offset) = query.offset {
                sql.push(" OFFSET ");
                sql.param(&Value::Int64(offset as i64), DataType::Int64);
            }
        }
        Ok(sql.finish())
    }

    /// `total_count`: the rows before paging — one for an aggregate without
    /// groups, the groups with a grouping.
    fn count(&self, query: &ValidatedQuery) -> Result<Statement, DataSourceError> {
        if query.group.is_empty() && !query.aggregate.is_empty() {
            return Ok(Statement {
                sql: "SELECT 1".to_owned(),
                params: Vec::new(),
            });
        }
        if query.group.is_empty() {
            let mut sql = Builder::default();
            sql.push("SELECT count(*)");
            self.table_and_filter(query, &mut sql)?;
            return Ok(sql.finish());
        }
        let counting = ValidatedQuery {
            sort: Vec::new(),
            offset: None,
            limit: None,
            ..query.clone()
        };
        let inner = self.query(&counting)?;
        Ok(Statement {
            sql: format!("SELECT count(*) FROM ({})", inner.sql),
            params: inner.params,
        })
    }

    fn table_and_filter(
        &self,
        query: &ValidatedQuery,
        sql: &mut Builder,
    ) -> Result<(), DataSourceError> {
        sql.push(" FROM ");
        sql.push(&quote(self.table)?);
        if let Some(filter) = &query.filter {
            sql.push(" WHERE ");
            self.filter(filter, sql)?;
        }
        Ok(())
    }

    /// How a column is read: its quoted name, or for a derived one the
    /// expression that computes it (UTC, rule S9).
    fn column(&self, name: &str) -> Result<String, DataSourceError> {
        let quoted = quote(name)?;
        let Some(derivation) = self
            .schema
            .field(name)
            .and_then(|field| field.from.as_ref())
        else {
            return Ok(quoted);
        };
        let source = quote(derivation.field.as_str())?;
        let seconds = match self.schema.data_type(derivation.field.as_str()) {
            // Microseconds to whole seconds, rounded down also before 1970.
            Some(DataType::Timestamp) => {
                format!("({source} / 1000000 - ({source} % 1000000 < 0))")
            }
            _ => format!("({source} * 86400)"),
        };
        let part = match derivation.part {
            DatePart::Year => "%Y",
            DatePart::Month => "%m",
        };
        Ok(format!(
            "CAST(strftime('{part}', {seconds}, 'unixepoch') AS INTEGER)"
        ))
    }

    fn data_type(&self, name: &str) -> Result<DataType, DataSourceError> {
        self.schema
            .data_type(name)
            .ok_or_else(|| backend(format!("no column {name:?}")))
    }

    /// One aggregate, answering in the types rule S12 names.
    fn aggregate(&self, aggregate: &Aggregate) -> Result<String, DataSourceError> {
        let Some(field) = &aggregate.field else {
            return Ok("count(*)".to_owned());
        };
        let column = self.column(field.as_str())?;
        let data_type = self.data_type(field.as_str())?;
        // A float set with a NaN in it sums and averages to NaN (S7).
        let nan_guard = |expression: String| {
            format!(
                "CASE WHEN count(CASE WHEN typeof({column}) = 'text' THEN 1 END) > 0 \
                 THEN 'NaN' ELSE {expression} END"
            )
        };
        Ok(match (aggregate.function, data_type) {
            (AggregateFn::Count, _) => format!("count({column})"),
            (AggregateFn::Min, _) => format!("min({column})"),
            (AggregateFn::Max, _) => format!("max({column})"),
            (AggregateFn::Sum, DataType::Float64) => nan_guard(format!("sum({column})")),
            (AggregateFn::Sum, _) => format!("sum({column})"),
            (AggregateFn::Avg, DataType::Float64) => nan_guard(format!("avg({column})")),
            (AggregateFn::Avg, DataType::Decimal { scale, .. }) => {
                format!("(avg({column}) / {:.1})", 10f64.powi(i32::from(scale)))
            }
            (AggregateFn::Avg, _) => format!("CAST(avg({column}) AS REAL)"),
        })
    }

    fn filter(&self, filter: &ValidatedFilter, sql: &mut Builder) -> Result<(), DataSourceError> {
        match filter {
            ValidatedFilter::And(parts) if parts.is_empty() => sql.push("1"),
            ValidatedFilter::Or(parts) if parts.is_empty() => sql.push("0"),
            ValidatedFilter::And(parts) | ValidatedFilter::Or(parts) => {
                let joiner = if matches!(filter, ValidatedFilter::And(_)) {
                    " AND "
                } else {
                    " OR "
                };
                sql.push("(");
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        sql.push(joiner);
                    }
                    self.filter(part, sql)?;
                }
                sql.push(")");
            }
            ValidatedFilter::Not(inner) => {
                sql.push("(NOT ");
                self.filter(inner, sql)?;
                sql.push(")");
            }
            ValidatedFilter::IsNull { field } => {
                sql.push(&format!("{} IS NULL", self.column(field.as_str())?));
            }
            ValidatedFilter::IsNotNull { field } => {
                sql.push(&format!("{} IS NOT NULL", self.column(field.as_str())?));
            }
            ValidatedFilter::InList {
                field,
                data_type,
                values,
            } => {
                if values.is_empty() {
                    sql.push("0");
                    return Ok(());
                }
                sql.push(&self.column(field.as_str())?);
                sql.push(" IN (");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        sql.push(", ");
                    }
                    sql.param(value, *data_type);
                }
                sql.push(")");
            }
            ValidatedFilter::Cmp {
                field,
                data_type,
                op,
                value,
            } => {
                let column = self.column(field.as_str())?;
                match op {
                    // `instr` compares bytes, case-sensitively (S5).
                    CmpOp::Contains => {
                        sql.push(&format!("instr({column}, "));
                        sql.param(value, *data_type);
                        sql.push(") > 0");
                    }
                    CmpOp::StartsWith => {
                        sql.push(&format!("instr({column}, "));
                        sql.param(value, *data_type);
                        sql.push(") = 1");
                    }
                    _ => {
                        sql.push(&format!("{column} {} ", operator(*op)));
                        sql.param(value, *data_type);
                    }
                }
            }
        }
        Ok(())
    }
}

/// One `ORDER BY` key over an output column, NULL placement written out (S3).
fn sort_key(key: &Sort) -> Result<String, DataSourceError> {
    Ok(format!(
        "{} {} {}",
        quote(key.field.as_str())?,
        match key.direction {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        },
        match key.nulls {
            NullsOrder::First => "NULLS FIRST",
            NullsOrder::Last => "NULLS LAST",
        }
    ))
}

fn operator(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "<>",
        CmpOp::Lt => "<",
        CmpOp::Lte => "<=",
        CmpOp::Gt => ">",
        CmpOp::Gte => ">=",
        CmpOp::In | CmpOp::Contains | CmpOp::StartsWith => "=",
    }
}

/// A value as SQLite stores it for a column of `data_type` (module docs).
fn bind(value: &Value, data_type: DataType) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(value) => Sql::Integer(i64::from(*value)),
        Value::Int64(value) => Sql::Integer(*value),
        Value::Float64(value) if value.is_nan() => Sql::Text("NaN".to_owned()),
        Value::Float64(value) => Sql::Real(*value),
        Value::Decimal(value) => {
            let scale = match data_type {
                DataType::Decimal { scale, .. } => scale,
                _ => value.scale(),
            };
            let shift = u32::from(scale.saturating_sub(value.scale()));
            Sql::Integer((value.value() * 10i128.pow(shift)) as i64)
        }
        Value::Utf8(value) => Sql::Text(value.clone()),
        Value::Date(value) => Sql::Integer(i64::from(value.days_since_epoch())),
        Value::Timestamp(value) => Sql::Integer(value.micros()),
    }
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
        (_, other) => return Err(wrong(&other)),
    })
}

/// Quotes an identifier; identifiers come from the schema and the caller,
/// never from a request.
fn quote(name: &str) -> Result<String, DataSourceError> {
    if name.is_empty() || name.contains('"') || name.contains('\0') {
        return Err(backend(format!(
            "{name:?} is not an identifier this connector quotes"
        )));
    }
    Ok(format!("\"{name}\""))
}

fn sqlite(error: rusqlite::Error) -> DataSourceError {
    backend(format!("SQLite: {error}"))
}

fn backend(message: impl Into<String>) -> DataSourceError {
    DataSourceError::Backend {
        message: message.into(),
    }
}
