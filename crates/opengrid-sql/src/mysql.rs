//! MySQL 8.0 and later.
//!
//! The connection runs with `ANSI_QUOTES` in its `sql_mode`, so `"name"` is an
//! identifier as in the other dialects, and with the time zone `+00:00`, so a
//! `TIMESTAMP` reads as the instant in UTC (S9). `opengrid-connector-mysql`
//! sets both on every connection it opens.
//!
//! | Rule | MySQL default | What is emitted |
//! |---|---|---|
//! | S3 | NULLs first for `ASC`, last for `DESC`; no `NULLS FIRST`/`LAST` | `(x IS NULL)` as a key of its own before `x` |
//! | S4 | the column's collation, usually accent- and case-insensitive | `COLLATE utf8mb4_0900_bin`: code-point order, no padding |
//! | S4a | — | the locale's `utf8mb4_<locale>_0900_as_cs`, then the bytes as a second key (equal by the collation orders by bytes); a locale MySQL has no collation for is [`CompileError::Unsupported`] |
//! | S5a | `LOWER` maps one character to one | [`CompileError::Unsupported`]: the final sigma and the dotted capital I fold differently from the engine |
//! | S12 | `sum(bigint)` → `decimal`, `avg` → `decimal` | `CAST(… AS SIGNED)`, `CAST(… AS DECIMAL(38,s))`, `CAST(… AS DOUBLE)` |
//!
//! No `GROUPING SETS` (MySQL has only `WITH ROLLUP`), so a pivot takes the
//! generic path. No tree in SQL: a temporary table cannot be read twice in one
//! statement, and a table has no row identity of its own. A DOUBLE column
//! holds no NaN: rule S7 never comes up.

use opengrid_query::{AggregateFn, CmpOp, NullsOrder};
use opengrid_types::{DataType, DatePart, Value};

use crate::{CompileError, Dialect, Sql};

/// MySQL: `?` placeholders with a cast where the type needs one, NULL order as
/// a key of its own, no `GROUPING SETS`, no tree.
#[derive(Clone, Copy, Debug, Default)]
pub struct MySql;

/// Code-point order of UTF-8, with no padding: `'a' <> 'a '` (S4).
const BINARY: &str = "utf8mb4_0900_bin";

/// The languages whose ICU collation is the root's: their sort is MySQL's
/// root collation.
const ROOT_LANGUAGES: &[&str] = &["und", "en", "de", "fr", "it", "nl", "pt"];

/// The languages MySQL has a collation of its own for (`utf8mb4_<l>_0900_as_cs`).
const TAILORED: &[&str] = &[
    "bg", "bs", "cs", "da", "eo", "es", "et", "gl", "hr", "hu", "is", "la", "lt", "lv", "nb", "nn",
    "pl", "ro", "ru", "sk", "sl", "sv", "tr", "vi",
];

/// The MySQL collation that sorts as the ICU collation of `locale` does, if
/// there is one. The region does not change the order of these languages; a
/// script does for Serbian and Mongolian, which MySQL has by script.
fn locale_collation(locale: &str) -> Option<String> {
    let mut parts = locale.split('-');
    let language = parts.next()?.to_ascii_lowercase();
    let script = parts
        .find(|part| part.len() == 4)
        .map(str::to_ascii_lowercase);
    match (language.as_str(), script.as_deref()) {
        ("sr", Some("latn")) => Some("utf8mb4_sr_latn_0900_as_cs".to_owned()),
        ("mn", Some("cyrl")) => Some("utf8mb4_mn_cyrl_0900_as_cs".to_owned()),
        (language, None | Some("latn")) if ROOT_LANGUAGES.contains(&language) => {
            Some("utf8mb4_0900_as_cs".to_owned())
        }
        (language, None) if TAILORED.contains(&language) => {
            Some(format!("utf8mb4_{language}_0900_as_cs"))
        }
        _ => None,
    }
}

/// `x IS NULL` sorts NULLs after the rest ascending, before them descending.
fn null_key(column: &str, nulls: NullsOrder) -> String {
    match nulls {
        NullsOrder::First => format!("({column} IS NULL) DESC"),
        NullsOrder::Last => format!("({column} IS NULL) ASC"),
    }
}

