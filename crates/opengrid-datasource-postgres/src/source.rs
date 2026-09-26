//! Running a compiled query against a real PostgreSQL (plan point 26).
//!
//! # One binding path
//!
//! Every parameter goes out as **text or NULL**, and the cast the compiler wrote
//! next to the placeholder (`$1::text::numeric(12,2)`) tells PostgreSQL what to
//! make of it. That means no type-specific binding, no decimal or date crate in the
//! dependency list, and — the part that matters for rule R4 — no conversion
//! between two libraries' idea of a number: an exact decimal travels as the
//! digits it is.
//!
//! # Reading back
//!
//! The same trick in reverse. `bigint`, `double precision`, `boolean` and `text`
//! are read natively; `numeric`, `date` and `timestamptz` are turned into text
//! **by PostgreSQL** and parsed by the same code that reads the wire format
//! ([`Value::deserialize_typed`]), so there is exactly one place in the project
//! that knows how a decimal or a timestamp is written down. The casting happens
//! in a wrapper around the compiled statement, which leaves the compiled SQL of
//! point 25 — and its snapshots — untouched.
//!
//! `timestamptz` gets an explicit format rather than PostgreSQL's default
//! (`2026-01-01 00:00:00+00`), because rule S9 wants an ISO-8601 instant in UTC
//! with microseconds.

use deadpool_postgres::{Config, Pool, Runtime};
use opengrid_datasource::{DataSourceCapabilities, DataSourceError, QueryResult, SendDataSource};
use opengrid_query::ValidatedQuery;
use opengrid_types::{DataType, Schema, Value};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use opengrid_pivot::{PivotResult, ValidatedPivotQuery};

use crate::compiler::{
    CompileError, CompiledQuery, GROUPING_PREFIX, PostgresCompiler, QueryCompiler, quote_ident,
};

/// A table in a PostgreSQL database, behind the `DataSource` contract.
pub struct PostgresDataSource {
    pub(crate) pool: Pool,
    pub(crate) compiler: PostgresCompiler,
    schema: Schema,
}

impl PostgresDataSource {
    /// Connects to `url` (a libpq connection string) for `table`.
    ///
    /// The schema comes from the configuration, not from the catalogue: it is the
    /// contract the queries are validated against, and reading it from the
    /// database would make that contract change under the caller's feet.
    pub fn connect(url: &str, table: &str, schema: Schema) -> Result<Self, DataSourceError> {
        let mut config = Config::new();
        config.url = Some(url.to_owned());
        let pool = config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|error| DataSourceError::Backend {
                message: format!("connection pool: {error}"),
            })?;
        Ok(Self {
            pool,
            compiler: PostgresCompiler::new(table, schema.clone()),
            schema,
        })
    }

    /// How many connections the pool holds at most — what a server sizes its
    /// concurrent exports against, since each holds one for its whole length.
    pub fn pool_size(&self) -> usize {
        self.pool.status().max_size
    }

    /// The compiler this source uses — for tests and for inspecting the SQL.
    pub fn compiler(&self) -> &PostgresCompiler {
        &self.compiler
    }

    /// Runs one compiled statement and answers its rows as text per column.
    async fn rows(
        &self,
        compiled: &CompiledQuery,
        output: &[(String, DataType)],
    ) -> Result<Vec<Vec<Option<String>>>, DataSourceError> {
        let client = self.pool.get().await.map_err(backend)?;
        let sql = wrap_for_reading(&compiled.sql, output).map_err(compile)?;
        let params = text_params(&compiled.params);
        let refs: Vec<&(dyn ToSql + Sync)> = params
            .iter()
            .map(|param| param as &(dyn ToSql + Sync))
            .collect();

        let rows = client.query(&sql, &refs).await.map_err(backend)?;
        Ok(texts(&rows, output.len()))
    }
}

/// Every cell of `rows` as the text PostgreSQL wrote, or NULL.
pub(crate) fn texts(rows: &[tokio_postgres::Row], width: usize) -> Vec<Vec<Option<String>>> {
    rows.iter()
        .map(|row| {
            (0..width)
                .map(|index| row.get::<_, Option<String>>(index))
                .collect()
        })
        .collect()
}

