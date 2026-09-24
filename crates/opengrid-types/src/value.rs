use std::fmt;

use serde::de::{self, Deserializer, Visitor};
use serde::{Serialize, Serializer};

use crate::{DataType, ValueError};

// ---------------------------------------------------------------------------
// Civil-calendar helpers (Howard Hinnant's algorithms), no external date crate.
// ---------------------------------------------------------------------------

fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a civil date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400; // [0, 399]
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) as i64 + 2) / 5 + day as i64 - 1; // [0, 365]
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year; // [0, 146096]
    era * 146097 + day_of_era - 719468
}

/// Civil `(year, month, day)` for a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let day_of_era = z - era * 146097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_prime = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32; // [1, 31]
    let month = (if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }) as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

// ---------------------------------------------------------------------------
// Decimal
// ---------------------------------------------------------------------------

/// Exact fixed-point number: `value * 10^(-scale)`, backed by `i128`.
///
/// The scale is unsigned, so a negative scale cannot be represented (point 03).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Decimal {
    value: i128,
    scale: u8,
}

impl Decimal {
    /// Wraps a coefficient and its scale.
    pub fn new(value: i128, scale: u8) -> Self {
        Self { value, scale }
    }

    /// The `i128` coefficient.
    pub fn value(&self) -> i128 {
        self.value
    }

    /// Digits after the decimal point.
    pub fn scale(&self) -> u8 {
        self.scale
    }

    /// Number of decimal digits in the coefficient (at least 1).
    pub fn digit_count(&self) -> u32 {
        let magnitude = self.value.unsigned_abs();
        let mut digits = 1;
        let mut rest = magnitude;
        while rest >= 10 {
            rest /= 10;
            digits += 1;
        }
        digits
    }

    /// Returns the value at `target_scale`, or `None` when that would lose digits.
    fn rescale(&self, target_scale: u8) -> Option<Self> {
        match target_scale.cmp(&self.scale) {
            std::cmp::Ordering::Equal => Some(*self),
            std::cmp::Ordering::Less => {
                let factor = pow10((self.scale - target_scale) as u32)?;
                if self.value % factor != 0 {
                    return None;
                }
                Some(Self::new(self.value / factor, target_scale))
            }
            std::cmp::Ordering::Greater => {
                let factor = pow10((target_scale - self.scale) as u32)?;
                Some(Self::new(self.value.checked_mul(factor)?, target_scale))
            }
        }
    }
}

fn pow10(exponent: u32) -> Option<i128> {
    let mut result: i128 = 1;
    for _ in 0..exponent {
        result = result.checked_mul(10)?;
    }
    Some(result)
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let magnitude = self.value.unsigned_abs().to_string();
        let scale = self.scale as usize;
        if self.value < 0 {
            f.write_str("-")?;
        }
        if scale == 0 {
            f.write_str(&magnitude)
        } else if magnitude.len() <= scale {
            write!(f, "0.{}{}", "0".repeat(scale - magnitude.len()), magnitude)
        } else {
            let split = magnitude.len() - scale;
            write!(f, "{}.{}", &magnitude[..split], &magnitude[split..])
        }
    }
}

/// Parses a plain decimal string (`-?digits[.digits]`) into a [`Decimal`].
fn parse_decimal(s: &str) -> Result<Decimal, ValueError> {
    let (negative, rest) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((int_part, frac_part)) => (int_part, frac_part),
        None => (rest, ""),
    };
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
        || (int_part.is_empty() && frac_part.is_empty())
    {
        return Err(ValueError::InvalidDecimal(s.to_owned()));
    }
    let scale =
        u8::try_from(frac_part.len()).map_err(|_| ValueError::InvalidDecimal(s.to_owned()))?;
    // The digits accumulate straight into the i128 — what `i128::parse` of the
    // concatenated digits did, without allocating the concatenation per cell
    // (point 45). An overflow is the same error it was.
    let mut magnitude: i128 = 0;
    for digit in int_part.bytes().chain(frac_part.bytes()) {
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(i128::from(digit - b'0')))
            .ok_or_else(|| ValueError::InvalidDecimal(s.to_owned()))?;
    }
    let value = if negative { -magnitude } else { magnitude };
    Ok(Decimal::new(value, scale))
}

/// `10^n` for every `n` a decimal precision can take (0..=38), as `u128`.
const POW10: [u128; 39] = {
    let mut table = [1u128; 39];
    let mut n = 1;
    while n < 39 {
        table[n] = table[n - 1] * 10;
        n += 1;
    }
    table
};

