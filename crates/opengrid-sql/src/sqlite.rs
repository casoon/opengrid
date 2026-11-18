//! SQLite.
//!
//! SQLite has five storage classes and no decimal, date or NaN, so the dialect
//! reads a table laid out the way `opengrid-connector-sqlite` documents:
//! decimals and dates as integers, a list as its JSON array, NaN as the text
//! `'NaN'`.
//!
//! - **S7, NaN**: text sorts after every number in SQLite, so `'NaN'` does what
//!   the rule says. `sum` and `avg` of a float column answer NaN when a NaN is
//!   in the set; SQLite alone would add the text as 0.
//! - **S4/S13, strings**: SQLite's default `BINARY` collation is a byte
//!   comparison of UTF-8 — the order the engine uses. `contains` and
//!   `starts_with` use `instr`, which is case-sensitive; `LIKE` would not be.
//!   `icontains` and `istarts_with` (S5a) fold both sides with `og_fold`, a
//!   function the connector registers on its connection.
//! - **S4a, a locale's collation**: `COLLATE "og_<locale>"`, registered by the
//!   connector the first time a statement names it.
//! - **S8, decimals**: integers throughout; a sum stays exact, an average is
//!   divided as a float (S12).

use opengrid_query::{AggregateFn, CmpOp};
use opengrid_types::{DataType, DatePart, Value};

use crate::{CompileError, Dialect, Sql};

/// SQLite: `?` placeholders, scaled integers, no `GROUPING SETS`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sqlite;

/// The SQL function that folds case as the engine does (rule S5a).
pub const FOLD: &str = "og_fold";
/// The prefix of the collations the connector registers (S4a): `og_de`.
pub const COLLATION: &str = "og_";

impl Dialect for Sqlite {
    fn placeholder(&self, _number: usize, _data_type: DataType) -> String {
        "?".to_owned()
    }

    /// A decimal is the integer coefficient at the column's scale; one that
    /// does not fit SQLite's 64-bit integer is an error, not a wrapped number
    /// (issue #222).
    fn param_value(&self, value: &Value, data_type: DataType) -> Result<Value, CompileError> {
        let Value::Decimal(decimal) = value else {
            return Ok(value.clone());
        };
        let scale = match data_type {
            DataType::Decimal { scale, .. } => scale,
            _ => decimal.scale(),
        };
        let shift = u32::from(scale.saturating_sub(decimal.scale()));
        let scaled = 10i128
            .checked_pow(shift)
            .and_then(|factor| decimal.value().checked_mul(factor))
            .and_then(|scaled| i64::try_from(scaled).ok())
            .ok_or_else(|| CompileError::Value {
                message: format!("a decimal of scale {scale} does not fit SQLite's 64-bit integer"),
            })?;
        Ok(Value::Decimal(opengrid_types::Decimal::new(
            i128::from(scaled),
            scale,
        )))
    }

    fn collate(&self, column: &str, _data_type: DataType) -> String {
        column.to_owned()
    }

    fn locale_sort(&self, column: &str, locale: &str) -> String {
        format!("{column} COLLATE \"{COLLATION}{locale}\"")
    }

    fn date_part(&self, part: DatePart, source: &str, source_type: Option<DataType>) -> String {
        let seconds = match source_type {
            // Microseconds to whole seconds, rounded down also before 1970.
            Some(DataType::Timestamp) => {
                format!("({source} / 1000000 - ({source} % 1000000 < 0))")
            }
            _ => format!("({source} * 86400)"),
        };
        let part = match part {
            DatePart::Year => "%Y",
            DatePart::Month => "%m",
        };
        format!("CAST(strftime('{part}', {seconds}, 'unixepoch') AS INTEGER)")
    }

    fn aggregate(&self, function: AggregateFn, column: &str, data_type: DataType) -> String {
        // A float set with a NaN in it sums and averages to NaN (S7).
        let nan_guard = |expression: String| {
            format!(
                "CASE WHEN count(CASE WHEN typeof({column}) = 'text' THEN 1 END) > 0 \
                 THEN 'NaN' ELSE {expression} END"
            )
        };
        match (function, data_type) {
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
        }
    }

