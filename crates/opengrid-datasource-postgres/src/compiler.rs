//! A validated query becomes parameterized SQL (plan point 25).
//!
//! Two properties matter more than anything else here, and both are structural
//! rather than a matter of care:
//!
//! * **No value ever reaches the SQL text.** Every literal becomes a placeholder
//!   (`$1`, `$2`, …) and travels in [`CompiledQuery::params`]. There is exactly
//!   one method that can put a value anywhere ([`Sql::param`]) and it can only
//!   produce a placeholder — so "did I forget to parameterize this one?" is not a
//!   question that can arise.
//! * **Identifiers come from the schema, never from the request.** A column name
//!   that reached this point was checked against the schema by validation; a
//!   table name comes from the configuration. Quoting rejects the one character
//!   that could break out ([`quote_ident`]).
//!
//! # Where PostgreSQL disagrees with the specification
//!
//! The rules S1–S14 are the contract, and PostgreSQL's defaults differ in four
//! places. The compiler makes each one explicit rather than relying on server
//! settings, a collation in the DDL, or a `search_path`:
//!
//! | Rule | PostgreSQL default | What is emitted |
//! |---|---|---|
//! | S3 | `NULLS LAST` for `ASC`, `NULLS FIRST` for `DESC` | `NULLS FIRST`/`NULLS LAST` always written out |
//! | S4 | the column's collation, usually locale-aware | `COLLATE "C"` on every string comparison and sort |
//! | S11 | — | `count(*)` and `count("field")` are different queries |
//! | S12 | `sum(bigint)` → `numeric`, `avg` → `numeric` | `::bigint`, `::numeric(38,s)`, `::double precision` |
//!
//! The shape is the one `plan/conformance-pg-abgleich.py` ran against PostgreSQL
//! 16 for all 48 conformance cases (48/48 exact, approved 2026-09-15). That
//! script inlines its literals because it is a throwaway oracle; this compiler
//! parameterizes them, which is the only intended difference.

use std::fmt::Write as _;

use opengrid_query::{
    Aggregate, AggregateFn, CmpOp, NullsOrder, Sort, SortDirection, ValidatedFilter, ValidatedQuery,
};
use opengrid_types::{DataType, Schema, Value};

/// SQL plus the values it expects, in placeholder order.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledQuery {
    /// The statement, with `$1`, `$2`, … where values go.
    pub sql: String,
    /// The values for those placeholders, in order.
    pub params: Vec<Value>,
}

impl CompiledQuery {
    /// The statement that answers `total_count` — the rows the filter matches
    /// **before** paging, which the grid needs for `aria-rowcount`.
    ///
    /// A **second statement**, not `count(*) OVER ()` in the first one. The
    /// window function looks cheaper — one round trip — but it is wrong for the
    /// case that matters: with `group`, `count(*) OVER ()` counts *groups in the
    /// current page*, not the groups the query has. It also forces the database
    /// to materialise the whole result before the `LIMIT` can help it.
    ///
    /// The second statement drops projection, sorting and paging and keeps the
    /// filter; with `group` it counts the groups through a subquery. Same
    /// parameters minus the paging ones, so the caller binds the same values.
    ///
    /// Trade-off, stated plainly: two round trips per page instead of one. Point
    /// 26 measures it; if it hurts, the answer is caching `total_count` per
    /// filter, not a wrong count.
    pub fn count_of(
        query: &ValidatedQuery,
        compiler: &PostgresCompiler,
    ) -> Result<Self, CompileError> {
        let mut counting = query.clone();
        counting.sort = Vec::new();
        counting.offset = None;
        counting.limit = None;

        if counting.group.is_empty() {
            let mut sql = Sql::default();
            sql.push("SELECT count(*) AS \"total_count\" FROM ");
            sql.push(&quote_ident(&compiler.table)?);
            if let Some(filter) = &counting.filter {
                sql.push(" WHERE ");
                compiler.filter(filter, &mut sql)?;
            }
            return Ok(sql.finish());
        }

        // With a grouping, the number of rows is the number of groups.
        let inner = compiler.compile(&counting)?;
        Ok(CompiledQuery {
            sql: format!(
                "SELECT count(*) AS \"total_count\" FROM ({}) AS \"grouped\"",
                inner.sql
            ),
            params: inner.params,
        })
    }
}

/// What a backend compiler does (plan/spezifikation/07-server.md
/// §Compiler-Pipeline).
///
/// One implementation today; MySQL and Mongo would lift this into a shared place
/// when they arrive.
pub trait QueryCompiler {
    type Output;
    type Error;

    fn compile(&self, query: &ValidatedQuery) -> Result<Self::Output, Self::Error>;
}

