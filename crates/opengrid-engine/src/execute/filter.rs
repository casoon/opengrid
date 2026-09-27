//! Filter evaluation: the tree of the query model into a row mask.
//!
//! Every comparison keeps the three-valued logic of rule S1: a NULL on either
//! side makes the comparison *unknown*, and an unknown row is dropped. The mask
//! carries that as `null` — the logical operators combine Kleene-style instead
//! of bitwise, so `not(unknown)` stays unknown and `unknown and false` becomes
//! `false` rather than `unknown`.

use std::cmp::Ordering;

use opengrid_columns::{Column, Table, Values};
use opengrid_query::{CmpOp, ValidatedFilter};
use opengrid_types::{DataType, FieldName, Value};

use super::ExecuteError;

/// A three-valued row mask: `Some(true)` keeps the row, `Some(false)` drops it,
/// `None` is *unknown* (rule S1) — and an unknown row is dropped too.
pub(crate) type Mask = Vec<Option<bool>>;

/// Evaluates a filter into the mask of the rows that survive it.
pub(crate) fn evaluate(filter: &ValidatedFilter, table: &Table) -> Result<Mask, ExecuteError> {
    let rows = table.num_rows();
    Ok(match filter {
        ValidatedFilter::And(items) => {
            let mut mask = vec![Some(true); rows];
            for item in items {
                mask = and_kleene(&mask, &evaluate(item, table)?);
            }
            mask
        }
        ValidatedFilter::Or(items) => {
            let mut mask = vec![Some(false); rows];
            for item in items {
                mask = or_kleene(&mask, &evaluate(item, table)?);
            }
            mask
        }
        ValidatedFilter::Not(inner) => not_kleene(&evaluate(inner, table)?),
        ValidatedFilter::Cmp {
            field,
            data_type,
            op,
            value,
        } => compare(column(table, field)?, field, *data_type, *op, value)?,
        ValidatedFilter::InList {
            field,
            data_type,
            values,
        } => {
            // Rule S2: `in` is the OR of equality against every list member, so
            // an empty list matches nothing — and never errors.
            let column = column(table, field)?;
            let mut mask = vec![Some(false); rows];
            for value in values {
                let hit = compare(column, field, *data_type, CmpOp::Eq, value)?;
                mask = or_kleene(&mask, &hit);
            }
            mask
        }
        ValidatedFilter::IsNull { field } => null_mask(column(table, field)?),
        ValidatedFilter::IsNotNull { field } => not_kleene(&null_mask(column(table, field)?)),
    })
}

/// The rows that hold a NULL — rule S1 reaches NULL only through a null check.
fn null_mask(column: &Column) -> Mask {
    (0..column.len())
        .map(|row| Some(column.is_null(row)))
        .collect()
}

fn column<'a>(table: &'a Table, field: &FieldName) -> Result<&'a Column, ExecuteError> {
    super::column(table, field.as_str())
}

/// Compares one column against one literal.
///
/// The literal has to be of the column's type — validation made it so, and a
/// value that is not is reported with the same words ingest uses for a cell
/// that does not fit its column. A NULL literal compares as unknown (S1).
fn compare(
    column: &Column,
    field: &FieldName,
    data_type: DataType,
    op: CmpOp,
    value: &Value,
) -> Result<Mask, ExecuteError> {
    if op.is_string_op() {
        return string_op(column, field, op, value);
    }
    if data_type == DataType::Float64 {
        return float_compare(column, field, op, value);
    }
    if column.data_type() != data_type {
        return Err(ExecuteError::TypeMismatch {
            field: field.to_string(),
            expected: data_type,
            found: column.data_type(),
        });
    }
    if *value == Value::Null {
        return Ok(vec![None; column.len()]);
    }
    Ok(match (column.values(), value) {
        (Values::Bool(values), Value::Bool(literal)) => rows(column, |at| values[at], *literal, op),
        (Values::Int64(values), Value::Int64(literal)) => {
            rows(column, |at| values[at], *literal, op)
        }
        (Values::Decimal(values), Value::Decimal(literal)) => {
            rows(column, |at| values[at], literal.value(), op)
        }
        (Values::Utf8 { .. }, Value::Utf8(literal)) => rows(
            column,
            |at| column.str(at).as_bytes(),
            literal.as_bytes(),
            op,
        ),
        (Values::Date(values), Value::Date(literal)) => {
            rows(column, |at| values[at], literal.days_since_epoch(), op)
        }
        (Values::Timestamp(values), Value::Timestamp(literal)) => {
            rows(column, |at| values[at], literal.micros(), op)
        }
        (_, other) => {
            return Err(ExecuteError::Value {
                field: field.to_string(),
                message: format!("column {field} expects {data_type}, found {other:?}"),
            });
        }
    })
}

