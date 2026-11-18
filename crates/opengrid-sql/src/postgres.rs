//! PostgreSQL.
//!
//! The rules S1–S14 are the contract, and PostgreSQL's defaults differ in four
//! places. The dialect makes each one explicit rather than relying on server
//! settings, a collation in the DDL, or a `search_path`:
//!
//! | Rule | PostgreSQL default | What is emitted |
//! |---|---|---|
//! | S3 | `NULLS LAST` for `ASC`, `NULLS FIRST` for `DESC` | `NULLS FIRST`/`NULLS LAST` always written out |
//! | S4 | the column's collation, usually locale-aware | `COLLATE "C"` on every string comparison and sort |
//! | S4a | — | a sort key with a locale is `COLLATE "<locale>-x-icu"`, the ICU collation PostgreSQL predefines for it — deterministic, so strings it calls equal order by their bytes, as in the engine. The database needs ICU, and a locale it has no collation for is its error |
//! | S11 | — | `count(*)` and `count("field")` are different queries |
//! | S12 | `sum(bigint)` → `numeric`, `avg` → `numeric` | `::bigint`, `::numeric(38,s)`, `::double precision` |
//!
//! The shape is the one `plan/conformance-pg-abgleich.py` ran against PostgreSQL
//! 16 for all 48 conformance cases (48/48 exact, approved 2026-09-15).

use opengrid_query::{AggregateFn, CmpOp};
use opengrid_types::{DataType, DatePart, Value};

use crate::{CompileError, Dialect, Sql};

/// PostgreSQL: `$1` placeholders, native `NULLS LAST`, `GROUPING SETS`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Postgres;

/// The PostgreSQL type of an opengrid type — the cast on every parameter.
fn pg_type(data_type: DataType) -> String {
    match data_type {
        DataType::Bool => "boolean".to_owned(),
        DataType::Int64 => "bigint".to_owned(),
        DataType::Float64 => "double precision".to_owned(),
        DataType::Decimal { precision, scale } => format!("numeric({precision},{scale})"),
        DataType::Utf8 => "text".to_owned(),
        DataType::Date => "date".to_owned(),
        // S9: an instant in UTC with microsecond resolution.
        DataType::Timestamp => "timestamptz".to_owned(),
        // An array (issue #185); the elements are never NULL.
        DataType::ListUtf8 => "text[]".to_owned(),
        DataType::ListInt64 => "bigint[]".to_owned(),
    }
}

impl Dialect for Postgres {
    /// The cast chain is what lets every parameter travel as text.
    ///
    /// `$1::numeric` would not do it: the cast makes PostgreSQL *expect* a
    /// numeric parameter, and a text binding is then refused before the query
    /// runs. `$1::text::numeric(12,2)` says what is actually true — text goes in,
    /// a numeric comes out — so one binding path serves every type, no decimal
    /// or date crate is needed, and the snapshot shows a reviewer what each
    /// parameter is meant to become.
    fn placeholder(&self, number: usize, data_type: DataType) -> String {
        let target = pg_type(data_type);
        if target == "text" {
            format!("${number}::text")
        } else {
            format!("${number}::text::{target}")
        }
    }

    /// `COLLATE "C"` for strings, nothing for anything else (S4, E7): the only
    /// way to get the same order out of PostgreSQL that the browser produces
    /// without ICU.
    fn collate(&self, column: &str, data_type: DataType) -> String {
        match data_type {
            DataType::Utf8 => format!("{column} COLLATE \"C\""),
            _ => column.to_owned(),
        }
    }

    fn locale_sort(&self, column: &str, locale: &str) -> String {
        format!("{column} COLLATE \"{locale}-x-icu\"")
    }

    /// `EXTRACT` on a `timestamptz` uses the **session time zone**; rule S9 says
    /// UTC, so the value is pinned to UTC first.
    fn date_part(&self, part: DatePart, source: &str, source_type: Option<DataType>) -> String {
        let instant = match source_type {
            Some(DataType::Timestamp) => format!("{source} AT TIME ZONE 'UTC'"),
            _ => source.to_owned(),
        };
        format!(
            "EXTRACT({} FROM {instant})::bigint",
            match part {
                DatePart::Year => "YEAR",
                DatePart::Month => "MONTH",
            }
        )
    }

    fn aggregate(&self, function: AggregateFn, column: &str, data_type: DataType) -> String {
        match function {
            AggregateFn::Count => format!("count({column})"),
            // S12: avg is Float64 whatever it sums, and PostgreSQL would answer
            // numeric for an integer or decimal input.
            AggregateFn::Avg => format!("avg({column})::double precision"),
            AggregateFn::Sum => match data_type {
                // S12: the sum of Int64 stays Int64; PostgreSQL widens to numeric.
                DataType::Int64 => format!("sum({column})::bigint"),
                // S12: the sum of a decimal widens to precision 38, scale kept.
                DataType::Decimal { scale, .. } => format!("sum({column})::numeric(38,{scale})"),
                _ => format!("sum({column})"),
            },
            // min/max keep the input type (S12) — no cast needed.
            AggregateFn::Min => format!("min({column})"),
            AggregateFn::Max => format!("max({column})"),
        }
    }

