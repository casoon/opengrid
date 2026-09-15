//! Filter evaluation: the tree of the query model into a row mask.
//!
//! Every comparison keeps the three-valued logic of rule S1: a NULL on either
//! side makes the comparison *unknown*, and an unknown row is dropped. The mask
//! carries that as `null` — the logical operators combine Kleene-style instead
//! of bitwise, so `not(unknown)` stays unknown and `unknown and false` becomes
//! `false` rather than `unknown`.

use std::cmp::Ordering;

use arrow_array::builder::BooleanBuilder;
use arrow_array::cast::AsArray;
use arrow_array::types::Float64Type;
use arrow_array::{Array, ArrayRef, BooleanArray, RecordBatch, Scalar, StringArray};
use arrow_ord::cmp;
use opengrid_query::{CmpOp, ValidatedFilter};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};

use super::ExecuteError;
use crate::ingest::batch;

/// Evaluates a filter into the mask of the rows that survive it.
pub(crate) fn evaluate(
    filter: &ValidatedFilter,
    batch: &RecordBatch,
) -> Result<BooleanArray, ExecuteError> {
    let rows = batch.num_rows();
    Ok(match filter {
        ValidatedFilter::And(items) => {
            let mut mask = BooleanArray::from(vec![true; rows]);
            for item in items {
                mask = and_kleene(&mask, &evaluate(item, batch)?);
            }
            mask
        }
        ValidatedFilter::Or(items) => {
            let mut mask = BooleanArray::from(vec![false; rows]);
            for item in items {
                mask = or_kleene(&mask, &evaluate(item, batch)?);
            }
            mask
        }
        ValidatedFilter::Not(inner) => not_kleene(&evaluate(inner, batch)?),
        ValidatedFilter::Cmp {
            field,
            data_type,
            op,
            value,
        } => compare(&column(batch, field)?, field, *data_type, *op, value)?,
        ValidatedFilter::InList {
            field,
            data_type,
            values,
        } => {
            // Rule S2: `in` is the OR of equality against every list member, so
            // an empty list matches nothing — and never errors.
            let column = column(batch, field)?;
            let mut mask = BooleanArray::from(vec![false; rows]);
            for value in values {
                let hit = compare(&column, field, *data_type, CmpOp::Eq, value)?;
                mask = or_kleene(&mask, &hit);
            }
            mask
        }
        ValidatedFilter::IsNull { field } => null_mask(&column(batch, field)?),
        ValidatedFilter::IsNotNull { field } => not_kleene(&null_mask(&column(batch, field)?)),
    })
}

/// The rows that hold a NULL — rule S1 reaches NULL only through a null check.
fn null_mask(column: &ArrayRef) -> BooleanArray {
    BooleanArray::from_iter((0..column.len()).map(|row| column.is_null(row)))
}

fn column(batch: &RecordBatch, field: &FieldName) -> Result<ArrayRef, ExecuteError> {
    let index =
        batch
            .schema()
            .index_of(field.as_str())
            .map_err(|_| ExecuteError::MissingField {
                field: field.to_string(),
            })?;
    Ok(batch.column(index).clone())
}

/// Compares one column against one literal.
fn compare(
    column: &ArrayRef,
    field: &FieldName,
    data_type: DataType,
    op: CmpOp,
    value: &Value,
) -> Result<BooleanArray, ExecuteError> {
    if op.is_string_op() {
        return string_op(column, field, op, value);
    }
    if data_type == DataType::Float64 {
        return float_compare(column, field, op, value);
    }

    let literal = Scalar::new(literal(column, field, data_type, value)?);
    Ok(match op {
        CmpOp::Eq => cmp::eq(column, &literal)?,
        CmpOp::Ne => cmp::neq(column, &literal)?,
        CmpOp::Lt => cmp::lt(column, &literal)?,
        CmpOp::Lte => cmp::lt_eq(column, &literal)?,
        CmpOp::Gt => cmp::gt(column, &literal)?,
        CmpOp::Gte => cmp::gt_eq(column, &literal)?,
        CmpOp::In | CmpOp::Contains | CmpOp::StartsWith => {
            unreachable!("`in` is an InList and the string operators are handled above")
        }
    })
}