/// Whether a decimal has more digits than `precision` allows — what
/// `digit_count() > precision` says, with one comparison instead of a division
/// per digit (point 45).
fn exceeds_precision(value: &Decimal, precision: u8) -> bool {
    match POW10.get(usize::from(precision)) {
        // `digit_count` is at least 1, so a precision of 0 holds nothing.
        Some(_) if precision == 0 => true,
        Some(limit) => value.value().unsigned_abs() >= *limit,
        None => value.digit_count() > u32::from(precision),
    }
}

// ---------------------------------------------------------------------------
// Date and Timestamp
// ---------------------------------------------------------------------------

/// A calendar date without a time zone, stored as days since 1970-01-01.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    days: i32,
}

impl Date {
    /// A date from its civil year, month and day, if the combination exists.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year as i64, month) {
            return None;
        }
        let days = days_from_civil(year as i64, month, day);
        i32::try_from(days).ok().map(|days| Self { days })
    }

    /// A date from a day count since 1970-01-01.
    pub fn from_days_since_epoch(days: i32) -> Self {
        Self { days }
    }

    /// Days since 1970-01-01 (negative before the epoch).
    pub fn days_since_epoch(&self) -> i32 {
        self.days
    }

    /// The civil `(year, month, day)`.
    pub fn ymd(&self) -> (i32, u32, u32) {
        let (year, month, day) = civil_from_days(self.days as i64);
        (year as i32, month, day)
    }

    /// Parses a strict `YYYY-MM-DD` string.
    pub fn parse(s: &str) -> Result<Self, ValueError> {
        let invalid = || ValueError::InvalidDate(s.to_owned());
        if s.len() != 10 || &s[4..5] != "-" || &s[7..8] != "-" {
            return Err(invalid());
        }
        let year: i32 = s[0..4].parse().map_err(|_| invalid())?;
        let month: u32 = s[5..7].parse().map_err(|_| invalid())?;
        let day: u32 = s[8..10].parse().map_err(|_| invalid())?;
        Self::from_ymd(year, month, day).ok_or_else(invalid)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.ymd();
        write!(f, "{year:04}-{month:02}-{day:02}")
    }
}

const MICROS_PER_DAY: i64 = 86_400_000_000;

/// An instant in UTC with microsecond resolution, stored as µs since the epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    micros: i64,
}

impl Timestamp {
    /// A timestamp from microseconds since 1970-01-01T00:00:00Z.
    pub fn from_micros(micros: i64) -> Self {
        Self { micros }
    }

    /// Microseconds since 1970-01-01T00:00:00Z.
    pub fn micros(&self) -> i64 {
        self.micros
    }

    /// A timestamp from a civil time in UTC, if the fields are in range.
    pub fn from_ymd_hms(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        micro: u32,
    ) -> Option<Self> {
        let date = Date::from_ymd(year, month, day)?;
        if hour > 23 || minute > 59 || second > 59 || micro > 999_999 {
            return None;
        }
        let time = hour as i64 * 3_600_000_000
            + minute as i64 * 60_000_000
            + second as i64 * 1_000_000
            + micro as i64;
        let micros = (date.days_since_epoch() as i64)
            .checked_mul(MICROS_PER_DAY)?
            .checked_add(time)?;
        Some(Self { micros })
    }

    /// The civil date in **UTC** (rule S9 — no time zone is ever applied).
    ///
    /// Floor division, so it is the calendar day before the epoch too.
    pub fn date(&self) -> Date {
        Date::from_days_since_epoch(self.micros.div_euclid(MICROS_PER_DAY) as i32)
    }