/// Rows of text, turned column-oriented in output order (E14).
pub(crate) fn columns_of(
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

impl SendDataSource for PostgresDataSource {
    fn schema(&self) -> impl Future<Output = Result<Schema, DataSourceError>> + Send {
        let schema = self.schema.clone();
        async move { Ok(schema) }
    }

    async fn execute(&self, query: ValidatedQuery) -> Result<QueryResult, DataSourceError> {
        {
            let output: Vec<(String, DataType)> = query
                .output_schema
                .fields()
                .iter()
                .map(|field| (field.name.as_str().to_owned(), field.data_type))
                .collect();

            let compiled = self.compiler.compile(&query).map_err(compile)?;
            let rows = self.rows(&compiled, &output).await?;

            // `total_count` is the rows before paging — a second statement, for
            // the reasons in `CompiledQuery::count_of`.
            let counting = CompiledQuery::count_of(&query, &self.compiler).map_err(compile)?;
            let counted = self
                .rows(&counting, &[("total_count".to_owned(), DataType::Int64)])
                .await?;
            let total_count: u64 = counted
                .first()
                .and_then(|row| row.first().cloned().flatten())
                .and_then(|text| text.parse().ok())
                .unwrap_or(rows.len() as u64);

            let columns = columns_of(rows, &output)?;

            Ok(QueryResult::new(
                query.output_schema.clone(),
                columns,
                total_count,
            ))
        }
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        DataSourceCapabilities {
            filter: true,
            sort: true,
            group: true,
            aggregate: true,
            paging: true,
            // Since point 31 a pivot is one `GROUPING SETS` statement.
            pivot: true,
            calculated_fields: false,
            streaming: false,
        }
    }
}

/// Wraps a compiled statement so every column comes back as text or NULL.
///
/// The names are quoted by the compiler's own `quote_ident`, so this is safe
/// whatever reached it — not only because the compiler ran first and would
/// have refused the same name.
pub(crate) fn wrap_for_reading(
    sql: &str,
    output: &[(String, DataType)],
) -> Result<String, CompileError> {
    let mut projection = Vec::with_capacity(output.len());
    for (name, data_type) in output {
        let column = quote_ident(name)?;
        projection.push(match data_type {
                // PostgreSQL writes the value; the same parser the wire format
                // uses reads it back, so there is one notation in the project.
                DataType::Bool
                | DataType::Int64
                | DataType::Float64
                | DataType::Utf8
                | DataType::Decimal { .. }
                | DataType::Date => format!("{column}::text AS {column}"),
                // S9: ISO-8601 in UTC with microseconds, not PostgreSQL's default
                // `2026-01-01 00:00:00+00`.
                DataType::Timestamp => format!(
                    "to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS {column}"
                ),
        });
    }
    Ok(format!(
        "SELECT {} FROM ({sql}) AS \"result\"",
        projection.join(", ")
    ))
}

/// Every parameter as text or NULL — the cast in the statement gives it its type.
pub(crate) fn text_params(values: &[Value]) -> Vec<Option<String>> {
    values.iter().map(text_of).collect()
}

/// The text form of a value, the same one the wire format uses.
fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Int64(number) => Some(number.to_string()),
        Value::Float64(number) => Some(number.to_string()),
        Value::Decimal(decimal) => Some(decimal.to_string()),
        Value::Utf8(text) => Some(text.clone()),
        Value::Date(date) => Some(date.to_string()),
        Value::Timestamp(timestamp) => Some(timestamp.to_string()),
    }
}

/// Reads one cell, through the same parser the wire format uses.
fn value_from_text(
    text: Option<&str>,
    data_type: DataType,
    column: &str,
) -> Result<Value, DataSourceError> {
    let Some(text) = text else {
        return Ok(Value::Null);
    };
    // A JSON string is what `deserialize_typed` reads for every type whose text
    // form is a string; for numbers and booleans the raw token is the JSON.
    let json = match data_type {
        DataType::Int64 | DataType::Float64 | DataType::Bool => {
            // PostgreSQL writes `t`/`f` for booleans in text form.
            match (data_type, text) {
                (DataType::Bool, "t") => "true".to_owned(),
                (DataType::Bool, "f") => "false".to_owned(),
                // NaN and ±Infinity come back as those words (E13).
                (DataType::Float64, "NaN" | "Infinity" | "-Infinity") => {
                    format!("\"{text}\"")
                }
                _ => text.to_owned(),
            }
        }
        _ => serde_json::Value::String(text.to_owned()).to_string(),
    };

    let mut deserializer = serde_json::Deserializer::from_str(&json);
    Value::deserialize_typed(&mut deserializer, &data_type).map_err(|error| {
        DataSourceError::Backend {
            message: format!("column {column:?}: {text:?} is not a {data_type:?}: {error}"),
        }
    })
}

pub(crate) fn backend(error: impl std::fmt::Display) -> DataSourceError {
    DataSourceError::Backend {
        message: error.to_string(),
    }
}

pub(crate) fn compile(error: crate::compiler::CompileError) -> DataSourceError {
    DataSourceError::Backend {
        message: error.to_string(),
    }
}