/// Why a query could not be compiled.
///
/// Short list on purpose: validation has already rejected everything that is
/// merely wrong. What is left are the things this compiler refuses to express.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// An identifier that cannot be quoted safely.
    Identifier { name: String },
    /// The query names a column the schema does not have. Validation prevents
    /// this; the compiler checks rather than guessing a type.
    UnknownField { name: String },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Identifier { name } => {
                write!(f, "identifier {name:?} cannot be quoted safely")
            }
            CompileError::UnknownField { name } => write!(f, "unknown field {name:?}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Compiles for one configured table.
pub struct PostgresCompiler {
    table: String,
    schema: Schema,
}

impl PostgresCompiler {
    /// A compiler for `table` with the columns of `schema`.
    ///
    /// Both come from the configuration and the source, never from a request.
    pub fn new(table: impl Into<String>, schema: Schema) -> Self {
        Self {
            table: table.into(),
            schema,
        }
    }

    fn data_type(&self, name: &str) -> Result<DataType, CompileError> {
        self.schema
            .field(name)
            .map(|field| field.data_type)
            .ok_or_else(|| CompileError::UnknownField {
                name: name.to_owned(),
            })
    }
}

impl QueryCompiler for PostgresCompiler {
    type Output = CompiledQuery;
    type Error = CompileError;

    fn compile(&self, query: &ValidatedQuery) -> Result<CompiledQuery, CompileError> {
        let mut sql = Sql::default();

        sql.push("SELECT ");
        self.projection(query, &mut sql)?;

        sql.push(" FROM ");
        sql.push(&quote_ident(&self.table)?);

        if let Some(filter) = &query.filter {
            sql.push(" WHERE ");
            self.filter(filter, &mut sql)?;
        }

        if !query.group.is_empty() {
            sql.push(" GROUP BY ");
            for (index, field) in query.group.iter().enumerate() {
                if index > 0 {
                    sql.push(", ");
                }
                sql.push(&quote_ident(field.as_str())?);
            }
        }

        if !query.sort.is_empty() {
            sql.push(" ORDER BY ");
            for (index, key) in query.sort.iter().enumerate() {
                if index > 0 {
                    sql.push(", ");
                }
                self.sort_key(key, query, &mut sql)?;
            }
        }

        // `offset` and `limit` are numbers the validator has already bounded, but
        // they are still values: they travel as parameters like everything else.
        if let Some(offset) = query.offset {
            sql.push(" OFFSET ");
            sql.param(Value::Int64(offset as i64));
        }
        if let Some(limit) = query.limit {
            sql.push(" LIMIT ");
            sql.param(Value::Int64(limit as i64));
        }

        Ok(sql.finish())
    }
}

impl PostgresCompiler {
    /// The output columns, in the order `ValidatedQuery::output_schema` declares.
    fn projection(&self, query: &ValidatedQuery, sql: &mut Sql) -> Result<(), CompileError> {
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
                Some(aggregate) => self.aggregate(aggregate, sql)?,
                None => sql.push(&quote_ident(name)?),
            }
            sql.push(" AS ");
            sql.push(&quote_ident(name)?);
        }
        Ok(())
    }

    /// One aggregate, with the casts rule S12 demands.
    fn aggregate(&self, aggregate: &Aggregate, sql: &mut Sql) -> Result<(), CompileError> {
        let Some(field) = &aggregate.field else {
            // `count(*)` counts rows, `count("x")` counts non-NULL values — two
            // different questions (S11).
            sql.push("count(*)");
            return Ok(());
        };
        let column = quote_ident(field.as_str())?;
        let data_type = self.data_type(field.as_str())?;

        match aggregate.function {
            AggregateFn::Count => {
                sql.push(&format!("count({column})"));
            }
            // S12: avg is Float64 whatever it sums, and PostgreSQL would answer
            // numeric for an integer or decimal input.
            AggregateFn::Avg => {
                sql.push(&format!("avg({column})::double precision"));
            }
            AggregateFn::Sum => match data_type {
                // S12: the sum of Int64 stays Int64; PostgreSQL widens to numeric.
                DataType::Int64 => sql.push(&format!("sum({column})::bigint")),
                // S12: the sum of a decimal widens to precision 38, scale kept.
                DataType::Decimal { scale, .. } => {
                    sql.push(&format!("sum({column})::numeric(38,{scale})"));
                }
                _ => sql.push(&format!("sum({column})")),
            },
            // min/max keep the input type (S12) — no cast needed.
            AggregateFn::Min => sql.push(&format!("min({column})")),
            AggregateFn::Max => sql.push(&format!("max({column})")),
        }
        Ok(())
    }

    fn filter(&self, filter: &ValidatedFilter, sql: &mut Sql) -> Result<(), CompileError> {
        match filter {
            // An empty `and` is true, an empty `or` is false — the identity of
            // each operation, so an empty filter list never silently drops rows.
            ValidatedFilter::And(parts) if parts.is_empty() => sql.push("TRUE"),
            ValidatedFilter::Or(parts) if parts.is_empty() => sql.push("FALSE"),
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
            ValidatedFilter::IsNull { field, .. } => {
                sql.push(&format!("{} IS NULL", quote_ident(field.as_str())?));
            }
            ValidatedFilter::IsNotNull { field, .. } => {
                sql.push(&format!("{} IS NOT NULL", quote_ident(field.as_str())?));
            }
            ValidatedFilter::InList {
                field,
                data_type,
                values,
            } => {
                if values.is_empty() {
                    // `x IN ()` is a syntax error, and the answer is "nothing".
                    sql.push("FALSE");
                    return Ok(());
                }
                sql.push(&collated(&quote_ident(field.as_str())?, *data_type));
                sql.push(" IN (");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        sql.push(", ");
                    }
                    sql.param(value.clone());
                }
                sql.push(")");
            }
            ValidatedFilter::Cmp {
                field,
                data_type,
                op,
                value,
            } => {
                let column = quote_ident(field.as_str())?;
                match op {
                    // Substring and prefix are exact byte comparisons in both
                    // engines (S5, case-sensitive), so no collation is involved.
                    CmpOp::Contains => {
                        sql.push(&format!("strpos({column}, "));
                        sql.param(value.clone());
                        sql.push(") > 0");
                    }
                    CmpOp::StartsWith => {
                        sql.push(&format!("starts_with({column}, "));
                        sql.param(value.clone());
                        sql.push(")");
                    }
                    _ => {
                        sql.push(&collated(&column, *data_type));
                        sql.push(&format!(" {} ", sql_operator(*op)));
                        sql.param(value.clone());
                    }
                }
            }
        }
        Ok(())
    }

    /// One `ORDER BY` key: collation and NULL placement always written out.
    fn sort_key(
        &self,
        key: &Sort,
        query: &ValidatedQuery,
        sql: &mut Sql,
    ) -> Result<(), CompileError> {
        // A sort key names an *output* column, which may be an aggregate alias.
        let data_type = query
            .output_schema
            .field(key.field.as_str())
            .map(|field| field.data_type)
            .ok_or_else(|| CompileError::UnknownField {
                name: key.field.as_str().to_owned(),
            })?;

        sql.push(&collated(&quote_ident(key.field.as_str())?, data_type));
        sql.push(match key.direction {
            SortDirection::Asc => " ASC",
            SortDirection::Desc => " DESC",
        });
        // S3: never rely on the default, which differs between the two engines
        // exactly for DESC.
        sql.push(match key.nulls {
            NullsOrder::First => " NULLS FIRST",
            NullsOrder::Last => " NULLS LAST",
        });
        Ok(())
    }
}