    fn true_literal(&self) -> &'static str {
        "1"
    }

    fn false_literal(&self) -> &'static str {
        "0"
    }

    fn one_row(&self) -> &'static str {
        "1"
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
            // `instr` compares bytes, case-sensitively (S5).
            CmpOp::Contains | CmpOp::StartsWith => {
                sql.push(&format!("instr({column}, "));
                sql.param(value, data_type)?;
                sql.push(if op == CmpOp::Contains {
                    ") > 0"
                } else {
                    ") = 1"
                });
            }
            // Both sides folded by the engine's function (S5a).
            _ => {
                sql.push(&format!("instr({FOLD}({column}), {FOLD}("));
                sql.param(value, data_type)?;
                sql.push(if op == CmpOp::IContains {
                    ")) > 0"
                } else {
                    ")) = 1"
                });
            }
        }
        Ok(())
    }

    /// A list column holds its JSON array as text (issue #185): `json_each`
    /// reads the elements back, typed. `EXISTS` over a NULL list is false, not
    /// unknown (S1): the CASE keeps the NULL list out of `NOT has` too.
    fn has(
        &self,
        column: &str,
        value: &Value,
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        sql.push(&format!("(CASE WHEN {column} IS NULL THEN NULL ELSE "));
        exists(column, value, element, sql)?;
        sql.push(" END)");
        Ok(())
    }

    /// Every value, and a NULL list is unknown: no values still needs a list.
    fn has_all(
        &self,
        column: &str,
        values: &[Value],
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        sql.push(&format!("(CASE WHEN {column} IS NULL THEN NULL ELSE (1"));
        for value in values {
            sql.push(" AND ");
            exists(column, value, element, sql)?;
        }
        sql.push(") END)");
        Ok(())
    }

    fn is_empty_list(&self, column: &str) -> String {
        format!("({column} IS NOT NULL AND json_array_length({column}) = 0)")
    }

    /// SQLite knows `OFFSET` only after a `LIMIT`; -1 is "no limit".
    fn paging(
        &self,
        limit: Option<u64>,
        offset: Option<u64>,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        if limit.is_some() || offset.is_some() {
            sql.push(" LIMIT ");
            sql.param(
                &Value::Int64(limit.map_or(-1, |limit| limit as i64)),
                DataType::Int64,
            )?;
            if let Some(offset) = offset {
                sql.push(" OFFSET ");
                sql.param(&Value::Int64(offset as i64), DataType::Int64)?;
            }
        }
        Ok(())
    }

    fn grouping_sets(&self) -> bool {
        false
    }

    fn row_id(&self) -> &'static str {
        "rowid"
    }

    fn tree_create(&self) -> &'static str {
        "CREATE TEMP TABLE og_tree_scoped AS SELECT"
    }

    fn tree_table(&self) -> &'static str {
        "temp.og_tree_scoped"
    }

    fn tree_indexes(&self) -> &'static str {
        "CREATE INDEX temp.og_tree_scoped_key ON og_tree_scoped(__og_key); \
         CREATE INDEX temp.og_tree_scoped_parent ON og_tree_scoped(__og_parent); \
         CREATE UNIQUE INDEX temp.og_tree_scoped_row ON og_tree_scoped(__og_row);"
    }

    fn tree_match(&self) -> (&'static str, &'static str, &'static str) {
        ("CASE WHEN ", " THEN 1 ELSE 0 END", "1")
    }

    fn is_match(&self, column: &str) -> String {
        format!("{column} = 1")
    }

    fn read_count(&self, expression: &str) -> String {
        format!("({expression})")
    }

    fn read_column(&self, column: &str, _data_type: DataType) -> String {
        column.to_owned()
    }
}

/// `column` holds `value` as an element.
fn exists(
    column: &str,
    value: &Value,
    element: DataType,
    sql: &mut Sql<'_>,
) -> Result<(), CompileError> {
    sql.push(&format!(
        "EXISTS (SELECT 1 FROM json_each({column}) WHERE value = "
    ));
    sql.param(value, element)?;
    sql.push(")");
    Ok(())
}
