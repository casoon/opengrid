//! Scaled integer columns (issue #537): money stored as cents, read and typed
//! as euros.
//!
//! An `int64` column with a `scale` of 2 stores `612` for what a reader reads
//! as `6,12`. Everything a reader **reads and types** is the scaled value; what
//! the page and the wire see — the query, the events, the view's filters, the
//! exports — stays the stored integer. This module is the one place that turns
//! one into the other, so that no surface does its own arithmetic:
//!
//! * [`shown`] — a stored value (or an aggregate of stored values) as the value
//!   a reader reads. A sum stays exact: integers are added as integers and only
//!   the result is given a decimal point.
//! * [`parse`] / [`parse_cell_scaled`] — what a reader typed or pasted, read with
//!   [`parse_number`](crate::paste::parse_number)'s rules, as the stored
//!   integer. More decimals than the scale are refused with
//!   [`NumberProblem::Decimals`], never rounded.
//! * [`stored_text`] / [`typed_text`] — a filter's value text from what was
//!   typed to what is stored, and back.

use opengrid_types::{DataType, Decimal, Value};

use crate::paste::{Notation, NumberProblem, number_text, parse_cell, parse_number};

/// The largest scale: `10^18` is the largest power of ten an `i64` holds.
pub const MAX_SCALE: u8 = 18;

/// A stored value as the reader reads it: an integer as a decimal of `scale`
/// places, the decimal average of stored values with the places added, a float
/// average divided. Anything else is as it was.
pub fn shown(value: &Value, scale: u8) -> Value {
    match value {
        Value::Int64(stored) => Value::Decimal(Decimal::new(i128::from(*stored), scale)),
        Value::Decimal(decimal) => match decimal.scale().checked_add(scale) {
            Some(places) => Value::Decimal(Decimal::new(decimal.value(), places)),
            None => value.clone(),
        },
        Value::Float64(stored) => Value::Float64(stored / 10f64.powi(i32::from(scale))),
        other => other.clone(),
    }
}

/// A stored integer's text (`612`) as the scaled value's text in the wire
/// notation (`6.12`); text that is not an integer is as it was.
pub fn shown_text(stored: &str, scale: u8) -> String {
    match stored.trim().parse::<i128>() {
        Ok(stored) => Decimal::new(stored, scale).to_string(),
        Err(_) => stored.to_owned(),
    }
}

/// What a reader typed (`6,12`), in the page's notation or the wire's, as the
/// stored integer (`612`).
pub fn parse(text: &str, scale: u8, notation: &Notation) -> Result<Value, NumberProblem> {
    let scaled = DataType::Decimal {
        precision: 38,
        scale,
    };
    let Value::Decimal(decimal) = parse_number(text, scaled, notation)? else {
        return Err(NumberProblem::NotANumber);
    };
    i64::try_from(decimal.value())
        .map(Value::Int64)
        .map_err(|_| NumberProblem::TooLarge)
}

/// [`parse_cell`] for a column that may be scaled: an `int64` with a `scale`
/// reads what was pasted as the scaled value.
pub fn parse_cell_scaled(
    text: &str,
    data_type: DataType,
    nullable: bool,
    scale: Option<u8>,
    notation: &Notation,
) -> Option<Value> {
    match scale {
        Some(scale) if data_type == DataType::Int64 && !text.trim().is_empty() => {
            parse(text, scale, notation).ok()
        }
        _ => parse_cell(text, data_type, nullable, notation),
    }
}

/// A filter's value as typed (`5,5`) as the stored integer's text (`550`) — what
/// the view and the query hold. Text that cannot be read is **as it was**, so
/// that the filter refuses it by what the reader typed; an amount too large to
/// be stored is the integer it would be, which no integer filter takes.
pub fn stored_text(typed: &str, scale: u8, notation: &Notation) -> String {
    let scaled = DataType::Decimal {
        precision: 38,
        scale,
    };
    match parse_number(typed, scaled, notation) {
        Ok(Value::Decimal(stored)) => stored.value().to_string(),
        _ => typed.to_owned(),
    }
}

