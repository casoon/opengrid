//! Sorting: sort keys into a row order.
//!
//! The order [`order`] produces:
//!
//! - **NULLs** go first or last per key, independent of the direction
//!   (rule S3: `nulls` is always explicit and not flipped by `desc`).
//! - **Strings** compare byte-wise, which for UTF-8 is codepoint order — the
//!   `binary` collation of rule S4, so `"Z" < "z" < "ä"`.
//! - **Floats** follow the IEEE 754 total order ([`f64::total_cmp`]): `NaN`
//!   after every number, `-0.0` before `0.0` (rule S7 for sorting; the
//!   *comparison* of S7, where `-0.0 = 0.0`, is the engine's business).
//! - **Booleans:** `false` before `true`.
//! - **Ties keep their input order.** S6 leaves ties undefined, but the grid
//!   pages through a sort that is nothing but ties, and every window has to
//!   agree with every other. The input position is therefore the last key: the
//!   order is total, the same for every `end`, and the sort may stop once it
//!   has the rows a page needs.
//!
//! Two paths to that one order:
//!
//! - **One key:** the typed values paired with their position, sorted directly.
//! - **Several keys:** every row's keys encoded once into bytes that compare
//!   with `memcmp` exactly as the values would — one byte comparison per pair
//!   instead of a walk through typed columns. The position is the final key.

use std::cmp::Ordering;

use opengrid_types::DataType;

use crate::{Column, Values};

/// One sort key: a column and how to order it.
#[derive(Clone, Copy, Debug)]
pub struct SortKey<'a> {
    pub column: &'a Column,
    pub descending: bool,
    pub nulls_first: bool,
}

