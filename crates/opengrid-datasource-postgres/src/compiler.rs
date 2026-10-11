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

use opengrid_pivot::ValidatedPivotQuery;
use opengrid_query::{
    Aggregate, AggregateFn, CmpOp, NullsOrder, Sort, SortDirection, ValidatedFilter, ValidatedQuery,
};
use opengrid_types::{DataType, DatePart, Schema, Value};

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

        // Aggregates without a grouping are **one** row — over no rows too,
        // the way SQL answers `SELECT count(*)` over an empty table — and the
        // local engine counts it so. Counting the filtered rows would be the
        // number of rows the aggregate read, not the rows it answers.
        if counting.group.is_empty() && !counting.aggregate.is_empty() {
            return Ok(CompiledQuery {
                sql: "SELECT 1::bigint AS \"total_count\"".to_owned(),
                params: Vec::new(),
            });
        }

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

        // `offset` and `limit` are numbers the validator has already bounded, but
        // they are still values: they travel as parameters like everything else.
        if let Some(offset) = query.offset {
            sql.push(" OFFSET ");
            sql.param(Value::Int64(offset as i64), DataType::Int64);
        }
        if let Some(limit) = query.limit {
            sql.push(" LIMIT ");
            sql.param(Value::Int64(limit as i64), DataType::Int64);
        }

        Ok(sql.finish())
    }
}