    /// Parses a strict `YYYY-MM-DDTHH:MM:SS[.ffffff]Z` string.
    pub fn parse(s: &str) -> Result<Self, ValueError> {
        let invalid = || ValueError::InvalidTimestamp(s.to_owned());
        let (date_part, rest) = s.split_once('T').ok_or_else(invalid)?;
        let time_part = rest.strip_suffix('Z').ok_or_else(invalid)?;
        let date = Date::parse(date_part).map_err(|_| invalid())?;
        let (hms, frac) = match time_part.split_once('.') {
            Some((hms, frac)) => (hms, frac),
            None => (time_part, ""),
        };
        if frac.len() > 6 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let mut fields = hms.split(':');
        let mut number = || -> Result<u32, ValueError> {
            let raw = fields.next().ok_or_else(invalid)?;
            if raw.is_empty() || raw.len() > 2 || !raw.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            raw.parse::<u32>().map_err(|_| invalid())
        };
        let hour = number()?;
        let minute = number()?;
        let second = number()?;
        if fields.next().is_some() {
            return Err(invalid());
        }
        // Up to six digits, padded on the right to microseconds — what parsing
        // `format!("{frac:0<6}")` did, without the string (point 45).
        let micro: u32 = frac
            .bytes()
            .chain(std::iter::repeat(b'0'))
            .take(6)
            .fold(0, |micro, digit| micro * 10 + u32::from(digit - b'0'));
        if hour > 23 || minute > 59 || second > 59 {
            return Err(invalid());
        }
        let time = i64::from(hour) * 3_600_000_000
            + i64::from(minute) * 60_000_000
            + i64::from(second) * 1_000_000
            + i64::from(micro);
        let micros = (date.days_since_epoch() as i64)
            .checked_mul(MICROS_PER_DAY)
            .and_then(|base| base.checked_add(time))
            .ok_or_else(invalid)?;
        Ok(Self { micros })
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let days = self.micros.div_euclid(MICROS_PER_DAY);
        let rem = self.micros.rem_euclid(MICROS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        let hour = rem / 3_600_000_000;
        let minute = (rem / 60_000_000) % 60;
        let second = (rem / 1_000_000) % 60;
        let micro = rem % 1_000_000;
        write!(
            f,
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}"
        )?;
        if micro != 0 {
            write!(f, ".{micro:06}")?;
        }
        f.write_str("Z")
    }
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// A single typed cell value: `Null` plus exactly one variant per [`DataType`].
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int64(i64),
    Float64(f64),
    Decimal(Decimal),
    Utf8(String),
    Date(Date),
    Timestamp(Timestamp),
}

impl Value {
    /// Parses a JSON scalar into a `Value` of the given column type.
    ///
    /// This is the typed counterpart of the plain [`Serialize`] representation:
    /// JSON alone cannot tell a decimal string from a `Utf8` string, so the
    /// caller supplies the column type (see the column-oriented wire format, E6).
    /// `null` is accepted for every type.
    pub fn deserialize_typed<'de, D>(
        deserializer: D,
        data_type: &DataType,
    ) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = deserializer.deserialize_any(RawVisitor)?;
        coerce(raw, data_type).map_err(de::Error::custom)
    }

    /// Reads a value of the given column type from its string spelling in the
    /// wire format (E13): a decimal (S8: rescaled, never rounded), a date, a UTC
    /// timestamp (S9), one of the non-finite floats, or a string.
    ///
    /// This **is** the string branch of [`Value::deserialize_typed`] — a JSON
    /// string and a CSV cell go through the same code, so the two formats cannot
    /// drift apart (plan point 45). It exists separately so that a caller holding
    /// a `&str` does not have to wrap it in a JSON value first.
    pub fn from_wire_str(text: &str, data_type: &DataType) -> Result<Self, ValueError> {
        match data_type {
            DataType::Utf8 => Ok(Value::Utf8(text.to_owned())),
            _ => coerce_str(text, data_type),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => serializer.serialize_none(),
            Value::Bool(v) => serializer.serialize_bool(*v),
            Value::Int64(v) => serializer.serialize_i64(*v),
            Value::Float64(v) => match non_finite_name(*v) {
                Some(name) => serializer.serialize_str(name),
                None => serializer.serialize_f64(*v),
            },
            Value::Decimal(v) => serializer.serialize_str(&v.to_string()),
            Value::Utf8(v) => serializer.serialize_str(v),
            Value::Date(v) => serializer.serialize_str(&v.to_string()),
            Value::Timestamp(v) => serializer.serialize_str(&v.to_string()),
        }
    }
}

/// The JSON shapes a typed value can be read from.
enum Raw {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
}

impl Raw {
    fn kind(&self) -> &'static str {
        match self {
            Raw::Null => "null",
            Raw::Bool(_) => "bool",
            Raw::Int(_) | Raw::UInt(_) => "integer",
            Raw::Float(_) => "number",
            Raw::Str(_) => "string",
        }
    }
}

struct RawVisitor;