/// `contains` and `starts_with` (rule S5: case-sensitive).
///
/// Plain substring tests on the UTF-8 bytes: rule S13 asks for a binary
/// comparison without Unicode normalisation, which is exactly what Rust's
/// `str::contains` does. A NULL cell stays unknown — `Option::map` keeps it.
fn string_op(
    column: &ArrayRef,
    field: &FieldName,
    op: CmpOp,
    value: &Value,
) -> Result<BooleanArray, ExecuteError> {
    let Value::Utf8(needle) = value else {
        return Err(wrong_literal(field, "utf8", value));
    };
    let Some(strings) = column.as_any().downcast_ref::<StringArray>() else {
        return Err(ExecuteError::TypeMismatch {
            field: field.to_string(),
            expected: DataType::Utf8,
            found: column.data_type().clone(),
        });
    };
    Ok(BooleanArray::from_iter(strings.iter().map(|cell| {
        cell.map(|cell| needle_check(cell, needle, op))
    })))
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
/// This is a per-row loop on purpose. The Arrow comparison kernels compare by
/// IEEE, where `NaN != NaN` and `NaN < any number` is not even defined — using
/// them would turn rule S7 into rule S1. A NULL cell stays unknown.
fn float_compare(
    column: &ArrayRef,
    field: &FieldName,
    op: CmpOp,
    value: &Value,
) -> Result<BooleanArray, ExecuteError> {
    let Value::Float64(literal) = value else {
        return Err(wrong_literal(field, "float64", value));
    };
    let values = column.as_primitive::<Float64Type>();
    let mut mask = BooleanBuilder::with_capacity(values.len());
    for cell in values.iter() {
        match cell {
            None => mask.append_null(),
            Some(cell) => mask.append_value(float_holds(cell, *literal, op)),
        }
    }
    Ok(mask.finish())
}

fn float_holds(cell: f64, literal: f64, op: CmpOp) -> bool {
    let order = float_order(cell, literal);
    match op {
        CmpOp::Eq => order == Ordering::Equal,
        CmpOp::Ne => order != Ordering::Equal,
        CmpOp::Lt => order == Ordering::Less,
        CmpOp::Lte => order != Ordering::Greater,
        CmpOp::Gt => order == Ordering::Greater,
        CmpOp::Gte => order != Ordering::Less,
        other => unreachable!("`in` is an InList, the rest is a comparison, got {other:?}"),
    }
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

/// The literal as a one-element Arrow array.
///
/// It goes through the **same coercion path as ingest** ([`crate::ingest::batch`]),
/// so the value a query compares against is built exactly like the cells of the
/// file it is compared against — one path, not two.
fn literal(
    column: &ArrayRef,
    field: &FieldName,
    data_type: DataType,
    value: &Value,
) -> Result<ArrayRef, ExecuteError> {
    let expected = arrow_schema::DataType::from(data_type);
    if column.data_type() != &expected {
        return Err(ExecuteError::TypeMismatch {
            field: field.to_string(),
            expected: data_type,
            found: column.data_type().clone(),
        });
    }
    let schema = Schema::new(vec![Field::new(field.clone(), data_type)]);
    let arrow = arrow_schema::Schema::from(&schema);
    let batch = batch::build(&schema, &arrow, &[vec![value.clone()]]).map_err(|message| {
        ExecuteError::Value {
            field: field.to_string(),
            message,
        }
    })?;
    Ok(batch.column(0).clone())
}

fn wrong_literal(field: &FieldName, expected: &str, value: &Value) -> ExecuteError {
    ExecuteError::Value {
        field: field.to_string(),
        message: format!("expected a {expected} literal, found {value:?}"),
    }
}

/// Three-valued AND: `false` wins, a NULL next to `true` stays unknown.
fn and_kleene(left: &BooleanArray, right: &BooleanArray) -> BooleanArray {
    debug_assert_eq!(left.len(), right.len(), "masks cover the same rows");
    BooleanArray::from_iter(left.iter().zip(right.iter()).map(|(left, right)| {
        match (left, right) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }
    }))
}

/// Three-valued OR: `true` wins, a NULL next to `false` stays unknown.
fn or_kleene(left: &BooleanArray, right: &BooleanArray) -> BooleanArray {
    debug_assert_eq!(left.len(), right.len(), "masks cover the same rows");
    BooleanArray::from_iter(left.iter().zip(right.iter()).map(|(left, right)| {
        match (left, right) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }
    }))
}

/// Three-valued NOT: unknown stays unknown.
fn not_kleene(mask: &BooleanArray) -> BooleanArray {
    BooleanArray::from_iter(mask.iter().map(|value| value.map(|value| !value)))
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
        let left = BooleanArray::from_iter(pairs.iter().map(|(left, _)| *left));
        let right = BooleanArray::from_iter(pairs.iter().map(|(_, right)| *right));

        let and: Vec<Option<bool>> = and_kleene(&left, &right).iter().collect();
        let or: Vec<Option<bool>> = or_kleene(&left, &right).iter().collect();
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

        let not: Vec<Option<bool>> = not_kleene(&left).iter().collect();
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