    fn true_literal(&self) -> &'static str {
        "TRUE"
    }

    fn false_literal(&self) -> &'static str {
        "FALSE"
    }

    fn one_row(&self) -> &'static str {
        "1::bigint"
    }

    fn text_match(
        &self,
        op: CmpOp,
        column: &str,
        value: &Value,
        data_type: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        match op {
            // Substring and prefix are exact byte comparisons in both engines
            // (S5, case-sensitive), so no collation is involved.
            CmpOp::Contains => {
                sql.push(&format!("strpos({column}, "));
                sql.param(value, data_type)?;
                sql.push(") > 0");
            }
            CmpOp::StartsWith => {
                sql.push(&format!("starts_with({column}, "));
                sql.param(value, data_type)?;
                sql.push(")");
            }
            // Both sides folded by ICU's root lowercase mapping, which is what
            // the engine's `to_lowercase` is (S5a): `lower` under the database's
            // own collation would be libc's, and differ.
            _ => {
                let contains = op == CmpOp::IContains;
                let function = if contains { "strpos" } else { "starts_with" };
                sql.push(&format!(
                    "{function}(lower({column} COLLATE \"und-x-icu\"), lower("
                ));
                sql.param(value, data_type)?;
                sql.push(if contains {
                    " COLLATE \"und-x-icu\")) > 0"
                } else {
                    " COLLATE \"und-x-icu\"))"
                });
            }
        }
        Ok(())
    }

    /// A list column is a PostgreSQL array (issue #185). A NULL array is unknown
    /// for `has` and `has_all` (S1) and plainly not empty.
    fn has(
        &self,
        column: &str,
        value: &Value,
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        sql.param(value, element)?;
        sql.push(&format!(" = ANY({column})"));
        Ok(())
    }

    fn has_all(
        &self,
        column: &str,
        values: &[Value],
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        if values.is_empty() {
            // Nothing is asked for: any list has it, a NULL does not.
            sql.push(&format!("({column} IS NOT NULL)"));
            return Ok(());
        }
        sql.push(&format!("{column} @> ARRAY["));
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.param(value, element)?;
        }
        sql.push(&format!("]::{}[]", pg_type(element)));
        Ok(())
    }

    fn is_empty_list(&self, column: &str) -> String {
        format!("({column} IS NOT NULL AND cardinality({column}) = 0)")
    }

    // `offset` and `limit` are numbers the validator has already bounded, but
    // they are still values: they travel as parameters like everything else.
    fn paging(
        &self,
        limit: Option<u64>,
        offset: Option<u64>,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        if let Some(offset) = offset {
            sql.push(" OFFSET ");
            sql.param(&Value::Int64(offset as i64), DataType::Int64)?;
        }
        if let Some(limit) = limit {
            sql.push(" LIMIT ");
            sql.param(&Value::Int64(limit as i64), DataType::Int64)?;
        }
        Ok(())
    }

    fn grouping_sets(&self) -> bool {
        true
    }

    fn row_id(&self) -> &'static str {
        "ctid"
    }

    fn tree_create(&self) -> &'static str {
        "CREATE TEMP TABLE og_tree_scoped ON COMMIT DROP AS SELECT"
    }

    fn tree_table(&self) -> &'static str {
        "og_tree_scoped"
    }

    /// The indexes on the table, and the statistics the planner needs to use
    /// them: every recursion joins a node to its parent or its children, and
    /// without them each step would hash the whole tree.
    fn tree_indexes(&self) -> &'static str {
        "CREATE INDEX ON og_tree_scoped (__og_key); \
         CREATE INDEX ON og_tree_scoped (__og_parent); \
         CREATE INDEX ON og_tree_scoped (__og_row); ANALYZE og_tree_scoped;"
    }

    fn tree_match(&self) -> (&'static str, &'static str, &'static str) {
        ("COALESCE(", ", FALSE)", "TRUE")
    }

    fn is_match(&self, column: &str) -> String {
        column.to_owned()
    }

    fn read_count(&self, expression: &str) -> String {
        format!("({expression})::text")
    }

    /// PostgreSQL's own text form, a timestamp as ISO-8601 in UTC with
    /// microseconds (S9) — the form the wire reads.
    fn read_column(&self, column: &str, data_type: DataType) -> String {
        match data_type {
            DataType::Timestamp => {
                format!("to_char({column} AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
            }
            _ => format!("{column}::text"),
        }
    }
}