/// One comparison per row; a NULL cell stays unknown.
fn rows<K: Ord>(column: &Column, cell: impl Fn(usize) -> K, literal: K, op: CmpOp) -> Mask {
    (0..column.len())
        .map(|row| (!column.is_null(row)).then(|| holds(cell(row).cmp(&literal), op)))
        .collect()
}

/// Whether an ordering satisfies a comparison operator.
fn holds(order: Ordering, op: CmpOp) -> bool {
    match op {
        CmpOp::Eq => order == Ordering::Equal,
        CmpOp::Ne => order != Ordering::Equal,
        CmpOp::Lt => order == Ordering::Less,
        CmpOp::Lte => order != Ordering::Greater,
        CmpOp::Gt => order == Ordering::Greater,
        CmpOp::Gte => order != Ordering::Less,
        CmpOp::In | CmpOp::Contains | CmpOp::StartsWith => {
            unreachable!("`in` is an InList and the string operators are handled apart")
        }
    }
}

/// `contains` and `starts_with` (rule S5: case-sensitive).
///
/// Plain substring tests on the UTF-8 bytes: rule S13 asks for a binary
/// comparison without Unicode normalisation, which is exactly what Rust's
/// `str::contains` does. A NULL cell stays unknown.
fn string_op(
    column: &Column,
    field: &FieldName,
    op: CmpOp,
    value: &Value,
) -> Result<Mask, ExecuteError> {
    let Value::Utf8(needle) = value else {
        return Err(wrong_literal(field, "utf8", value));
    };
    if column.data_type() != DataType::Utf8 {
        return Err(ExecuteError::TypeMismatch {
            field: field.to_string(),
            expected: DataType::Utf8,
            found: column.data_type(),
        });
    }
    Ok((0..column.len())
        .map(|row| (!column.is_null(row)).then(|| needle_check(column.str(row), needle, op)))
        .collect())
}

fn needle_check(cell: &str, needle: &str, op: CmpOp) -> bool {
    match op {
        CmpOp::Contains => cell.contains(needle),
        CmpOp::StartsWith => cell.starts_with(needle),
        other => unreachable!("only the string operators reach here, got {other:?}"),
    }
}

/// Float comparison with **PostgreSQL's order** (rule S7): `NaN` equals `NaN`
/// and is greater than every number, `-0.0` equals `0.0`.
///
/// Not the sort's total order, where `-0.0 < 0.0`: IEEE comparison would make
/// `NaN != NaN`, and the total order would split zero — either would turn rule
/// S7 into something else. A NULL cell stays unknown.
fn float_compare(
    column: &Column,
    field: &FieldName,
    op: CmpOp,
    value: &Value,
) -> Result<Mask, ExecuteError> {
    let Value::Float64(literal) = value else {
        return Err(wrong_literal(field, "float64", value));
    };
    let Values::Float64(values) = column.values() else {
        return Err(ExecuteError::TypeMismatch {
            field: field.to_string(),
            expected: DataType::Float64,
            found: column.data_type(),
        });
    };
    Ok(values
        .iter()
        .enumerate()
        .map(|(row, cell)| (!column.is_null(row)).then(|| float_holds(*cell, *literal, op)))
        .collect())
}

fn float_holds(cell: f64, literal: f64, op: CmpOp) -> bool {
    holds(float_order(cell, literal), op)
}

/// Total order of two floats the way PostgreSQL compares them (rule S7).
fn float_order(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).expect("neither operand is NaN"),
    }
}