/// The first `end` input positions in sort order (all of them when `end` is
/// larger than the column).
///
/// Positions are four bytes, so a table of more than `u32::MAX` rows is refused
/// rather than wrapped — a wasm32 address space holds far fewer rows anyway.
///
/// # Panics
/// When `keys` is empty or the key columns differ in length; the executor
/// resolves the keys against one table.
pub fn order(keys: &[SortKey<'_>], end: usize) -> Result<Vec<u32>, String> {
    let rows = keys.first().expect("at least one sort key").column.len();
    assert!(
        keys.iter().all(|key| key.column.len() == rows),
        "sort keys come from one table"
    );
    u32::try_from(rows).map_err(|_| format!("{rows} rows are too many to sort"))?;
    let end = end.min(rows);
    Ok(match keys {
        [key] => one_key(key, end),
        _ => several_keys(keys, rows, end),
    })
}

fn one_key(key: &SortKey<'_>, end: usize) -> Vec<u32> {
    let column = key.column;
    let mut nulls = Vec::with_capacity(column.null_count());
    let mut present = Vec::with_capacity(column.len() - column.null_count());
    for row in 0..column.len() {
        if column.is_null(row) {
            nulls.push(row as u32);
        } else {
            present.push(row as u32);
        }
    }

    let (null_rows, value_rows) = if key.nulls_first {
        let null_rows = nulls.len().min(end);
        (null_rows, end - null_rows)
    } else {
        let value_rows = present.len().min(end);
        (end - value_rows, value_rows)
    };
    let desc = key.descending;
    // Every fixed-width type orders as an `i64` key (floats through the total
    // order, decimals up to 18 digits exactly), so one sorter serves them all;
    // only wide decimals and strings need their own.
    let sorted = match column.values() {
        Values::Bool(values) => by(
            pairs(&present, |at| i64::from(values[at])),
            desc,
            value_rows,
        ),
        Values::Int64(values) | Values::Timestamp(values) => {
            by(pairs(&present, |at| values[at]), desc, value_rows)
        }
        Values::Float64(values) => by(
            pairs(&present, |at| total_key(values[at])),
            desc,
            value_rows,
        ),
        Values::Date(values) => by(
            pairs(&present, |at| i64::from(values[at])),
            desc,
            value_rows,
        ),
        // A decimal of up to 18 digits fits an `i64`, and half the bytes move.
        Values::Decimal(values) if decimal_fits_i64(column) => {
            by(pairs(&present, |at| values[at] as i64), desc, value_rows)
        }
        Values::Decimal(values) => by(pairs(&present, |at| values[at]), desc, value_rows),
        Values::Utf8 { .. } => by(
            pairs(&present, |at| column.str(at).as_bytes()),
            desc,
            value_rows,
        ),
    };

    nulls.truncate(null_rows);
    if key.nulls_first {
        nulls.extend(sorted);
        nulls
    } else {
        let mut out = sorted;
        out.extend(nulls);
        out
    }
}

/// Each row with its key.
fn pairs<K>(rows: &[u32], key: impl Fn(usize) -> K) -> Vec<(K, u32)> {
    rows.iter().map(|at| (key(*at as usize), *at)).collect()
}

/// The first `need` rows ordered by their key, ties by position.
fn by<K: Ord>(mut pairs: Vec<(K, u32)>, desc: bool, need: usize) -> Vec<u32> {
    let compare = |a: &(K, u32), b: &(K, u32)| {
        let order = a.0.cmp(&b.0);
        let order = if desc { order.reverse() } else { order };
        order.then(a.1.cmp(&b.1))
    };
    first(&mut pairs, need, compare);
    pairs.into_iter().map(|(_, at)| at).collect()
}

/// Whether every coefficient of a decimal column fits an `i64` — true by type
/// up to 18 digits of precision.
fn decimal_fits_i64(column: &Column) -> bool {
    matches!(column.data_type(), DataType::Decimal { precision, .. } if precision <= 18)
}

/// Sorts the front `need` elements into place and drops the rest. The order is
/// total, so the front of a partial sort is the front of the full one.
///
/// Data that is already in order — the grid's default sort by `id` — is
/// recognised in one pass and only cut.
fn first<T>(items: &mut Vec<T>, need: usize, compare: impl Fn(&T, &T) -> Ordering) {
    if need == 0 {
        items.clear();
        return;
    }
    if items.is_sorted_by(|a, b| compare(a, b) != Ordering::Greater) {
        items.truncate(need);
        return;
    }
    if need < items.len() {
        items.select_nth_unstable_by(need - 1, &compare);
        items.truncate(need);
    }
    items.sort_unstable_by(compare);
}

fn several_keys(keys: &[SortKey<'_>], rows: usize, end: usize) -> Vec<u32> {
    let mut bytes = Vec::new();
    let mut starts = Vec::with_capacity(rows + 1);
    for row in 0..rows {
        starts.push(bytes.len());
        for key in keys {
            encode(key, row, &mut bytes);
        }
    }
    starts.push(bytes.len());

    let mut pairs: Vec<(&[u8], u32)> = (0..rows)
        .map(|row| (&bytes[starts[row]..starts[row + 1]], row as u32))
        .collect();
    first(&mut pairs, end, |a, b| a.cmp(b));
    pairs.into_iter().map(|(_, at)| at).collect()
}

/// Appends the order-preserving bytes of one key cell.
///
/// The NULL marker sits outside the direction: `0x00` sorts a NULL before every
/// value, `0x02` after, and `desc` inverts only the value bytes that follow a
/// value's `0x01`. Every value encoding is prefix-free, so inverting it reverses
/// its order.
fn encode(key: &SortKey<'_>, row: usize, out: &mut Vec<u8>) {
    let column = key.column;
    if column.is_null(row) {
        out.push(if key.nulls_first { 0x00 } else { 0x02 });
        return;
    }
    out.push(0x01);
    let start = out.len();
    match column.values() {
        Values::Bool(values) => out.push(u8::from(values[row])),
        Values::Int64(values) | Values::Timestamp(values) => {
            out.extend_from_slice(&(values[row] ^ i64::MIN).to_be_bytes());
        }
        Values::Float64(values) => {
            out.extend_from_slice(&(total_key(values[row]) ^ i64::MIN).to_be_bytes());
        }
        Values::Decimal(values) => {
            out.extend_from_slice(&(values[row] ^ i128::MIN).to_be_bytes());
        }
        Values::Date(values) => out.extend_from_slice(&(values[row] ^ i32::MIN).to_be_bytes()),
        Values::Utf8 { .. } => {
            // Escaped and terminated, so no string is a prefix of another's
            // encoding and the byte order is kept.
            for byte in column.str(row).as_bytes() {
                match byte {
                    0x00 => out.extend_from_slice(&[0x00, 0xFF]),
                    other => out.push(*other),
                }
            }
            out.extend_from_slice(&[0x00, 0x00]);
        }
    }
    if key.descending {
        for byte in &mut out[start..] {
            *byte = !*byte;
        }
    }
}

/// An integer that orders like [`f64::total_cmp`] — the same bit trick.
fn total_key(value: f64) -> i64 {
    let bits = value.to_bits() as i64;
    bits ^ ((((bits >> 63) as u64) >> 1) as i64)
}

#[cfg(test)]
mod tests {
    use opengrid_types::{DataType, Date, Decimal, Timestamp, Value};
    use proptest::prelude::*;

    use super::*;
    use crate::ColumnBuilder;

    fn column(data_type: DataType, values: &[Value]) -> Column {
        let mut builder = ColumnBuilder::new(data_type, values.len());
        for value in values {
            builder.push(value).unwrap();
        }
        builder.finish()
    }

    /// The order spelled out on values — what the fast paths must agree with.
    fn compare(a: &Value, b: &Value) -> Ordering {
        match (a, b) {
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Int64(a), Value::Int64(b)) => a.cmp(b),
            (Value::Float64(a), Value::Float64(b)) => a.total_cmp(b),
            (Value::Decimal(a), Value::Decimal(b)) => a.value().cmp(&b.value()),
            (Value::Utf8(a), Value::Utf8(b)) => a.as_bytes().cmp(b.as_bytes()),
            (Value::Date(a), Value::Date(b)) => a.days_since_epoch().cmp(&b.days_since_epoch()),
            (Value::Timestamp(a), Value::Timestamp(b)) => a.micros().cmp(&b.micros()),
            other => panic!("not one type: {other:?}"),
        }
    }

    fn reference(columns: &[(Vec<Value>, bool, bool)], rows: usize) -> Vec<u32> {
        let mut positions: Vec<u32> = (0..rows as u32).collect();
        positions.sort_by(|a, b| {
            for (values, descending, nulls_first) in columns {
                let (a, b) = (&values[*a as usize], &values[*b as usize]);
                let order = match (a, b) {
                    (Value::Null, Value::Null) => Ordering::Equal,
                    (Value::Null, _) if *nulls_first => Ordering::Less,
                    (Value::Null, _) => Ordering::Greater,
                    (_, Value::Null) if *nulls_first => Ordering::Greater,
                    (_, Value::Null) => Ordering::Less,
                    (a, b) if *descending => compare(a, b).reverse(),
                    (a, b) => compare(a, b),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
            a.cmp(b)
        });
        positions
    }

    fn typed_values(data_type: DataType) -> BoxedStrategy<Value> {
        let value: BoxedStrategy<Value> = match data_type {
            DataType::Bool => any::<bool>().prop_map(Value::Bool).boxed(),
            DataType::Int64 => (-3i64..3).prop_map(Value::Int64).boxed(),
            DataType::Float64 => prop_oneof![
                Just(f64::NAN),
                Just(-f64::NAN),
                Just(0.0),
                Just(-0.0),
                Just(f64::INFINITY),
                Just(f64::NEG_INFINITY),
                -2.0f64..2.0,
            ]
            .prop_map(Value::Float64)
            .boxed(),
            // Wide decimals get coefficients past `i64`, so a key that were
            // cut to 64 bits would misorder them.
            DataType::Decimal { precision, scale } => {
                let wide = i128::from(i64::MAX) * 4;
                let coefficient = if precision > 18 {
                    prop_oneof![-300i128..300, Just(wide), Just(-wide), Just(wide + 1)].boxed()
                } else {
                    (-300i128..300).boxed()
                };
                coefficient
                    .prop_map(move |v| Value::Decimal(Decimal::new(v, scale)))
                    .boxed()
            }
            DataType::Utf8 => prop_oneof![
                Just(String::new()),
                Just("\0".to_owned()),
                Just("a\0".to_owned()),
                Just("ä".to_owned()),
                "[aZz]{0,3}",
            ]
            .prop_map(Value::Utf8)
            .boxed(),
            DataType::Date => (-2i32..2)
                .prop_map(|d| Value::Date(Date::from_days_since_epoch(d)))
                .boxed(),
            DataType::Timestamp => (-2i64..2)
                .prop_map(|t| Value::Timestamp(Timestamp::from_micros(t)))
                .boxed(),
        };
        prop_oneof![1 => Just(Value::Null), 4 => value].boxed()
    }

    fn data_types() -> impl Strategy<Value = DataType> {
        prop_oneof![
            Just(DataType::Bool),
            Just(DataType::Int64),
            Just(DataType::Float64),
            Just(DataType::Decimal {
                precision: 10,
                scale: 2
            }),
            // Past 18 digits the key stays an `i128`.
            Just(DataType::Decimal {
                precision: 38,
                scale: 2
            }),
            Just(DataType::Utf8),
            Just(DataType::Date),
            Just(DataType::Timestamp),
        ]
    }

    fn keys() -> impl Strategy<Value = Vec<(DataType, Vec<Value>, bool, bool)>> {
        (0usize..12, 1usize..4).prop_flat_map(|(rows, count)| {
            proptest::collection::vec(
                data_types().prop_flat_map(move |data_type| {
                    (
                        Just(data_type),
                        proptest::collection::vec(typed_values(data_type), rows),
                        any::<bool>(),
                        any::<bool>(),
                    )
                }),
                count,
            )
        })
    }

    proptest! {
        /// Both paths, every type, every direction and NULL placement, every
        /// page end: the order is the one spelled out on values.
        #[test]
        fn order_matches_the_reference(keys in keys(), end in 0usize..14) {
            let rows = keys[0].1.len();
            let columns: Vec<Column> = keys
                .iter()
                .map(|(data_type, values, _, _)| column(*data_type, values))
                .collect();
            let sort_keys: Vec<SortKey<'_>> = keys
                .iter()
                .zip(&columns)
                .map(|((_, _, descending, nulls_first), column)| SortKey {
                    column,
                    descending: *descending,
                    nulls_first: *nulls_first,
                })
                .collect();
            let spelled: Vec<(Vec<Value>, bool, bool)> = keys
                .iter()
                .map(|(_, values, d, n)| (values.clone(), *d, *n))
                .collect();
            let mut expected = reference(&spelled, rows);
            expected.truncate(end);
            prop_assert_eq!(order(&sort_keys, end).unwrap(), expected.clone());
            // The same order through the byte path: a second key that is
            // constant changes nothing.
            let constant = column(DataType::Int64, &vec![Value::Int64(0); rows]);
            let mut two = sort_keys.clone();
            two.push(SortKey { column: &constant, descending: false, nulls_first: false });
            prop_assert_eq!(order(&two, end).unwrap(), expected);
        }
    }

    #[test]
    fn total_key_orders_like_total_cmp() {
        let values = [
            -f64::NAN,
            f64::NEG_INFINITY,
            -1.5,
            -0.0,
            0.0,
            f64::MIN_POSITIVE,
            2.0,
            f64::INFINITY,
            f64::NAN,
        ];
        for a in values {
            for b in values {
                assert_eq!(total_key(a).cmp(&total_key(b)), a.total_cmp(&b), "{a} {b}");
            }
        }
    }
}
