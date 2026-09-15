//! Semantics conformance suite for opengrid
//! (plan/spezifikation/02-query-modell.md §Semantik-Regeln, S1–S14).
//!
//! Every engine and every SQL compiler must return the same result for the same
//! query — that is risk R4 (plan/spezifikation/13-risiken.md) and the reason this
//! suite exists. It pins the rules down as executable cases: a hand-built
//! dataset, one query per rule, and the expected table.
//!
//! The engines that will run these cases arrive in points 07, 08 and 26. Until
//! then the suite runs in **validation mode**: it loads every case, validates its
//! query against the dataset schema and reads the expected values against the
//! query's output types. [`Engine`] is the docking point, and it is deliberately
//! a placeholder — the real traits land in `opengrid-datasource` (point 09).

use opengrid_query::ValidatedQuery;
use opengrid_types::{FieldName, Schema, Value};

mod case;

pub use case::{
    Case, CaseError, Checked, ExpectedTable, check_case, check_dir, load_schema, rules_covered,
};

/// A materialised result: columns plus rows, one [`Value`] per cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub columns: Vec<FieldName>,
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    /// Builds a table from columns and rows.
    pub fn new(columns: Vec<FieldName>, rows: Vec<Vec<Value>>) -> Self {
        Self { columns, rows }
    }
}

/// What an [`Engine`] fails with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineError(pub String);

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EngineError {}

/// A component that executes a validated query.
///
/// Implemented by the local Arrow engine (point 07/08), the PostgreSQL compiler
/// and execution (point 26) and later MySQL/Mongo. Implemented against
/// [`ValidatedQuery`], never against the raw [`opengrid_query::Query`], so that
/// no engine can skip validation.
pub trait Engine {
    /// Runs a validated query against a schema.
    fn run(&self, schema: &Schema, query: &ValidatedQuery) -> Result<Table, EngineError>;
}

/// Whether row order is part of an expectation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowOrder {
    /// Rows must appear in the expected order.
    Ordered,
    /// Only the multiset of rows is compared.
    Unordered,
}

/// Relative tolerance for `Float64` comparison. Rule S12 requires `avg` to be
/// compared with a tolerance instead of exactly.
pub const FLOAT_TOLERANCE: f64 = 1e-9;