fn wrong_literal(field: &FieldName, expected: &str, value: &Value) -> ExecuteError {
    ExecuteError::Value {
        field: field.to_string(),
        message: format!("expected a {expected} literal, found {value:?}"),
    }
}

/// Three-valued AND: `false` wins, a NULL next to `true` stays unknown.
fn and_kleene(left: &[Option<bool>], right: &[Option<bool>]) -> Mask {
    debug_assert_eq!(left.len(), right.len(), "masks cover the same rows");
    left.iter()
        .zip(right)
        .map(|pair| match pair {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        })
        .collect()
}

/// Three-valued OR: `true` wins, a NULL next to `false` stays unknown.
fn or_kleene(left: &[Option<bool>], right: &[Option<bool>]) -> Mask {
    debug_assert_eq!(left.len(), right.len(), "masks cover the same rows");
    left.iter()
        .zip(right)
        .map(|pair| match pair {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        })
        .collect()
}

/// Three-valued NOT: unknown stays unknown.
fn not_kleene(mask: &[Option<bool>]) -> Mask {
    mask.iter().map(|value| value.map(|value| !value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rule S7: NaN equals NaN, is greater than every number and less than none.
    #[test]
    fn nan_equals_nan_and_beats_every_number() {
        assert_eq!(float_order(f64::NAN, f64::NAN), Ordering::Equal);
        assert_eq!(float_order(f64::NAN, 1.0), Ordering::Greater);
        assert_eq!(float_order(f64::NAN, f64::INFINITY), Ordering::Greater);
        assert_eq!(float_order(1.0, f64::NAN), Ordering::Less);
        assert_eq!(float_order(f64::NEG_INFINITY, -1.0), Ordering::Less);

        assert!(float_holds(f64::NAN, f64::NAN, CmpOp::Eq));
        assert!(
            !float_holds(f64::NAN, f64::NAN, CmpOp::Gt),
            "NaN is not above itself"
        );
        assert!(float_holds(f64::NAN, 1.0, CmpOp::Gt));
        assert!(float_holds(f64::NAN, 1.0, CmpOp::Gte));
        assert!(!float_holds(1.0, f64::NAN, CmpOp::Gte));
        assert!(float_holds(1.0, f64::NAN, CmpOp::Lt));
    }

    /// Rule S7: `-0.0` and `0.0` are the same number for every comparison.
    #[test]
    fn negative_zero_equals_zero() {
        assert_eq!(float_order(-0.0, 0.0), Ordering::Equal);
        assert!(float_holds(-0.0, 0.0, CmpOp::Eq));
        assert!(float_holds(-0.0, 0.0, CmpOp::Lte));
        assert!(float_holds(-0.0, 0.0, CmpOp::Gte));
        assert!(!float_holds(-0.0, 0.0, CmpOp::Ne));
    }

    /// Rule S1: unknown stays unknown, except where `false` (AND) or `true` (OR)
    /// already decides the outcome.
    #[test]
    fn kleene_logic_keeps_unknown_unknown() {
        let pairs = [
            (Some(true), Some(true)),
            (Some(true), Some(false)),
            (Some(true), None),
            (Some(false), Some(true)),
            (Some(false), Some(false)),
            (Some(false), None),
            (None, Some(true)),
            (None, Some(false)),
            (None, None),
        ];
        let left: Mask = pairs.iter().map(|(left, _)| *left).collect();
        let right: Mask = pairs.iter().map(|(_, right)| *right).collect();

        let and = and_kleene(&left, &right);
        let or = or_kleene(&left, &right);
        assert_eq!(
            and,
            vec![
                Some(true),
                Some(false),
                None,
                Some(false),
                Some(false),
                Some(false),
                None,
                Some(false),
                None
            ]
        );
        assert_eq!(
            or,
            vec![
                Some(true),
                Some(true),
                Some(true),
                Some(true),
                Some(false),
                None,
                Some(true),
                None,
                None
            ]
        );

        let not = not_kleene(&left);
        assert_eq!(
            not,
            vec![
                Some(false),
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                Some(true),
                None,
                None,
                None
            ]
        );
    }
}
