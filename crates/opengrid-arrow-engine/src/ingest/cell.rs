//! Reading a cell as a typed value — one implementation for both formats.
//!
//! CSV and JSON differ in how a cell is *written*, not in what it *means*. For
//! `decimal`, `date` and `timestamp` the CSV text therefore goes through the
//! same wire coercion as a JSON cell (E13), so scale handling (rule S8) and UTC
//! normalization (rule S9) cannot drift apart between the two formats.

use opengrid_types::{DataType, Value};

/// Reads a CSV cell as the column's type.
pub(crate) fn from_text(raw: &str, data_type: DataType) -> Result<Value, String> {
    match data_type {
        DataType::Bool => match raw {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            other => Err(format!(
                "{other:?} is not a bool (expected \"true\" or \"false\")"
            )),
        },
        DataType::Int64 => raw
            .parse::<i64>()
            .map(Value::Int64)
            .map_err(|_| format!("{raw:?} is not an int64")),
        DataType::Float64 => parse_float(raw).map(Value::Float64),
        DataType::Utf8 => Ok(Value::Utf8(raw.to_owned())),
        // The wire coercion owns the spellings, the scale handling and the
        // normalization of these three.
        DataType::Decimal { .. } | DataType::Date | DataType::Timestamp => {
            from_json(serde_json::Value::String(raw.to_owned()), data_type)
        }
    }
}

/// Reads a JSON cell as the column's type, through the wire contract (E13).
pub(crate) fn from_json(cell: serde_json::Value, data_type: DataType) -> Result<Value, String> {
    Value::deserialize_typed(cell, &data_type).map_err(|error| error.to_string())
}

/// A plain number, plus exactly the three non-finite spellings of the wire
/// format (E13).
///
/// `nan` or `inf` are rejected: a NaN is a value with its own semantics (rule
/// S7), so it must not enter through a spelling nobody agreed on.
fn parse_float(raw: &str) -> Result<f64, String> {
    match raw {
        "NaN" => Ok(f64::NAN),
        "Infinity" => Ok(f64::INFINITY),
        "-Infinity" => Ok(f64::NEG_INFINITY),
        other => match other.parse::<f64>() {
            Ok(value) if value.is_finite() => Ok(value),
            _ => Err(format!("{other:?} is not a float64")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(raw: &str, data_type: DataType) -> Result<Value, String> {
        from_text(raw, data_type)
    }

    #[test]
    fn reads_the_plain_types() {
        assert_eq!(text("true", DataType::Bool).unwrap(), Value::Bool(true));
        assert_eq!(text("false", DataType::Bool).unwrap(), Value::Bool(false));
        assert_eq!(text("42", DataType::Int64).unwrap(), Value::Int64(42));
        assert_eq!(text("-7", DataType::Int64).unwrap(), Value::Int64(-7));
        assert_eq!(text("1.5", DataType::Float64).unwrap(), Value::Float64(1.5));
        assert_eq!(text("ä", DataType::Utf8).unwrap(), Value::Utf8("ä".into()));
    }

    #[test]
    fn a_text_cell_is_never_trimmed() {
        assert_eq!(
            text(" a ", DataType::Utf8).unwrap(),
            Value::Utf8(" a ".into())
        );
        assert!(text(" 1", DataType::Int64).is_err());
    }

    #[test]
    fn an_empty_cell_is_the_empty_string() {
        assert_eq!(
            text("", DataType::Utf8).unwrap(),
            Value::Utf8(String::new())
        );
        // … and not a NULL, and not a zero: a type mismatch or an error is fine,
        // silently reading it as 0 would not be (rule S14).
        assert!(text("", DataType::Int64).is_err());
        assert!(text("", DataType::Bool).is_err());
    }

    #[test]
    fn reads_the_non_finite_spellings_of_the_wire_format() {
        assert!(matches!(text("NaN", DataType::Float64), Ok(Value::Float64(v)) if v.is_nan()));
        assert_eq!(
            text("Infinity", DataType::Float64).unwrap(),
            Value::Float64(f64::INFINITY)
        );
        assert_eq!(
            text("-Infinity", DataType::Float64).unwrap(),
            Value::Float64(f64::NEG_INFINITY)
        );
        assert!(text("nan", DataType::Float64).is_err());
        assert!(text("inf", DataType::Float64).is_err());
        assert!(text("NAN", DataType::Float64).is_err());
    }

    #[test]
    fn keeps_negative_zero_and_exponents() {
        match text("-0.0", DataType::Float64).unwrap() {
            Value::Float64(value) => {
                assert_eq!(value, 0.0);
                assert!(value.is_sign_negative());
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            text("1e3", DataType::Float64).unwrap(),
            Value::Float64(1000.0)
        );
    }

    #[test]
    fn decimals_are_rescaled_to_the_column() {
        let column = DataType::decimal(12, 2).unwrap();
        assert_eq!(
            text("10.00", column).unwrap(),
            Value::Decimal(opengrid_types::Decimal::new(1000, 2))
        );
        assert_eq!(
            text("10", column).unwrap(),
            Value::Decimal(opengrid_types::Decimal::new(1000, 2))
        );
        assert_eq!(
            text("-0.05", column).unwrap(),
            Value::Decimal(opengrid_types::Decimal::new(-5, 2))
        );
        // More digits than the column holds is an error, not a rounding.
        assert!(text("1.005", column).is_err());
        // So is a value that does not fit the precision.
        assert!(text("10000000000.00", column).is_err());
    }

    #[test]
    fn reads_dates_and_timestamps() {
        assert_eq!(
            text("2025-12-31", DataType::Date).unwrap(),
            Value::Date(opengrid_types::Date::from_ymd(2025, 12, 31).unwrap())
        );
        assert!(text("2025-02-30", DataType::Date).is_err());
        assert!(text("31.12.2025", DataType::Date).is_err());
        assert_eq!(
            text("2026-03-01T00:00:00.5Z", DataType::Timestamp).unwrap(),
            Value::Timestamp(opengrid_types::Timestamp::from_micros(
                1_772_323_200_500_000
            ))
        );
        // S9: an instant without a zone is not a timestamp.
        assert!(text("2026-03-01T00:00:00", DataType::Timestamp).is_err());
        assert!(text("2026-03-01T00:00:00+01:00", DataType::Timestamp).is_err());
    }

    #[test]
    fn json_cells_take_the_same_route() {
        assert_eq!(
            from_json(serde_json::json!("2025-12-31"), DataType::Date).unwrap(),
            text("2025-12-31", DataType::Date).unwrap()
        );
        assert_eq!(
            from_json(serde_json::json!(-0.0), DataType::Float64).unwrap(),
            text("-0.0", DataType::Float64).unwrap()
        );
        assert_eq!(
            from_json(serde_json::json!(10), DataType::Float64).unwrap(),
            Value::Float64(10.0)
        );
        assert_eq!(
            from_json(serde_json::Value::Null, DataType::Int64).unwrap(),
            Value::Null
        );
        assert!(from_json(serde_json::json!("10.00"), DataType::Float64).is_err());
    }
}
