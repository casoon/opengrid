//! A validated query becomes parameterized SQL.

use opengrid_pivot::ValidatedPivotQuery;
use opengrid_query::{Aggregate, CmpOp, Sort, SortDirection, ValidatedFilter, ValidatedQuery};
use opengrid_types::{DataType, Schema, Value};

use crate::dialect::{Dialect, Sql, quote_ident};
use crate::{Postgres, Sqlite};

/// SQL plus the values it expects, in placeholder order.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledQuery {
    /// The statement, with the dialect's placeholders where values go.
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
    pub fn count_of<D: Dialect>(
        query: &ValidatedQuery,
        compiler: &Compiler<D>,
    ) -> Result<Self, CompileError> {
        let mut counting = query.clone();
        counting.sort = Vec::new();
        counting.offset = None;
        counting.limit = None;
        let dialect = &compiler.dialect;

        // Aggregates without a grouping are **one** row — over no rows too,
        // the way SQL answers `SELECT count(*)` over an empty table — and the
        // local engine counts it so. Counting the filtered rows would be the
        // number of rows the aggregate read, not the rows it answers.
        if counting.group.is_empty() && !counting.aggregate.is_empty() {
            return Ok(CompiledQuery {
                sql: format!("SELECT {} AS \"total_count\"", dialect.one_row()),
                params: Vec::new(),
            });
        }

        if counting.group.is_empty() {
            let mut sql = Sql::new(dialect);
            sql.push("SELECT count(*) AS \"total_count\" FROM ");
            sql.push(&compiler.table_ref()?);
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

/// What a backend compiler does.
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
    /// A value the dialect cannot give the database (a decimal too wide for a
    /// 64-bit integer, say).
    Value { message: String },
    /// The dialect cannot do this (a capability it lacks).
    Unsupported { what: &'static str },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Identifier { name } => {
                write!(f, "identifier {name:?} cannot be quoted safely")
            }
            CompileError::UnknownField { name } => write!(f, "unknown field {name:?}"),
            CompileError::Value { message } => f.write_str(message),
            CompileError::Unsupported { what } => write!(f, "this dialect has no {what}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Compiles for one configured table, in the dialect `D`.
pub struct Compiler<D> {
    pub(crate) dialect: D,
    pub(crate) table: String,
    /// The schema the table lives in; none means the connection's search path.
    namespace: Option<String>,
    pub(crate) schema: Schema,
}

impl<D: Dialect + Default> Compiler<D> {
    /// A compiler for `table` with the columns of `schema`.
    ///
    /// Both come from the configuration and the source, never from a request.
    pub fn new(table: impl Into<String>, schema: Schema) -> Self {
        Self::with_dialect(D::default(), table, schema)
    }
}

impl Compiler<Postgres> {
    /// A compiler for PostgreSQL.
    pub fn postgres(table: impl Into<String>, schema: Schema) -> Self {
        Self::new(table, schema)
    }
}

impl Compiler<Sqlite> {
    /// A compiler for SQLite.
    pub fn sqlite(table: impl Into<String>, schema: Schema) -> Self {
        Self::new(table, schema)
    }
}

impl<D: Dialect> Compiler<D> {
    pub fn with_dialect(dialect: D, table: impl Into<String>, schema: Schema) -> Self {
        Self {
            dialect,
            table: table.into(),
            namespace: None,
            schema,
        }
    }

    /// The table lives in the database schema `namespace`: `"namespace"."table"`
    /// instead of `"table"`. Each part is quoted by itself, so a dot in a name
    /// stays a dot in the name (issue #222).
    pub fn in_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    pub fn dialect(&self) -> &D {
        &self.dialect
    }

    /// The schema the compiler reads columns from.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub(crate) fn sql(&self) -> Sql<'_> {
        Sql::new(&self.dialect)
    }

    /// The table as the statements name it.
    pub(crate) fn table_ref(&self) -> Result<String, CompileError> {
        match &self.namespace {
            None => quote_ident(&self.table),
            Some(namespace) => Ok(format!(
                "{}.{}",
                quote_ident(namespace)?,
                quote_ident(&self.table)?
            )),
        }
    }

    pub(crate) fn data_type(&self, name: &str) -> Result<DataType, CompileError> {
        self.schema
            .field(name)
            .map(|field| field.data_type)
            .ok_or_else(|| CompileError::UnknownField {
                name: name.to_owned(),
            })
    }

    /// How one column of the schema is **read** in SQL.
    ///
    /// Usually its quoted name. A derived column (plan point 54) has no column in
    /// the table, so it expands to the expression that computes it — here, and at
    /// every other place the name appears, which is what makes it behave like an
    /// ordinary column for filters, grouping and sorting alike.
    pub(crate) fn column(&self, name: &str) -> Result<String, CompileError> {
        let quoted = quote_ident(name)?;
        let Some(field) = self.schema.field(name) else {
            return Ok(quoted);
        };
        let Some(derivation) = &field.from else {
            return Ok(quoted);
        };
        let source = quote_ident(derivation.field.as_str())?;
        Ok(self.dialect.date_part(
            derivation.part,
            &source,
            self.schema.data_type(derivation.field.as_str()),
        ))
    }

    /// The output columns, in the order `ValidatedQuery::output_schema` declares.
    pub(crate) fn projection(
        &self,
        query: &ValidatedQuery,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
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
            sql.push(&quote_ident(name)?);
        }
        Ok(())
    }

    /// One aggregate, with the casts rule S12 demands.
    pub(crate) fn aggregate(&self, aggregate: &Aggregate) -> Result<String, CompileError> {
        let Some(field) = &aggregate.field else {
            // `count(*)` counts rows, `count("x")` counts non-NULL values — two
            // different questions (S11).
            return Ok("count(*)".to_owned());
        };
        let column = self.column(field.as_str())?;
        let data_type = self.data_type(field.as_str())?;
        Ok(self
            .dialect
            .aggregate(aggregate.function, &column, data_type))
    }

    pub(crate) fn filter(
        &self,
        filter: &ValidatedFilter,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        match filter {
            // An empty `and` is true, an empty `or` is false — the identity of
            // each operation, so an empty filter list never silently drops rows.
            ValidatedFilter::And(parts) if parts.is_empty() => {
                sql.push(self.dialect.true_literal());
            }
            ValidatedFilter::Or(parts) if parts.is_empty() => {
                sql.push(self.dialect.false_literal());
            }
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
                sql.push(&format!("{} IS NULL", self.column(field.as_str())?));
            }
            ValidatedFilter::IsNotNull { field, .. } => {
                sql.push(&format!("{} IS NOT NULL", self.column(field.as_str())?));
            }
            ValidatedFilter::Has { field, value } => {
                let column = self.column(field.as_str())?;
                self.dialect.has(&column, value, element_type(value), sql)?;
            }
            ValidatedFilter::HasAll { field, values } => {
                let column = self.column(field.as_str())?;
                let element = values.first().map_or(DataType::Utf8, element_type);
                self.dialect.has_all(&column, values, element, sql)?;
            }
            ValidatedFilter::IsEmpty { field } => {
                let column = self.column(field.as_str())?;
                sql.push(&self.dialect.is_empty_list(&column));
            }
            ValidatedFilter::InList {
                field,
                data_type,
                values,
            } => {
                if values.is_empty() {
                    // `x IN ()` is a syntax error, and the answer is "nothing".
                    sql.push(self.dialect.false_literal());
                    return Ok(());
                }
                sql.push(
                    &self
                        .dialect
                        .collate(&self.column(field.as_str())?, *data_type),
                );
                sql.push(" IN (");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        sql.push(", ");
                    }
                    sql.param(value, *data_type)?;
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
                    CmpOp::Contains | CmpOp::StartsWith | CmpOp::IContains | CmpOp::IStartsWith => {
                        self.dialect
                            .text_match(*op, &column, value, *data_type, sql)?;
                    }
                    _ => {
                        sql.push(&self.dialect.collate(&column, *data_type));
                        sql.push(&format!(" {} ", sql_operator(*op)));
                        sql.param(value, *data_type)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// One `ORDER BY` key: collation and NULL placement always written out.
    pub(crate) fn sort_key(
        &self,
        key: &Sort,
        query: &ValidatedQuery,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        // A sort key names an *output* column, which may be an aggregate alias.
        let data_type = query
            .output_schema
            .field(key.field.as_str())
            .map(|field| field.data_type)
            .ok_or_else(|| CompileError::UnknownField {
                name: key.field.as_str().to_owned(),
            })?;
        let column = quote_ident(key.field.as_str())?;

        match key.collation.locale() {
            // S4a. The tag is letters, digits and `-` (`Locale::parse`), so it
            // cannot leave the quotes.
            Some(locale) => sql.push(&self.dialect.locale_sort(&column, locale.as_str())),
            None => sql.push(&self.dialect.collate(&column, data_type)),
        }
        sql.push(match key.direction {
            SortDirection::Asc => " ASC",
            SortDirection::Desc => " DESC",
        });
        // S3: never rely on the default, which differs between the engines
        // exactly for DESC.
        sql.push(self.dialect.nulls(key.nulls));
        Ok(())
    }
}

impl<D: Dialect> QueryCompiler for Compiler<D> {
    type Output = CompiledQuery;
    type Error = CompileError;

    fn compile(&self, query: &ValidatedQuery) -> Result<CompiledQuery, CompileError> {
        let mut sql = self.sql();

        sql.push("SELECT ");
        self.projection(query, &mut sql)?;

        sql.push(" FROM ");
        sql.push(&self.table_ref()?);

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
                sql.push(&self.column(field.as_str())?);
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

        self.dialect.paging(query.limit, query.offset, &mut sql)?;
        Ok(sql.finish())
    }
}

/// The type of a list element a filter literal is: a string or an integer.
fn element_type(value: &Value) -> DataType {
    match value {
        Value::Int64(_) => DataType::Int64,
        _ => DataType::Utf8,
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
        CmpOp::In
        | CmpOp::Contains
        | CmpOp::StartsWith
        | CmpOp::IContains
        | CmpOp::IStartsWith
        | CmpOp::Has
        | CmpOp::HasAll => "=",
    }
}

/// The prefix of the `GROUPING()` columns a pivot statement carries.
///
/// A leading underscore is a legal identifier but not one a schema would use,
/// and the validator would have rejected a column of this name long before it
/// got here.
pub const GROUPING_PREFIX: &str = "__grouping_";

impl<D: Dialect> Compiler<D> {
    /// A whole pivot as **one** statement (plan point 31); only for a dialect
    /// with `GROUPING SETS`, otherwise the pivot takes the generic path.
    ///
    /// `GROUP BY GROUPING SETS` computes every level in a single pass instead of
    /// the `n+1` queries the generic path sends. Two things make the answer
    /// usable afterwards:
    ///
    /// * **`GROUPING(d)` per row dimension.** Without it a subtotal row — whose
    ///   deeper dimensions are NULL — is indistinguishable from a real NULL
    ///   group (S10). This is the SQL counterpart of `PivotResult::row_levels`,
    ///   and the reason rule P2 exists.
    /// * **An `ORDER BY` that produces rule P6.** For each row dimension, the
    ///   value first and its `GROUPING()` second: a subtotal ties with the NULL
    ///   group on the value and then loses on the flag, so it lands *after* the
    ///   rows it sums. The grand total lands last for the same reason.
    ///
    /// The statement answers the **long** form — one row per group, all levels
    /// in one table. Reshaping stays in `opengrid-pivot`.
    pub fn compile_pivot(
        &self,
        pivot: &ValidatedPivotQuery,
    ) -> Result<CompiledQuery, CompileError> {
        if !self.dialect.grouping_sets() {
            return Err(CompileError::Unsupported {
                what: "GROUPING SETS",
            });
        }
        let deepest = pivot.sets.first().ok_or(CompileError::UnknownField {
            name: "pivot".to_owned(),
        })?;
        let mut sql = self.sql();

        sql.push("SELECT ");
        self.projection(deepest, &mut sql)?;
        for field in &pivot.rows {
            sql.push(", GROUPING(");
            sql.push(&self.column(field.as_str())?);
            sql.push(") AS ");
            sql.push(&quote_ident(&format!(
                "{GROUPING_PREFIX}{}",
                field.as_str()
            ))?);
        }

        sql.push(" FROM ");
        sql.push(&self.table_ref()?);

        if let Some(filter) = &deepest.filter {
            sql.push(" WHERE ");
            self.filter(filter, &mut sql)?;
        }

        sql.push(" GROUP BY GROUPING SETS (");
        for (index, set) in pivot.sets.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.push("(");
            for (position, field) in set.group.iter().enumerate() {
                if position > 0 {
                    sql.push(", ");
                }
                sql.push(&self.column(field.as_str())?);
            }
            sql.push(")");
        }
        sql.push(")");

        sql.push(" ORDER BY ");
        let mut first = true;
        for field in &pivot.rows {
            let column = self.column(field.as_str())?;
            let data_type = self.data_type(field.as_str())?;
            if !first {
                sql.push(", ");
            }
            first = false;
            // S3/S4 as everywhere, then the flag that separates a real NULL
            // group from the subtotal that shares its NULL.
            sql.push(&self.dialect.collate(&column, data_type));
            sql.push(" ASC NULLS LAST, GROUPING(");
            sql.push(&column);
            sql.push(") ASC");
        }
        for field in &pivot.columns {
            let column = self.column(field.as_str())?;
            let data_type = self.data_type(field.as_str())?;
            if !first {
                sql.push(", ");
            }
            first = false;
            sql.push(&self.dialect.collate(&column, data_type));
            sql.push(" ASC NULLS LAST");
        }

        // One past what the sets may hold together (`max_cells` each): more
        // rows than that and some level is over the bound, which `assemble`
        // reports - without the database having sent the rest (issue #218).
        sql.push(&format!(
            " LIMIT {}",
            pivot
                .sets
                .len()
                .saturating_mul(pivot.limits.max_cells)
                .saturating_add(1)
        ));

        Ok(sql.finish())
    }
}
