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
    let digits = format!("{int_part}{frac_part}");
    let magnitude: i128 = digits
        .parse()
        .map_err(|_| ValueError::InvalidDecimal(s.to_owned()))?;
    let value = if negative { -magnitude } else { magnitude };
    Ok(Decimal::new(value, scale))
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
        let micro: u32 = if frac.is_empty() {
            0
        } else {
            format!("{frac:0<6}").parse().map_err(|_| invalid())?
        };
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
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => serializer.serialize_none(),
            Value::Bool(v) => serializer.serialize_bool(*v),
            Value::Int64(v) => serializer.serialize_i64(*v),
            Value::Float64(v) => serializer.serialize_f64(*v),
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
        (Raw::Str(s), DataType::Utf8) => Ok(Value::Utf8(s)),
        (Raw::Str(s), DataType::Decimal { precision, scale }) => {
            let parsed = parse_decimal(&s)?;
            let rescaled = parsed
                .rescale(*scale)
                .ok_or(ValueError::DecimalOutOfRange {
                    precision: *precision,
                    scale: *scale,
                })?;
            if rescaled.digit_count() > u32::from(*precision) {
                return Err(ValueError::DecimalOutOfRange {
                    precision: *precision,
                    scale: *scale,
                });
            }
            Ok(Value::Decimal(rescaled))
        }
        (Raw::Str(s), DataType::Date) => Ok(Value::Date(Date::parse(&s)?)),
        (Raw::Str(s), DataType::Timestamp) => Ok(Value::Timestamp(Timestamp::parse(&s)?)),
        (other, _) => Err(mismatch(other.kind())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