impl PostgresDataSource {
    /// A whole pivot in **one** statement (plan point 31).
    ///
    /// The database computes every level at once; this method splits the long
    /// answer back into the levels the generic path would have fetched one by
    /// one, and hands them to the very same assembler
    /// ([`opengrid_pivot::assemble`]). The pushdown therefore changes how many
    /// round trips happen and nothing about what comes out — which is exactly
    /// what the differential test needs to be worth anything.
    pub async fn execute_pivot(
        &self,
        pivot: &ValidatedPivotQuery,
    ) -> Result<PivotResult, DataSourceError> {
        let deepest = pivot.sets.first().ok_or_else(|| DataSourceError::Backend {
            message: "a pivot has at least one level".to_owned(),
        })?;

        // The statement's columns: the deepest level's output, then one
        // `GROUPING()` flag per row dimension.
        let mut output: Vec<(String, DataType)> = deepest
            .output_schema
            .fields()
            .iter()
            .map(|field| (field.name.as_str().to_owned(), field.data_type))
            .collect();
        let flags = output.len();
        for field in &pivot.rows {
            output.push((
                format!("{GROUPING_PREFIX}{}", field.as_str()),
                DataType::Int64,
            ));
        }

        let compiled = self.compiler.compile_pivot(pivot).map_err(compile)?;
        let rows = self.rows(&compiled, &output).await?;

        let depth = pivot.rows.len();
        let across = pivot.columns.len();
        let measures = pivot.values.len();

        // One `QueryResult` per level, in the order `pivot.sets` declares them
        // (deepest first). A row belongs to the level whose leading dimensions
        // are the ones it does *not* aggregate away.
        let mut levels: Vec<Vec<Vec<Value>>> = vec![Vec::new(); pivot.sets.len()];
        for row in rows {
            let mut cells = Vec::with_capacity(flags);
            for (index, (name, data_type)) in output.iter().enumerate().take(flags) {
                cells.push(value_from_text(row[index].as_deref(), *data_type, name)?);
            }
            // The level is the number of leading dimensions still grouped by.
            let level = (0..depth)
                .take_while(|index| {
                    matches!(
                        value_from_text(row[flags + index].as_deref(), DataType::Int64, "grouping"),
                        Ok(Value::Int64(0))
                    )
                })
                .count();
            levels[depth - level].push(cells);
        }

        let results: Vec<QueryResult> = levels
            .into_iter()
            .enumerate()
            .map(|(index, rows)| {
                let level = depth - index;
                let schema = level_schema(deepest, level, across, measures);
                let count = rows.len() as u64;
                let mut columns = vec![Vec::with_capacity(rows.len()); schema.len()];
                for row in rows {
                    // A level keeps the row dimensions it still groups by, the
                    // column dimensions and the measures. The dimensions it
                    // aggregated away are **dropped**, not NULLed: the generic
                    // path would never have asked for them, and a NULL there
                    // would be a value (S10).
                    for (position, value) in row.into_iter().enumerate() {
                        if position < level {
                            columns[position].push(value);
                        } else if (depth..depth + across).contains(&position) {
                            columns[level + (position - depth)].push(value);
                        } else if position >= depth + across {
                            columns[level + across + (position - depth - across)].push(value);
                        }
                    }
                }
                QueryResult::new(schema, columns, count)
            })
            .collect();

        opengrid_pivot::assemble(pivot, &results).map_err(|error| DataSourceError::Backend {
            message: error.to_string(),
        })
    }
}

/// The output schema one level of a pivot would have had on its own.
fn level_schema(deepest: &ValidatedQuery, level: usize, across: usize, measures: usize) -> Schema {
    let fields = deepest.output_schema.fields();
    let keys = fields[..level]
        .iter()
        .chain(&fields[deepest.group.len() - across..deepest.group.len()])
        .cloned();
    let values = fields[fields.len() - measures..].iter().cloned();
    Schema::new(keys.chain(values).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Date, Decimal, Timestamp};

    #[test]
    fn a_value_survives_the_text_form_both_ways() {
        let cases = [
            (Value::Int64(-42), DataType::Int64, "-42"),
            (Value::Bool(true), DataType::Bool, "t"),
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
        for (value, data_type, from_pg) in cases {
            assert_eq!(
                value_from_text(Some(from_pg), data_type, "x").unwrap(),
                value,
                "reading {from_pg:?}"
            );
            assert!(text_of(&value).is_some());
        }
        assert_eq!(
            value_from_text(None, DataType::Int64, "x").unwrap(),
            Value::Null
        );
    }

    /// The three floats that are not numbers keep their names (E13).
    #[test]
    fn non_finite_floats_come_back_as_floats() {
        for (text, check) in [("NaN", true), ("Infinity", false), ("-Infinity", false)] {
            let value = value_from_text(Some(text), DataType::Float64, "ratio").unwrap();
            match value {
                Value::Float64(number) if check => assert!(number.is_nan()),
                Value::Float64(number) => assert!(number.is_infinite()),
                other => panic!("{text}: {other:?}"),
            }
        }
    }
}