/// A filter's stored value (`550`) as the reader reads and types it in the
/// page's notation (`5,50`); text that is not an integer is as it was.
pub fn typed_text(stored: &str, scale: u8, notation: &Notation) -> String {
    match stored.trim().parse::<i128>() {
        Ok(stored) => number_text(&Decimal::new(stored, scale).to_string(), notation),
        Err(_) => stored.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GERMAN: Notation = Notation {
        decimal: ',',
        group: Some('.'),
    };
    const ENGLISH: Notation = Notation {
        decimal: '.',
        group: Some(','),
    };

    #[test]
    fn a_stored_integer_is_shown_with_its_decimal_point() {
        let text = |value: i64, scale| match shown(&Value::Int64(value), scale) {
            Value::Decimal(decimal) => decimal.to_string(),
            other => panic!("not a decimal: {other:?}"),
        };
        assert_eq!(text(612, 2), "6.12");
        assert_eq!(text(5, 2), "0.05");
        assert_eq!(text(-5, 2), "-0.05");
        assert_eq!(text(123_450, 2), "1234.50");
        assert_eq!(text(7, 3), "0.007");
    }

    /// An aggregate of stored values is shown like a stored value: a sum stays
    /// an exact integer until the point is set, an average carries more places.
    #[test]
    fn an_aggregate_of_stored_values_is_shown_in_the_scale_too() {
        assert_eq!(
            shown(&Value::Decimal(Decimal::new(55_600, 2)), 2),
            Value::Decimal(Decimal::new(55_600, 4)),
            "the average of 612 and 500 is 556.00, which is 5.5600"
        );
        assert_eq!(shown(&Value::Float64(612.5), 2), Value::Float64(6.125));
        assert_eq!(shown(&Value::Null, 2), Value::Null);
        // The sum is exact where a float is not: 0.1 + 0.2 in cents.
        let sum = 10 + 20;
        assert_eq!(
            shown(&Value::Int64(sum), 2),
            Value::Decimal(Decimal::new(30, 2))
        );
    }

    #[test]
    fn what_is_typed_is_the_stored_integer() {
        for (text, notation, stored) in [
            ("6,12", GERMAN, 612),
            ("6.12", GERMAN, 612),
            ("1.234,50", GERMAN, 123_450),
            ("6", GERMAN, 600),
            ("-0,05", GERMAN, -5),
            ("6,5", GERMAN, 650),
            ("1,234.50", ENGLISH, 123_450),
            ("6.12", ENGLISH, 612),
            (" 6.12 ", ENGLISH, 612),
        ] {
            assert_eq!(
                parse(text, 2, &notation),
                Ok(Value::Int64(stored)),
                "{text:?}"
            );
        }
        assert_eq!(parse("1,5", 3, &GERMAN), Ok(Value::Int64(1500)));
    }

    #[test]
    fn more_decimals_than_the_scale_are_refused_and_what_is_not_a_number_too() {
        assert_eq!(
            parse("6,123", 2, &GERMAN),
            Err(NumberProblem::Decimals { scale: 2 })
        );
        assert_eq!(
            parse("6.123", 2, &ENGLISH),
            Err(NumberProblem::Decimals { scale: 2 })
        );
        // Zeros past the scale lose nothing.
        assert_eq!(parse("6,120", 2, &GERMAN), Ok(Value::Int64(612)));
        assert_eq!(parse("abc", 2, &GERMAN), Err(NumberProblem::NotANumber));
        assert_eq!(parse("", 2, &GERMAN), Err(NumberProblem::NotANumber));
    }

    /// The scaled value must fit once multiplied: the largest `i64` is
    /// 92233720368547758,07 in cents.
    #[test]
    fn a_value_that_does_not_fit_when_scaled_is_refused() {
        assert_eq!(
            parse("92233720368547758,07", 2, &GERMAN),
            Ok(Value::Int64(i64::MAX))
        );
        assert_eq!(
            parse("92233720368547758,08", 2, &GERMAN),
            Err(NumberProblem::TooLarge)
        );
        assert_eq!(
            parse("92233720368547759", 2, &GERMAN),
            Err(NumberProblem::TooLarge)
        );
    }

    #[test]
    fn a_pasted_cell_reads_scaled_only_on_a_scaled_integer_column() {
        let read =
            |text: &str, data_type, scale| parse_cell_scaled(text, data_type, true, scale, &GERMAN);
        assert_eq!(
            read("6,12", DataType::Int64, Some(2)),
            Some(Value::Int64(612))
        );
        assert_eq!(read("6,123", DataType::Int64, Some(2)), None);
        // An empty cell is NULL, as it is on any column that may hold one.
        assert_eq!(read("", DataType::Int64, Some(2)), Some(Value::Null));
        // Without a scale it is a whole number, as before.
        assert_eq!(read("612", DataType::Int64, None), Some(Value::Int64(612)));
        assert_eq!(read("6,12", DataType::Int64, None), None);
    }

    #[test]
    fn a_filter_value_goes_to_the_stored_text_and_back() {
        assert_eq!(stored_text("5", 2, &GERMAN), "500");
        assert_eq!(stored_text("5,5", 2, &GERMAN), "550");
        assert_eq!(stored_text("1.234,50", 2, &GERMAN), "123450");
        assert_eq!(typed_text("550", 2, &GERMAN), "5,50");
        assert_eq!(typed_text("550", 2, &ENGLISH), "5.50");
        assert_eq!(typed_text("-5", 2, &GERMAN), "-0,05");
        assert_eq!(typed_text("123450", 2, &GERMAN), "1234,50");
        // Refused text is left as typed, so that the filter names it.
        assert_eq!(stored_text("5,555", 2, &GERMAN), "5,555");
        assert_eq!(stored_text("abc", 2, &GERMAN), "abc");
        assert_eq!(typed_text("abc", 2, &GERMAN), "abc");
        // Too large to be stored: a number that no integer literal takes, so
        // the filter refuses it rather than reading it as another amount.
        assert!(
            stored_text("92233720368547759", 2, &GERMAN)
                .parse::<i64>()
                .is_err()
        );
    }

    /// The text the filter reads from a typed amount and the one it opens a
    /// stored value with are each other's inverse.
    #[test]
    fn typed_and_stored_text_round_trip() {
        for stored in ["0", "5", "-5", "612", "123450", "-99999"] {
            assert_eq!(
                stored_text(&typed_text(stored, 2, &GERMAN), 2, &GERMAN),
                stored
            );
            assert_eq!(
                stored_text(&typed_text(stored, 2, &ENGLISH), 2, &ENGLISH),
                stored
            );
        }
        assert_eq!(shown_text("612", 2), "6.12");
        assert_eq!(shown_text("", 2), "");
    }
}
