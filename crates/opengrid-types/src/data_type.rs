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
    /// The most digits an `i128` coefficient holds throughout, and the limit
    /// `sum(Decimal)` is cast to (semantics rule S12).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical form, exactly as the conformance schema file writes it.
    #[test]
    fn the_json_form_is_the_one_already_in_use() {
        let cases = [
            (DataType::Int64, "\"int64\""),
            (DataType::Utf8, "\"utf8\""),
            (DataType::Bool, "\"bool\""),
            (DataType::Date, "\"date\""),
            (DataType::Timestamp, "\"timestamp\""),
            (DataType::Float64, "\"float64\""),
            (
                DataType::Decimal {
                    precision: 12,
                    scale: 2,
                },
                "{\"decimal\":{\"precision\":12,\"scale\":2}}",
            ),
        ];
        for (data_type, json) in cases {
            assert_eq!(opengrid_json::to_string(&data_type), json);
            assert_eq!(
                opengrid_json::from_str::<DataType>(json).unwrap(),
                data_type
            );
        }
    }

    /// Reading goes through the constructor, so an impossible decimal cannot
    /// enter through JSON.
    #[test]
    fn an_invalid_decimal_is_rejected_on_read() {
        for json in [
            "{\"decimal\":{\"precision\":0,\"scale\":0}}",
            "{\"decimal\":{\"precision\":2,\"scale\":5}}",
            "{\"decimal\":{\"precision\":39,\"scale\":2}}",
        ] {
            assert!(opengrid_json::from_str::<DataType>(json).is_err(), "{json}");
        }
        assert!(opengrid_json::from_str::<DataType>("\"int128\"").is_err());
    }
}