/// `COLLATE "C"` for strings, nothing for anything else (S4, E7).
///
/// Binary collation is the specification's only one in V1, and it is the only
/// way to get the same order out of PostgreSQL that the browser produces without
/// ICU.
fn collated(column: &str, data_type: DataType) -> String {
    match data_type {
        DataType::Utf8 => format!("{column} COLLATE \"C\""),
        _ => column.to_owned(),
    }
}

fn sql_operator(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "<>",
        CmpOp::Lt => "<",
        CmpOp::Lte => "<=",
        CmpOp::Gt => ">",
        CmpOp::Gte => ">=",
        // Handled before this point.
        CmpOp::In | CmpOp::Contains | CmpOp::StartsWith => "=",
    }
}

/// Quotes an identifier, refusing the one character that could end the quoting.
///
/// Identifiers only ever come from the schema or the configuration, so this can
/// never be reached by a request — it is the second lock on a door that should
/// already be closed.
fn quote_ident(name: &str) -> Result<String, CompileError> {
    if name.is_empty() || name.contains('"') || name.contains('\0') {
        return Err(CompileError::Identifier {
            name: name.to_owned(),
        });
    }
    Ok(format!("\"{name}\""))
}

/// The statement under construction.
///
/// The only way to place a value is [`Sql::param`], and it writes a placeholder.
/// That is what makes "no value in the SQL text" a property of the type rather
/// than a habit.
#[derive(Default)]
struct Sql {
    text: String,
    params: Vec<Value>,
}

impl Sql {
    /// Appends SQL text. Never a value — see [`Sql::param`].
    fn push(&mut self, text: &str) {
        self.text.push_str(text);
    }

    /// Appends a placeholder and remembers the value for it.
    fn param(&mut self, value: Value) {
        self.params.push(value);
        let _ = write!(self.text, "${}", self.params.len());
    }

    fn finish(self) -> CompiledQuery {
        CompiledQuery {
            sql: self.text,
            params: self.params,
        }
    }
}
