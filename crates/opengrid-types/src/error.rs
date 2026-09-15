use std::error::Error;
use std::fmt;

use crate::DataType;

/// A string does not satisfy the identifier rule
/// `^[A-Za-z_][A-Za-z0-9_]{0,62}$` (63 characters max).
///
/// The same rule governs field names, data source ids and — later — SQL
/// identifiers (plan/spezifikation/02-query-modell.md, point 03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidIdentifier(pub String);

impl fmt::Display for InvalidIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid identifier {:?} (expected ^[A-Za-z_][A-Za-z0-9_]{{0,62}}$)",
            self.0
        )
    }
}

impl Error for InvalidIdentifier {}

/// Everything that can go wrong while interpreting a JSON scalar as a typed
/// [`Value`](crate::Value) or while building a [`DataType`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    /// The JSON shape does not match the expected column type.
    TypeMismatch {
        /// The type the value was interpreted as.
        expected: DataType,
        /// The JSON shape that was found (`"null"`, `"bool"`, `"integer"`, …).
        found: &'static str,
    },
    /// A `Decimal` JSON string could not be parsed.
    InvalidDecimal(String),
    /// A `Date` string is not a valid `YYYY-MM-DD`.
    InvalidDate(String),
    /// A `Timestamp` string is not a valid ISO-8601 instant with a `Z` suffix.
    InvalidTimestamp(String),
    /// A value does not fit the target precision and scale.
    DecimalOutOfRange { precision: u8, scale: u8 },
    /// The requested decimal type itself is invalid (precision 0, > 38 or scale > precision).
    InvalidDecimalType { precision: u8, scale: u8 },
    /// An integer value does not fit the target type.
    OutOfRange,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::TypeMismatch { expected, found } => {
                write!(f, "expected {expected}, found JSON {found}")
            }
            ValueError::InvalidDecimal(s) => write!(f, "invalid decimal: {s:?}"),
            ValueError::InvalidDate(s) => write!(f, "invalid date: {s:?}"),
            ValueError::InvalidTimestamp(s) => write!(f, "invalid timestamp: {s:?}"),
            ValueError::DecimalOutOfRange { precision, scale } => {
                write!(f, "decimal does not fit decimal({precision}, {scale})")
            }
            ValueError::InvalidDecimalType { precision, scale } => {
                write!(f, "invalid decimal type decimal({precision}, {scale})")
            }
            ValueError::OutOfRange => write!(f, "value out of range"),
        }
    }
}

impl Error for ValueError {}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataType::Bool => f.write_str("bool"),
            DataType::Int64 => f.write_str("int64"),
            DataType::Float64 => f.write_str("float64"),
            DataType::Decimal { precision, scale } => {
                write!(f, "decimal({precision}, {scale})")
            }
            DataType::Utf8 => f.write_str("utf8"),
            DataType::Date => f.write_str("date"),
            DataType::Timestamp => f.write_str("timestamp"),
        }
    }
}