impl Dialect for MySql {
    /// A decimal, a date and a timestamp travel as text and are cast to the
    /// column's type; the connector binds the rest as what they are.
    fn placeholder(&self, _number: usize, data_type: DataType) -> String {
        match data_type {
            DataType::Decimal { precision, scale } => {
                format!("CAST(? AS DECIMAL({precision},{scale}))")
            }
            DataType::Date => "CAST(? AS DATE)".to_owned(),
            DataType::Timestamp => "CAST(? AS DATETIME(6))".to_owned(),
            _ => "?".to_owned(),
        }
    }

    fn collate(&self, column: &str, data_type: DataType) -> String {
        match data_type {
            DataType::Utf8 => format!("{column} COLLATE {BINARY}"),
            _ => column.to_owned(),
        }
    }

    /// Only through [`MySql::order`], which checks the locale first.
    fn locale_sort(&self, column: &str, locale: &str) -> String {
        match locale_collation(locale) {
            Some(collation) => format!("{column} COLLATE {collation}"),
            None => format!("{column} COLLATE {BINARY}"),
        }
    }

    /// Written by [`MySql::order`] as a key of its own.
    fn nulls(&self, _nulls: NullsOrder) -> &'static str {
        ""
    }

    fn order(
        &self,
        column: &str,
        data_type: DataType,
        locale: Option<&str>,
        descending: bool,
        nulls: NullsOrder,
    ) -> Result<String, CompileError> {
        let direction = if descending { " DESC" } else { " ASC" };
        let null_key = null_key(column, nulls);
        match locale {
            None => Ok(format!(
                "{null_key}, {}{direction}",
                self.collate(column, data_type)
            )),
            Some(locale) => {
                let collation = locale_collation(locale).ok_or(CompileError::Unsupported {
                    what: "collation for this locale",
                })?;
                // A collation calls some different strings equal; the engine
                // orders those by their bytes (S4a).
                Ok(format!(
                    "{null_key}, {column} COLLATE {collation}{direction}, \
                     {column} COLLATE {BINARY}{direction}"
                ))
            }
        }
    }

    /// The session's time zone is UTC, so a `TIMESTAMP` is read in UTC (S9).
    fn date_part(&self, part: DatePart, source: &str, _source_type: Option<DataType>) -> String {
        let function = match part {
            DatePart::Year => "YEAR",
            DatePart::Month => "MONTH",
        };
        format!("CAST({function}({source}) AS SIGNED)")
    }

    fn aggregate(&self, function: AggregateFn, column: &str, data_type: DataType) -> String {
        match function {
            AggregateFn::Count => format!("count({column})"),
            // S12: avg is Float64 whatever it averages.
            AggregateFn::Avg => format!("CAST(avg({column}) AS DOUBLE)"),
            AggregateFn::Sum => match data_type {
                // S12: the sum of Int64 stays Int64; MySQL widens to decimal.
                DataType::Int64 => format!("CAST(sum({column}) AS SIGNED)"),
                // S12: the sum of a decimal widens to precision 38, scale kept.
                DataType::Decimal { scale, .. } => {
                    format!("CAST(sum({column}) AS DECIMAL(38,{scale}))")
                }
                _ => format!("sum({column})"),
            },
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
            // `INSTR` under the binary collation compares code points,
            // case-sensitively (S5); the first match at 1 is a prefix.
            CmpOp::Contains | CmpOp::StartsWith => {
                sql.push(&format!("INSTR({column} COLLATE {BINARY}, "));
                sql.param(value, data_type)?;
                sql.push(if op == CmpOp::Contains {
                    ") > 0"
                } else {
                    ") = 1"
                });
                Ok(())
            }
            _ => Err(CompileError::Unsupported {
                what: "case folding as the engine does it",
            }),
        }
    }

    /// A list column is a JSON array (issue #185). `MEMBER OF` a NULL list is
    /// unknown (S1), as the rule wants.
    fn has(
        &self,
        column: &str,
        value: &Value,
        element: DataType,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        sql.param(value, element)?;
        sql.push(&format!(" MEMBER OF ({column})"));
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
        sql.push(&format!("JSON_CONTAINS({column}, JSON_ARRAY("));
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                sql.push(", ");
            }
            sql.param(value, element)?;
        }
        sql.push("))");
        Ok(())
    }

    fn is_empty_list(&self, column: &str) -> String {
        format!("({column} IS NOT NULL AND JSON_LENGTH({column}) = 0)")
    }

    /// MySQL knows `OFFSET` only after a `LIMIT`; the largest one is "no limit".
    fn paging(
        &self,
        limit: Option<u64>,
        offset: Option<u64>,
        sql: &mut Sql<'_>,
    ) -> Result<(), CompileError> {
        if limit.is_none() && offset.is_none() {
            return Ok(());
        }
        sql.push(" LIMIT ");
        match limit {
            Some(limit) => sql.param(&Value::Int64(limit as i64), DataType::Int64)?,
            None => sql.push("18446744073709551615"),
        }
        if let Some(offset) = offset {
            sql.push(" OFFSET ");
            sql.param(&Value::Int64(offset as i64), DataType::Int64)?;
        }
        Ok(())
    }

    fn grouping_sets(&self) -> bool {
        false
    }

    /// A temporary table cannot be read twice in one MySQL statement, and a
    /// tree's statements read it many times: the engine answers trees.
    fn recursive(&self) -> bool {
        false
    }

    // The tree's statements are never written for MySQL (`recursive` is
    // false); these answer what they would read if they were.

    fn row_id(&self) -> &'static str {
        "NULL"
    }

    fn tree_create(&self) -> &'static str {
        "CREATE TEMPORARY TABLE og_tree_scoped AS SELECT"
    }

    fn tree_table(&self) -> &'static str {
        "og_tree_scoped"
    }

    fn tree_indexes(&self) -> &'static str {
        ""
    }

    fn tree_match(&self) -> (&'static str, &'static str, &'static str) {
        ("COALESCE(", ", FALSE)", "TRUE")
    }

    fn is_match(&self, column: &str) -> String {
        column.to_owned()
    }

    fn read_count(&self, expression: &str) -> String {
        format!("CAST(({expression}) AS CHAR)")
    }

    fn read_column(&self, column: &str, data_type: DataType) -> String {
        read_as_text(column, data_type)
    }
}