impl<'de> Visitor<'de> for RawVisitor {
    type Value = Raw;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON scalar (null, bool, number or string)")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Raw, E> {
        Ok(Raw::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Raw, E> {
        Ok(Raw::Int(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Raw, E> {
        Ok(Raw::UInt(v))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Raw, E> {
        Ok(Raw::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Raw, E> {
        Ok(Raw::Str(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Raw, E> {
        Ok(Raw::Str(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Raw, E> {
        Ok(Raw::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Raw, E> {
        Ok(Raw::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Raw, D::Error> {
        deserializer.deserialize_any(RawVisitor)
    }
}

/// The JSON notation for a non-finite `Float64`.
///
/// JSON has no `NaN` and no infinities, so a `serialize_f64` would silently emit
/// `null` and turn a NaN into a NULL — a different value with different semantics
/// (rule S1 vs S7). Non-finite floats therefore travel as strings, in both
/// directions (plan/spezifikation/02-query-modell.md §Typsystem).
fn non_finite_name(value: f64) -> Option<&'static str> {
    if value.is_nan() {
        Some("NaN")
    } else if value == f64::INFINITY {
        Some("Infinity")
    } else if value == f64::NEG_INFINITY {
        Some("-Infinity")
    } else {
        None
    }
}

/// The inverse of [`non_finite_name`]. `None` for any other string, so a typo in a
/// query literal stays a type mismatch.
fn parse_non_finite(name: &str) -> Option<f64> {
    match name {
        "NaN" => Some(f64::NAN),
        "Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

fn coerce(raw: Raw, data_type: &DataType) -> Result<Value, ValueError> {
    let mismatch = |found: &'static str| ValueError::TypeMismatch {
        expected: *data_type,
        found,
    };
    match (raw, data_type) {
        (Raw::Null, _) => Ok(Value::Null),
        (Raw::Bool(b), DataType::Bool) => Ok(Value::Bool(b)),
        (Raw::Int(i), DataType::Int64) => Ok(Value::Int64(i)),
        (Raw::UInt(u), DataType::Int64) => i64::try_from(u)
            .map(Value::Int64)
            .map_err(|_| mismatch("integer")),
        (Raw::Float(f), DataType::Float64) => Ok(Value::Float64(f)),
        (Raw::Int(i), DataType::Float64) => Ok(Value::Float64(i as f64)),
        (Raw::UInt(u), DataType::Float64) => Ok(Value::Float64(u as f64)),
        // The owned string moves into the value; every other string goes
        // through the one parser both formats share.
        (Raw::Str(s), DataType::Utf8) => Ok(Value::Utf8(s)),
        (Raw::Str(s), _) => coerce_str(&s, data_type),
        (other, _) => Err(mismatch(other.kind())),
    }
}

/// The string spellings of every non-string type: what [`coerce`] does with a
/// JSON string, and what [`Value::from_wire_str`] does with a CSV cell.
fn coerce_str(s: &str, data_type: &DataType) -> Result<Value, ValueError> {
    match data_type {
        DataType::Float64 => {
            parse_non_finite(s)
                .map(Value::Float64)
                .ok_or(ValueError::TypeMismatch {
                    expected: *data_type,
                    found: "string",
                })
        }
        DataType::Utf8 => Ok(Value::Utf8(s.to_owned())),
        DataType::Decimal { precision, scale } => {
            let parsed = parse_decimal(s)?;
            let rescaled = parsed
                .rescale(*scale)
                .ok_or(ValueError::DecimalOutOfRange {
                    precision: *precision,
                    scale: *scale,
                })?;
            if exceeds_precision(&rescaled, *precision) {
                return Err(ValueError::DecimalOutOfRange {
                    precision: *precision,
                    scale: *scale,
                });
            }
            Ok(Value::Decimal(rescaled))
        }
        DataType::Date => Ok(Value::Date(Date::parse(s)?)),
        DataType::Timestamp => Ok(Value::Timestamp(Timestamp::parse(s)?)),
        DataType::Bool | DataType::Int64 => Err(ValueError::TypeMismatch {
            expected: *data_type,
            found: "string",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parsers of point 45 read digits without building a string; these are
    /// the edges where that could differ from parsing the concatenation.
    #[test]
    fn decimal_and_fraction_parsing_keep_their_edges() {
        let decimal = |text: &str, precision, scale| {
            Value::from_wire_str(text, &DataType::decimal(precision, scale).unwrap())
        };
        assert_eq!(
            decimal("+0001.50", 12, 2),
            Ok(Value::Decimal(Decimal::new(150, 2)))
        );
        assert_eq!(decimal("-0", 12, 2), Ok(Value::Decimal(Decimal::new(0, 2))));
        assert_eq!(
            decimal(".5", 12, 2),
            Ok(Value::Decimal(Decimal::new(50, 2)))
        );
        assert_eq!(
            decimal("5.", 12, 2),
            Ok(Value::Decimal(Decimal::new(500, 2)))
        );
        // The precision boundary: 12 digits fit, 13 do not.
        assert_eq!(
            decimal("9999999999.99", 12, 2),
            Ok(Value::Decimal(Decimal::new(999_999_999_999, 2)))
        );
        assert!(decimal("10000000000.00", 12, 2).is_err());
        assert!(decimal("99999999999999999999999999999999999999", 38, 0).is_ok());
        // Past i128: the same error parsing the concatenation gave.
        assert!(matches!(
            decimal("1701411834604692317316873037158841057280", 38, 0),
            Err(ValueError::InvalidDecimal(_))
        ));
        assert!(decimal("1.2.3", 12, 2).is_err());
        assert!(decimal(".", 12, 2).is_err());

        for (text, micros) in [
            ("1970-01-01T00:00:00Z", 0),
            ("1970-01-01T00:00:00.1Z", 100_000),
            ("1970-01-01T00:00:00.12Z", 120_000),
            ("1970-01-01T00:00:00.000001Z", 1),
            ("1970-01-01T00:00:00.999999Z", 999_999),
        ] {
            assert_eq!(
                Timestamp::parse(text).map(|t| t.micros()),
                Ok(micros),
                "{text}"
            );
        }
        assert!(Timestamp::parse("1970-01-01T00:00:00.1234567Z").is_err());
        assert!(Timestamp::parse("1970-01-01T00:00:00.12aZ").is_err());
    }

    #[test]
    fn decimal_display() {
        assert_eq!(Decimal::new(12345, 2).to_string(), "123.45");
        assert_eq!(Decimal::new(-12345, 2).to_string(), "-123.45");
        assert_eq!(Decimal::new(5, 3).to_string(), "0.005");
        assert_eq!(Decimal::new(0, 0).to_string(), "0");
        assert_eq!(Decimal::new(-5, 0).to_string(), "-5");
        assert_eq!(
            Decimal::new(i128::MAX, 0).to_string(),
            i128::MAX.to_string()
        );
    }

    #[test]
    fn date_rejects_impossible_days() {
        assert!(Date::from_ymd(2024, 2, 29).is_some());
        assert!(Date::from_ymd(2023, 2, 29).is_none());
        assert!(Date::from_ymd(2024, 4, 31).is_none());
        assert!(Date::from_ymd(2024, 13, 1).is_none());
    }

    #[test]
    fn date_roundtrips_civil_conversion() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (1969, 12, 31), (9999, 12, 31)] {
            let date = Date::from_ymd(y, m, d).unwrap();
            assert_eq!(date.ymd(), (y, m, d));
            assert_eq!(Date::parse(&date.to_string()).unwrap(), date);
        }
        assert_eq!(Date::from_ymd(1970, 1, 1).unwrap().days_since_epoch(), 0);
    }

    #[test]
    fn timestamp_parses_and_formats() {
        let ts = Timestamp::parse("2024-01-15T10:30:00Z").unwrap();
        assert_eq!(ts.to_string(), "2024-01-15T10:30:00Z");
        let ts = Timestamp::parse("2024-01-15T10:30:00.123456Z").unwrap();
        assert_eq!(ts.to_string(), "2024-01-15T10:30:00.123456Z");
        let ts = Timestamp::parse("2024-01-15T10:30:00.5Z").unwrap();
        assert_eq!(ts.micros() % 1_000_000, 500_000);
        assert!(Timestamp::parse("2024-01-15T10:30:00").is_err());
        assert!(Timestamp::parse("2024-01-15 10:30:00Z").is_err());
    }

    #[test]
    fn non_finite_floats_survive_the_json_round_trip() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.5, -0.0] {
            let json = serde_json::to_string(&Value::Float64(value)).unwrap();
            let back = Value::deserialize_typed(
                serde_json::from_str::<serde_json::Value>(&json).unwrap(),
                &DataType::Float64,
            )
            .unwrap();
            match back {
                Value::Float64(back) => assert!(
                    back == value || (back.is_nan() && value.is_nan()),
                    "{json} came back as {back}"
                ),
                other => panic!("{json} came back as {other:?}"),
            }
        }
        // A NaN must not quietly become a NULL: that would change rule S7 into S1.
        assert_eq!(
            serde_json::to_string(&Value::Float64(f64::NAN)).unwrap(),
            "\"NaN\""
        );
    }

    #[test]
    fn only_the_three_non_finite_spellings_are_accepted_for_floats() {
        for (text, expected) in [("NaN", f64::NAN), ("Infinity", f64::INFINITY)] {
            let value = Value::deserialize_typed(
                serde_json::Value::String(text.to_owned()),
                &DataType::Float64,
            )
            .unwrap();
            match value {
                Value::Float64(value) => assert!(
                    value == expected || (value.is_nan() && expected.is_nan()),
                    "{text}"
                ),
                other => panic!("{text} came back as {other:?}"),
            }
        }
        assert!(
            Value::deserialize_typed(
                serde_json::Value::String("nan".to_owned()),
                &DataType::Float64
            )
            .is_err()
        );
    }
}
