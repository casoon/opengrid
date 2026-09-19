//! Looking a group up again — the one place that compares values.
//!
//! The pivot engine never decides an **order**; every grouping set is requested
//! sorted, so S3/S4 stay with the engine or the database that owns them. What is
//! unavoidable is asking "is this the same group as that one?" while stitching
//! the levels together, and that is equality, not ordering.

use opengrid_types::Value;

/// A group's identity, comparable and cheap to look up.
pub(crate) type Key = String;

/// Whether two group keys are the same group, by rule S7.
///
/// `NaN == NaN` and `-0.0 == 0.0` — PostgreSQL groups them that way and so does
/// the engine, so the pivot has to agree or a level would find no partner.
pub(crate) fn same_key(values: &[Value]) -> Key {
    let mut key = String::new();
    for value in values {
        key.push('\u{1f}');
        match value {
            Value::Null => key.push_str("\u{0}null"),
            // S7: the two floats that are not equal to themselves, and the two
            // zeroes that are.
            Value::Float64(number) if number.is_nan() => key.push_str("nan"),
            Value::Float64(number) if *number == 0.0 => key.push('0'),
            other => {
                key.push_str(&serde_json::to_string(other).expect("a value serializes into JSON"))
            }
        }
    }
    key
}