/// A column as the text the connector reads back: the form the wire format
/// uses, a timestamp as ISO-8601 in UTC with microseconds (S9).
pub fn read_as_text(column: &str, data_type: DataType) -> String {
    match data_type {
        DataType::Timestamp => format!("DATE_FORMAT({column}, '%Y-%m-%dT%H:%i:%s.%fZ')"),
        _ => format!("CAST({column} AS CHAR)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locale_maps_to_mysql_s_collation_or_to_none() {
        assert_eq!(
            locale_collation("und").as_deref(),
            Some("utf8mb4_0900_as_cs")
        );
        assert_eq!(
            locale_collation("de-AT").as_deref(),
            Some("utf8mb4_0900_as_cs")
        );
        assert_eq!(
            locale_collation("sv").as_deref(),
            Some("utf8mb4_sv_0900_as_cs")
        );
        assert_eq!(
            locale_collation("sr-Latn").as_deref(),
            Some("utf8mb4_sr_latn_0900_as_cs")
        );
        // Serbian in Cyrillic, Japanese and an unknown language: no collation.
        assert_eq!(locale_collation("sr"), None);
        assert_eq!(locale_collation("ja"), None);
        assert_eq!(locale_collation("xx"), None);
    }

    #[test]
    fn nulls_are_a_key_of_their_own() {
        let order = MySql
            .order("\"x\"", DataType::Utf8, None, true, NullsOrder::Last)
            .unwrap();
        assert_eq!(
            order,
            "(\"x\" IS NULL) ASC, \"x\" COLLATE utf8mb4_0900_bin DESC"
        );
        let order = MySql
            .order("\"x\"", DataType::Int64, None, false, NullsOrder::First)
            .unwrap();
        assert_eq!(order, "(\"x\" IS NULL) DESC, \"x\" ASC");
    }
}