impl PostgresCompiler {
    /// How one column of the schema is **read** in SQL.
    ///
    /// Usually its quoted name. A derived column (plan point 54) has no column in
    /// the table, so it expands to the expression that computes it — here, and at
    /// every other place the name appears, which is what makes it behave like an
    /// ordinary column for filters, grouping and sorting alike.
    ///
    /// `EXTRACT` on a `timestamptz` uses the **session time zone**; rule S9 says
    /// UTC, so the value is pinned to UTC first. Without that, the same query
    /// would answer differently depending on the connection.
    fn column(&self, name: &str) -> Result<String, CompileError> {
        let quoted = quote_ident(name)?;
        let Some(field) = self.schema.field(name) else {
            return Ok(quoted);
        };
        let Some(derivation) = &field.from else {
            return Ok(quoted);
        };
        let source = quote_ident(derivation.field.as_str())?;
        let instant = match self.schema.data_type(derivation.field.as_str()) {
            Some(DataType::Timestamp) => format!("{source} AT TIME ZONE 'UTC'"),
            _ => source,
        };
        Ok(format!(
            "EXTRACT({} FROM {instant})::bigint",
            match derivation.part {
                DatePart::Year => "YEAR",
                DatePart::Month => "MONTH",
            }
        ))
    }

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
                None => sql.push(&self.column(name)?),
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
        let column = self.column(field.as_str())?;
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
                sql.push(&format!("{} IS NULL", self.column(field.as_str())?));
            }
            ValidatedFilter::IsNotNull { field, .. } => {
                sql.push(&format!("{} IS NOT NULL", self.column(field.as_str())?));
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
                sql.push(&collated(&self.column(field.as_str())?, *data_type));
                sql.push(" IN (");
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        sql.push(", ");
                    }
                    sql.param(value.clone(), *data_type);
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
                    // Substring and prefix are exact byte comparisons in both
                    // engines (S5, case-sensitive), so no collation is involved.
                    CmpOp::Contains => {
                        sql.push(&format!("strpos({column}, "));
                        sql.param(value.clone(), *data_type);
                        sql.push(") > 0");
                    }
                    CmpOp::StartsWith => {
                        sql.push(&format!("starts_with({column}, "));
                        sql.param(value.clone(), *data_type);
                        sql.push(")");
                    }
                    _ => {
                        sql.push(&collated(&column, *data_type));
                        sql.push(&format!(" {} ", sql_operator(*op)));
                        sql.param(value.clone(), *data_type);
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

/// The PostgreSQL type of an opengrid type — the cast on every parameter, and
/// the column type the test fixture creates.
pub fn pg_type(data_type: DataType) -> String {
    match data_type {
        DataType::Bool => "boolean".to_owned(),
        DataType::Int64 => "bigint".to_owned(),
        DataType::Float64 => "double precision".to_owned(),
        DataType::Decimal { precision, scale } => format!("numeric({precision},{scale})"),
        DataType::Utf8 => "text".to_owned(),
        DataType::Date => "date".to_owned(),
        // S9: an instant in UTC with microsecond resolution.
        DataType::Timestamp => "timestamptz".to_owned(),
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
pub(crate) fn quote_ident(name: &str) -> Result<String, CompileError> {
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

    /// Appends a placeholder **with its type** and remembers the value for it.
    ///
    /// The cast chain is what lets every parameter travel as text.
    ///
    /// `$1::numeric` would not do it: the cast makes PostgreSQL *expect* a
    /// numeric parameter, and a text binding is then refused before the query
    /// runs. `$1::text::numeric(12,2)` says what is actually true — text goes in,
    /// a numeric comes out — so one binding path serves every type (point 26),
    /// no decimal or date crate is needed, and the snapshot shows a reviewer what
    /// each parameter is meant to become.
    fn param(&mut self, value: Value, data_type: DataType) {
        self.params.push(value);
        let target = pg_type(data_type);
        let _ = if target == "text" {
            write!(self.text, "${}::text", self.params.len())
        } else {
            write!(self.text, "${}::text::{target}", self.params.len())
        };
    }

    fn finish(self) -> CompiledQuery {
        CompiledQuery {
            sql: self.text,
            params: self.params,
        }
    }
}

/// The prefix of the `GROUPING()` columns a pivot statement carries.
///
/// A leading underscore is a legal identifier but not one a schema would use,
/// and the validator would have rejected a column of this name long before it
/// got here.
pub const GROUPING_PREFIX: &str = "__grouping_";

impl PostgresCompiler {
    /// A whole pivot as **one** statement (plan point 31).
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
    /// in one table. Reshaping stays in `opengrid-pivot`, so there is exactly
    /// one place that builds a matrix and the differential test compares the
    /// database against the engine rather than two reshapers against each other.
    pub fn compile_pivot(
        &self,
        pivot: &ValidatedPivotQuery,
    ) -> Result<CompiledQuery, CompileError> {
        let deepest = pivot.sets.first().ok_or(CompileError::UnknownField {
            name: "pivot".to_owned(),
        })?;
        let mut sql = Sql::default();

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
        sql.push(&quote_ident(&self.table)?);

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
            sql.push(&collated(&column, data_type));
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
            sql.push(&collated(&column, data_type));
            sql.push(" ASC NULLS LAST");
        }

        Ok(sql.finish())
    }
}

/// The columns a tree's statements carry beside the table's own (issue #132).
const TREE_KEY: &str = "__og_key";
const TREE_PARENT: &str = "__og_parent";
const TREE_MATCH: &str = "__og_match";
const TREE_ROW: &str = "__og_row";

/// The extra output columns of a tree's level: each node's shown children,
/// whether it matches, and its place on the page.
pub(crate) const TREE_CHILDREN: &str = "__og_children";
pub(crate) const TREE_MATCHED: &str = "__og_matched";
const TREE_POSITION: &str = "__og_position";

/// The temporary table a tree's statements read, dropped with the
/// transaction that made it.
const TREE_TABLE: &str = "og_tree_scoped";
/// The source's own table, joined by `ctid` for the columns a level shows.
const TREE_BASE: &str = "og_base";

/// The indexes on [`TREE_TABLE`], and the statistics the planner needs to use
/// them: every recursion joins a node to its parent or its children, and
/// without them each step would hash the whole tree.
pub(crate) const TREE_INDEXES: &str = "CREATE INDEX ON og_tree_scoped (__og_key); \
     CREATE INDEX ON og_tree_scoped (__og_parent); \
     CREATE INDEX ON og_tree_scoped (__og_row); ANALYZE og_tree_scoped;";

/// One level of a tree as SQL (issue #132, rules T1–T7). The tree's rows —
/// each one's `ctid` as its identity (a NULL key cannot be one), key, parent
/// and whether it matches the filter — are copied once into a temporary
/// table under the tree's scope ([`Self::tree_prepare`]); every statement then
/// reads it as `scoped`, and the source's table only for the columns a level
/// shows. `UNION` in every recursion, so a cycle ends the walk; T3 names it.
impl PostgresCompiler {
    /// `CREATE TEMP TABLE … ON COMMIT DROP AS …`: the tree's rows.
    pub(crate) fn tree_prepare(
        &self,
        query: &ValidatedQuery,
        tree: &opengrid_query::ValidatedTree,
    ) -> Result<CompiledQuery, CompileError> {
        let mut sql = Sql::default();
        sql.push(&format!(
            "CREATE TEMP TABLE {TREE_TABLE} ON COMMIT DROP AS SELECT ctid AS {TREE_ROW}, {} AS {TREE_KEY}, {} AS {TREE_PARENT}, ",
            self.column(tree.key.as_str())?,
            self.column(tree.parent.as_str())?
        ));
        match &query.filter {
            Some(filter) => {
                sql.push("COALESCE(");
                self.filter(filter, &mut sql)?;
                sql.push(", FALSE)");
            }
            None => sql.push("TRUE"),
        }
        sql.push(&format!(" AS {TREE_MATCH} FROM "));
        sql.push(&quote_ident(&self.table)?);
        if let Some(scope) = &tree.scope {
            sql.push(" WHERE ");
            self.filter(scope, &mut sql)?;
        }
        Ok(sql.finish())
    }

    /// `WITH RECURSIVE scoped AS (…)`: the tree's rows as prepared —
    /// `NOT MATERIALIZED`, or PostgreSQL copies a CTE named more than once into
    /// a table of its own, without the indexes, and sorts the whole tree in
    /// every step of a recursion (12 s instead of 0.1 s at 100 000 nodes).
    fn tree_scoped(sql: &mut Sql) {
        sql.push(&format!(
            "WITH RECURSIVE scoped AS NOT MATERIALIZED (SELECT * FROM {TREE_TABLE})"
        ));
    }

    /// The key of a row as text, the way every other value is read back.
    fn tree_key_text(&self, tree: &opengrid_query::ValidatedTree) -> Result<String, CompileError> {
        Ok(read_as_text(TREE_KEY, self.data_type(tree.key.as_str())?))
    }

    /// T1: one key that appears more than once, as text — none if every key
    /// is there once.
    pub(crate) fn tree_duplicate_key(
        &self,
        tree: &opengrid_query::ValidatedTree,
    ) -> Result<CompiledQuery, CompileError> {
        let mut sql = Sql::default();
        Self::tree_scoped(&mut sql);
        sql.push(&format!(
            " SELECT {} FROM scoped WHERE {TREE_KEY} IS NOT NULL GROUP BY {TREE_KEY} \
             HAVING count(*) > 1 ORDER BY {TREE_KEY} LIMIT 1",
            self.tree_key_text(tree)?
        ));
        Ok(sql.finish())
    }

    /// T3: a node of a cycle, as text — none if every node is reached from a
    /// root. A node not reached lies on a cycle or below one; the walk up from
    /// it finds the one that is its own ancestor.
    pub(crate) fn tree_cycle(
        &self,
        tree: &opengrid_query::ValidatedTree,
    ) -> Result<CompiledQuery, CompileError> {
        let mut sql = Sql::default();
        Self::tree_scoped(&mut sql);
        sql.push(&format!(
            ", reached(__og_r) AS (SELECT c.{TREE_ROW} FROM scoped c WHERE {root} \
             UNION SELECT c.{TREE_ROW} FROM reached JOIN scoped p ON p.{TREE_ROW} = reached.__og_r \
             JOIN scoped c ON c.{TREE_PARENT} = p.{TREE_KEY}), \
             lost(__og_r) AS (SELECT s.{TREE_ROW} FROM scoped s WHERE NOT EXISTS \
             (SELECT 1 FROM reached WHERE reached.__og_r = s.{TREE_ROW})), \
             up(__og_s, __og_r, __og_d) AS (SELECT __og_r, __og_r, 0 FROM lost \
             UNION SELECT up.__og_s, p.{TREE_ROW}, up.__og_d + 1 FROM up JOIN scoped c ON c.{TREE_ROW} = up.__og_r \
             JOIN scoped p ON p.{TREE_KEY} = c.{TREE_PARENT} WHERE up.__og_d < (SELECT count(*) FROM lost)) \
             SELECT {key} FROM up JOIN scoped ON scoped.{TREE_ROW} = up.__og_s \
             WHERE up.__og_r = up.__og_s AND up.__og_d > 0 ORDER BY up.__og_s LIMIT 1",
            root = tree_root("c"),
            key = self.tree_key_text(tree)?,
        ));
        Ok(sql.finish())
    }

    /// T2 and T5: the orphans and the matches of the whole tree, and — when
    /// the query names a node — whether that node exists (T4).
    pub(crate) fn tree_counts(
        &self,
        tree: &opengrid_query::ValidatedTree,
    ) -> Result<CompiledQuery, CompileError> {
        let mut sql = Sql::default();
        Self::tree_scoped(&mut sql);
        sql.push(&format!(
            " SELECT (SELECT count(*) FROM scoped c WHERE c.{TREE_PARENT} IS NOT NULL AND NOT EXISTS \
             (SELECT 1 FROM scoped p WHERE p.{TREE_KEY} = c.{TREE_PARENT}))::text, \
             (SELECT count(*) FROM scoped WHERE {TREE_MATCH})::text, "
        ));
        match &tree.under {
            Some(under) => {
                sql.push(&format!("(SELECT count(*) FROM scoped WHERE {TREE_KEY} = "));
                sql.param(under.clone(), self.data_type(tree.key.as_str())?);
                sql.push(")::text");
            }
            None => sql.push("'1'"),
        }
        Ok(sql.finish())
    }

    /// `, shown(__og_r), members(__og_r)`: the matches with their ancestors (T5), and of
    /// those the children of `under`, or the roots (T4).
    fn tree_members(
        &self,
        tree: &opengrid_query::ValidatedTree,
        sql: &mut Sql,
    ) -> Result<(), CompileError> {
        sql.push(&format!(
            ", shown(__og_r) AS (SELECT {TREE_ROW} FROM scoped WHERE {TREE_MATCH} \
             UNION SELECT p.{TREE_ROW} FROM shown JOIN scoped c ON c.{TREE_ROW} = shown.__og_r \
             JOIN scoped p ON p.{TREE_KEY} = c.{TREE_PARENT}), \
             members(__og_r) AS (SELECT c.{TREE_ROW} FROM scoped c JOIN shown ON shown.__og_r = c.{TREE_ROW} WHERE "
        ));
        match &tree.under {
            Some(under) => {
                sql.push(&format!("c.{TREE_PARENT} = "));
                sql.param(under.clone(), self.data_type(tree.key.as_str())?);
            }
            None => sql.push(&tree_root("c")),
        }
        sql.push(")");
        Ok(())
    }

    /// How many nodes the level has, before paging (T6).
    pub(crate) fn tree_total(
        &self,
        tree: &opengrid_query::ValidatedTree,
    ) -> Result<CompiledQuery, CompileError> {
        let mut sql = Sql::default();
        Self::tree_scoped(&mut sql);
        self.tree_members(tree, &mut sql)?;
        sql.push(" SELECT count(*)::text FROM members");
        Ok(sql.finish())
    }

    /// The level itself: the query's columns, each node's shown children and
    /// whether it matches, sorted among siblings (T6, ties by the key) and
    /// paged — and, asked for, each node's subtree aggregates over the
    /// matches below and at it (T7). Answers the statement and its output
    /// columns, every one read back as text.
    pub(crate) fn tree_level(
        &self,
        query: &ValidatedQuery,
        tree: &opengrid_query::ValidatedTree,
    ) -> Result<(CompiledQuery, Vec<(String, DataType)>), CompileError> {
        let mut sql = Sql::default();
        Self::tree_scoped(&mut sql);
        self.tree_members(tree, &mut sql)?;
        sql.push(&format!(
            ", kids(__og_r, __og_n) AS (SELECT p.{TREE_ROW}, count(*) FROM shown JOIN scoped c ON c.{TREE_ROW} = shown.__og_r \
             JOIN scoped p ON p.{TREE_KEY} = c.{TREE_PARENT} GROUP BY p.{TREE_ROW}), \
             level AS (SELECT "
        ));
        for field in query.output_schema.fields() {
            sql.push(&self.column(field.name.as_str())?);
            sql.push(" AS ");
            sql.push(&quote_ident(field.name.as_str())?);
            sql.push(", ");
        }
        sql.push(&format!(
            "coalesce(kids.__og_n, 0) AS {TREE_CHILDREN}, scoped.{TREE_MATCH} AS {TREE_MATCHED}, \
             scoped.{TREE_KEY} AS {TREE_KEY}, scoped.{TREE_ROW} AS {TREE_ROW} FROM members \
             JOIN scoped ON scoped.{TREE_ROW} = members.__og_r \
             JOIN {base} AS {TREE_BASE} ON {TREE_BASE}.ctid = scoped.{TREE_ROW} LEFT JOIN kids ON kids.__og_r = scoped.{TREE_ROW}), \
             page AS MATERIALIZED (SELECT *, row_number() OVER (ORDER BY ",
            base = quote_ident(&self.table)?
        ));
        // The order once for the numbering and once for the paging: an output
        // column is an input column of `level`, so its collation can be written.
        let mut order = Sql::default();
        for key in &query.sort {
            self.sort_key(key, query, &mut order)?;
            order.push(", ");
        }
        let key_type = self.data_type(tree.key.as_str())?;
        order.push(&collated(TREE_KEY, key_type));
        order.push(" ASC NULLS LAST");
        sql.push(&order.text);
        sql.push(&format!(") AS {TREE_POSITION} FROM level ORDER BY "));
        sql.push(&order.text);
        if let Some(offset) = query.offset {
            sql.push(" OFFSET ");
            sql.param(Value::Int64(offset as i64), DataType::Int64);
        }
        if let Some(limit) = query.limit {
            sql.push(" LIMIT ");
            sql.param(Value::Int64(limit as i64), DataType::Int64);
        }
        sql.push(")");

        let mut output: Vec<(String, DataType)> = query
            .output_schema
            .fields()
            .iter()
            .map(|field| (field.name.as_str().to_owned(), field.data_type))
            .collect();
        output.push((TREE_CHILDREN.to_owned(), DataType::Int64));
        output.push((TREE_MATCHED.to_owned(), DataType::Bool));

        if !tree.aggregate.is_empty() {
            sql.push(&format!(
                ", below(__og_t, __og_r) AS (SELECT {TREE_ROW}, {TREE_ROW} FROM page \
                 UNION SELECT below.__og_t, c.{TREE_ROW} FROM below JOIN scoped p ON p.{TREE_ROW} = below.__og_r \
                 JOIN scoped c ON c.{TREE_PARENT} = p.{TREE_KEY}), \
                 subtree AS (SELECT below.__og_t AS __og_top"
            ));
            for aggregate in &tree.aggregate {
                sql.push(", ");
                self.aggregate(aggregate, &mut sql)?;
                sql.push(" AS ");
                sql.push(&quote_ident(aggregate.alias.as_str())?);
            }
            sql.push(&format!(
                " FROM below JOIN scoped ON scoped.{TREE_ROW} = below.__og_r \
                 JOIN {base} AS {TREE_BASE} ON {TREE_BASE}.ctid = scoped.{TREE_ROW} \
                 WHERE scoped.{TREE_MATCH} GROUP BY below.__og_t)",
                base = quote_ident(&self.table)?
            ));
            for field in tree.aggregate_schema.fields() {
                output.push((field.name.as_str().to_owned(), field.data_type));
            }
        }

        // Every column read back as text, the page in its order.
        sql.push(" SELECT ");
        for (index, (name, data_type)) in output.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.push(&read_as_text(&quote_ident(name)?, *data_type));
        }
        sql.push(" FROM page");
        if !tree.aggregate.is_empty() {
            sql.push(&format!(
                " LEFT JOIN subtree ON subtree.__og_top = page.{TREE_ROW}"
            ));
        }
        sql.push(&format!(" ORDER BY page.{TREE_POSITION}"));
        Ok((sql.finish(), output))
    }
}

/// A root's condition on the row `alias` (T2): no parent, or one that is no node.
fn tree_root(alias: &str) -> String {
    format!(
        "({alias}.{TREE_PARENT} IS NULL OR NOT EXISTS \
         (SELECT 1 FROM scoped p WHERE p.{TREE_KEY} = {alias}.{TREE_PARENT}))"
    )
}

/// A column read back as text: PostgreSQL's own text form, a timestamp as
/// ISO-8601 in UTC with microseconds (S9) — the form the wire reads.
pub(crate) fn read_as_text(column: &str, data_type: DataType) -> String {
    match data_type {
        DataType::Timestamp => {
            format!("to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
        }
        _ => format!("{column}::text"),
    }
}