/// Compares an expected table against an engine's result.
///
/// Reports the first difference; `Ok(())` means the engine conforms for this
/// case. Column names and the row count must match exactly, cells are compared
/// with [`values_equal`].
pub fn compare(expected: &Table, actual: &Table, order: RowOrder) -> Result<(), String> {
    if expected.columns != actual.columns {
        return Err(format!(
            "columns differ: expected {:?}, got {:?}",
            names(&expected.columns),
            names(&actual.columns)
        ));
    }
    if expected.rows.len() != actual.rows.len() {
        return Err(format!(
            "row count differs: expected {}, got {}",
            expected.rows.len(),
            actual.rows.len()
        ));
    }

    match order {
        RowOrder::Ordered => {
            for (i, (expected_row, actual_row)) in
                expected.rows.iter().zip(&actual.rows).enumerate()
            {
                cells(expected_row, actual_row).map_err(|m| format!("row {i}: {m}"))?;
            }
        }
        RowOrder::Unordered => {
            let mut taken = vec![false; actual.rows.len()];
            for (i, expected_row) in expected.rows.iter().enumerate() {
                let found =
                    actual.rows.iter().enumerate().find(|(j, actual_row)| {
                        !taken[*j] && cells(expected_row, actual_row).is_ok()
                    });
                match found {
                    Some((j, _)) => taken[j] = true,
                    None => {
                        return Err(format!(
                            "row {i} has no match in the result: {expected_row:?}"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Type-aware cell equality.
///
/// `Decimal` is compared exactly (rule S8), `Float64` with [`FLOAT_TOLERANCE`]
/// and with `NaN == NaN` as well as `-0.0 == 0.0` (rule S7). Everything else is
/// compared exactly. Two cells of *different* types are never equal: an engine
/// that returns `Int64` where `Decimal` is declared has a type bug (rule S12).
pub fn values_equal(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int64(a), Value::Int64(b)) => a == b,
        (Value::Float64(a), Value::Float64(b)) => floats_equal(*a, *b),
        (Value::Decimal(a), Value::Decimal(b)) => a.scale() == b.scale() && a.value() == b.value(),
        (Value::Utf8(a), Value::Utf8(b)) => a == b,
        (Value::Date(a), Value::Date(b)) => a.days_since_epoch() == b.days_since_epoch(),
        (Value::Timestamp(a), Value::Timestamp(b)) => a.micros() == b.micros(),
        _ => false,
    }
}

fn floats_equal(a: f64, b: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    if a == b {
        return true;
    }
    (a - b).abs() <= FLOAT_TOLERANCE * a.abs().max(b.abs()).max(1.0)
}

fn cells(expected: &[Value], actual: &[Value]) -> Result<(), String> {
    if expected.len() != actual.len() {
        return Err(format!(
            "cell count differs: expected {}, got {}",
            expected.len(),
            actual.len()
        ));
    }
    for (i, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        if !values_equal(expected, actual) {
            return Err(format!("column {i}: expected {expected:?}, got {actual:?}"));
        }
    }
    Ok(())
}

fn names(columns: &[FieldName]) -> Vec<String> {
    columns.iter().map(|column| column.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Date, Decimal, Timestamp};

    fn table(rows: Vec<Vec<Value>>) -> Table {
        Table::new(
            vec![FieldName::new("v").unwrap()],
            rows.into_iter().map(|row| vec![row[0].clone()]).collect(),
        )
    }

    #[test]
    fn identical_tables_compare_equal() {
        let t = table(vec![vec![Value::Int64(1)], vec![Value::Null]]);
        assert_eq!(compare(&t, &t, RowOrder::Ordered), Ok(()));
    }

    #[test]
    fn ordered_comparison_sees_swapped_rows_as_a_difference() {
        let a = table(vec![vec![Value::Int64(1)], vec![Value::Int64(2)]]);
        let b = table(vec![vec![Value::Int64(2)], vec![Value::Int64(1)]]);
        assert!(compare(&a, &b, RowOrder::Ordered).is_err());
        assert_eq!(compare(&a, &b, RowOrder::Unordered), Ok(()));
    }

    #[test]
    fn nan_equals_nan_and_negative_zero_equals_zero() {
        assert!(values_equal(
            &Value::Float64(f64::NAN),
            &Value::Float64(f64::NAN)
        ));
        assert!(values_equal(&Value::Float64(-0.0), &Value::Float64(0.0)));
        assert!(!values_equal(
            &Value::Float64(f64::NAN),
            &Value::Float64(1.0)
        ));
    }

    #[test]
    fn floats_are_compared_with_a_tolerance() {
        assert!(values_equal(
            &Value::Float64(1.0),
            &Value::Float64(1.0 + 1e-15)
        ));
        assert!(!values_equal(&Value::Float64(1.0), &Value::Float64(1.001)));
    }

    #[test]
    fn decimals_are_compared_exactly() {
        let a = Value::Decimal(Decimal::new(1250, 2));
        assert!(values_equal(&a, &Value::Decimal(Decimal::new(1250, 2))));
        assert!(!values_equal(&a, &Value::Decimal(Decimal::new(1251, 2))));
    }

    #[test]
    fn different_types_are_never_equal() {
        assert!(!values_equal(&Value::Int64(1), &Value::Float64(1.0)));
        assert!(!values_equal(&Value::Null, &Value::Utf8(String::new())));
    }

    #[test]
    fn columns_and_row_counts_must_match() {
        let a = table(vec![vec![Value::Int64(1)]]);
        let b = table(vec![vec![Value::Int64(1)], vec![Value::Int64(2)]]);
        assert!(compare(&a, &b, RowOrder::Ordered).is_err());
        let other_columns = Table::new(vec![FieldName::new("w").unwrap()], vec![]);
        assert!(compare(&a, &other_columns, RowOrder::Ordered).is_err());
    }

    #[test]
    fn date_and_timestamp_compare_on_their_value() {
        let date = Date::from_ymd(2026, 1, 1).unwrap();
        assert!(values_equal(&Value::Date(date), &Value::Date(date)));
        assert!(values_equal(
            &Value::Timestamp(Timestamp::from_micros(1)),
            &Value::Timestamp(Timestamp::from_micros(1))
        ));
        assert!(!values_equal(
            &Value::Timestamp(Timestamp::from_micros(1)),
            &Value::Timestamp(Timestamp::from_micros(2))
        ));
    }
}
