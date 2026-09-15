use crate::ValueError;

/// The closed set of value types in opengrid V1
/// (plan/spezifikation/02-query-modell.md §Typsystem).
///
/// Deliberately narrow: no extra integer widths, no lists or structs, no time
/// zones beyond UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataType {
    Bool,
    Int64,
    Float64,
    /// Exact fixed-point number with `precision` total digits and `scale` digits
    /// after the decimal point. Backed by `i128`.
    Decimal {
        precision: u8,
        scale: u8,
    },
    /// UTF-8 string, compared and sorted by Unicode code point (collation
    /// `binary`, semantics rule S4).
    Utf8,
    /// Calendar date without a time zone.
    Date,
    /// Instant in UTC with microsecond resolution (semantics rule S9).
    Timestamp,
}

impl DataType {
    /// Arrow's `Decimal128` maximum precision, and the limit `sum(Decimal)` is
    /// cast to (semantics rule S12).
    pub const MAX_DECIMAL_PRECISION: u8 = 38;

    /// Builds a decimal type, rejecting invalid precision/scale combinations.
    ///
    /// A negative scale is unrepresentable by construction (`scale: u8`).
    pub fn decimal(precision: u8, scale: u8) -> Result<Self, ValueError> {
        if precision == 0 || precision > Self::MAX_DECIMAL_PRECISION || scale > precision {
            return Err(ValueError::InvalidDecimalType { precision, scale });
        }
        Ok(DataType::Decimal { precision, scale })
    }

    /// True for any `Decimal` variant.
    pub fn is_decimal(&self) -> bool {
        matches!(self, DataType::Decimal { .. })
    }

    /// True for types ordered exactly (everything except `Float64`).
    pub fn is_exact(&self) -> bool {
        !matches!(self, DataType::Float64)
    }
}
