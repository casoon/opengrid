//! The calendar, the decimal and the timestamp text, pinned down from outside: mutation testing
//! found the leap-year rule, the day arithmetic, the digit count, the rescale and the field limits
//! unchecked (they were only exercised through round trips that one broken step can cancel out).

use opengrid_json::Json;
use opengrid_types::{DataType, Date, Decimal, Timestamp, Value};

/// A calendar written the slow way: day by day, each date the day after the one before, and each
/// day count read back as the date it came from.
fn walk(first_year: i32, last_year: i32, first_day: i32) {
    let leap = |y: i32| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let (mut year, mut month, mut day) = (first_year, 1u32, 1u32);
    let mut days = first_day;
    while year < last_year {
        let date = Date::from_ymd(year, month, day).unwrap();
        assert_eq!(date.days_since_epoch(), days, "{year}-{month}-{day}");
        assert_eq!(date.ymd(), (year, month, day), "day {days}");
        assert_eq!(Date::from_days_since_epoch(days).ymd(), (year, month, day));
        let length = match month {
            2 if leap(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        // One past the end of the month is not a date.
        assert!(
            Date::from_ymd(year, month, length + 1).is_none(),
            "{year}-{month}"
        );
        days += 1;
        if day < length {
            day += 1;
        } else {
            day = 1;
            if month < 12 {
                month += 1;
            } else {
                month = 1;
                year += 1;
            }
        }
    }
}

#[test]
fn every_day_is_the_next_one_across_four_centuries() {
    assert_eq!(Date::from_ymd(1970, 1, 1).unwrap().days_since_epoch(), 0);
    assert_eq!(
        Date::from_ymd(1600, 1, 1).unwrap().days_since_epoch(),
        -135_140
    );
    walk(1600, 2400, -135_140);
}

/// Before the year 0 the calendar takes its other branch: 0000-03-01 is day -719468, and the
/// walk crosses from the negative years into the positive ones.
#[test]
fn the_calendar_holds_before_the_year_zero() {
    assert_eq!(
        Date::from_ymd(0, 3, 1).unwrap().days_since_epoch(),
        -719_468
    );
    let first = Date::from_ymd(-450, 1, 1).unwrap().days_since_epoch();
    walk(-450, 50, first);
}

#[test]
fn the_leap_rule_and_the_edges_of_a_date() {
    for (year, leap) in [
        (2024, true),
        (2023, false),
        (1900, false),
        (2000, true),
        (2100, false),
    ] {
        assert_eq!(Date::from_ymd(year, 2, 29).is_some(), leap, "{year}");
    }
    assert!(Date::from_ymd(2026, 0, 1).is_none());
    assert!(Date::from_ymd(2026, 13, 1).is_none());
    assert!(Date::from_ymd(2026, 1, 0).is_none());
    assert!(Date::from_ymd(2026, 12, 31).is_some());
    for bad in [
        "2026-1-05",
        "2026-01-5",
        "2026/01/05",
        "2026-01/05",
        "2026/01-05",
        "abcd-01-05",
        "2026-ab-05",
        "2026-01-ab",
        "2026-01-050",
        "026-01-05",
        "",
        "2026-02-30",
        "2026-13-01",
    ] {
        assert!(Date::parse(bad).is_err(), "{bad:?}");
    }
    assert_eq!(Date::parse("2026-02-28").unwrap().ymd(), (2026, 2, 28));
    assert_eq!(Date::parse("0001-01-01").unwrap().ymd(), (1, 1, 1));
}

#[test]
fn the_timestamp_text_reads_and_writes_every_field() {
    let at = Timestamp::from_ymd_hms(2026, 3, 5, 12, 34, 56, 789_012).unwrap();
    assert_eq!(at.to_string(), "2026-03-05T12:34:56.789012Z");
    assert_eq!(Timestamp::parse("2026-03-05T12:34:56.789012Z").unwrap(), at);
    assert_eq!(
        Timestamp::from_ymd_hms(2026, 3, 5, 0, 0, 0, 0)
            .unwrap()
            .to_string(),
        "2026-03-05T00:00:00Z"
    );
    // Before the epoch the day is the calendar day before it, and the time of day counts up.
    let before = Timestamp::from_micros(-1);
    assert_eq!(before.to_string(), "1969-12-31T23:59:59.999999Z");
    assert_eq!(Timestamp::parse(&before.to_string()).unwrap(), before);
    assert_eq!(before.date().ymd(), (1969, 12, 31));
    // Every field's last value, and the first one past it, through the by-position reading and
    // the general one (a fraction of other than six digits, a one-digit field).
    for ok in [
        "2026-03-05T23:59:59Z",
        "2026-03-05T23:59:59.999999Z",
        "2026-03-05T00:00:00.5Z",
        "2026-03-05T1:2:3Z",
        "2026-03-05T12:00:00.1Z",
        "2026-03-05T12:00:00.123456Z",
    ] {
        assert!(Timestamp::parse(ok).is_ok(), "{ok}");
    }
    for bad in [
        "2026-03-05T24:00:00Z",
        "2026-03-05T12:60:00Z",
        "2026-03-05T12:00:60Z",
        "2026-03-05T24:0:0Z",
        "2026-03-05T1:60:0Z",
        "2026-03-05T1:0:60Z",
        "2026-03-05T12:00:00.1234567Z",
        "2026-03-05T12:00:00.12a4Z",
        "2026-03-05T12:00:00:00Z",
        "2026-03-05T12:00Z",
        "2026-03-05T12::00Z",
        "2026-03-05T012:00:00Z",
        "2026-03-05 12:00:00Z",
        "2026-03-05T12:00:00",
        "2026-03-05T12:00:00+01:00",
        "T12:00:00Z",
        "2026-03-05T:00:00Z",
    ] {
        assert!(Timestamp::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(
        Timestamp::from_ymd_hms(2026, 3, 5, 23, 59, 59, 999_999)
            .unwrap()
            .micros()
            % 1_000_000,
        999_999
    );
    for (h, m, s, us) in [
        (24, 0, 0, 0),
        (0, 60, 0, 0),
        (0, 0, 60, 0),
        (0, 0, 0, 1_000_000),
    ] {
        assert!(
            Timestamp::from_ymd_hms(2026, 3, 5, h, m, s, us).is_none(),
            "{h}:{m}:{s}.{us}"
        );
    }
    // The fields add up: one second, one minute, one hour, one microsecond.
    let base = Timestamp::from_ymd_hms(2026, 3, 5, 0, 0, 0, 0)
        .unwrap()
        .micros();
    for (h, m, s, us, add) in [
        (1, 0, 0, 0, 3_600_000_000i64),
        (0, 1, 0, 0, 60_000_000),
        (0, 0, 1, 0, 1_000_000),
        (0, 0, 0, 1, 1),
    ] {
        assert_eq!(
            Timestamp::from_ymd_hms(2026, 3, 5, h, m, s, us)
                .unwrap()
                .micros(),
            base + add
        );
    }
}

#[test]
fn a_decimal_counts_its_digits_and_keeps_its_scale() {
    for (value, digits) in [
        (0i128, 1),
        (9, 1),
        (10, 2),
        (99, 2),
        (100, 3),
        (-100, 3),
        (999_999_999_999_999_999, 18),
        (1_000_000_000_000_000_000, 19),
        (i128::MAX, 39),
    ] {
        assert_eq!(Decimal::new(value, 2).digit_count(), digits, "{value}");
    }
    assert_eq!(Decimal::new(5, 0).scale(), 0);
    assert_eq!(Decimal::new(5, 7).scale(), 7);
}

#[test]
fn a_decimal_is_rescaled_without_losing_a_digit_and_fits_its_precision_exactly() {
    let decimal = |text: &str, precision, scale| {
        Value::from_wire_str(text, &DataType::decimal(precision, scale).unwrap())
    };
    // Up: the coefficient grows. Down: only zeros go.
    assert_eq!(
        decimal("1.5", 12, 3),
        Ok(Value::Decimal(Decimal::new(1500, 3)))
    );
    assert_eq!(
        decimal("1.500", 12, 1),
        Ok(Value::Decimal(Decimal::new(15, 1)))
    );
    assert_eq!(
        decimal("-1.500", 12, 1),
        Ok(Value::Decimal(Decimal::new(-15, 1)))
    );
    assert!(decimal("1.55", 12, 1).is_err());
    assert!(decimal("-1.55", 12, 1).is_err());
    assert_eq!(
        decimal("150", 12, 0),
        Ok(Value::Decimal(Decimal::new(150, 0)))
    );
    // The precision: three digits fit, four do not, at either sign.
    assert!(decimal("999", 3, 0).is_ok());
    assert!(decimal("-999", 3, 0).is_ok());
    assert!(decimal("1000", 3, 0).is_err());
    assert!(decimal("-1000", 3, 0).is_err());
    assert!(decimal("9.99", 3, 2).is_ok());
    assert!(decimal("10.00", 3, 2).is_err());
    // Up to 38 digits, the widest a decimal is.
    assert!(decimal(&"9".repeat(38), 38, 0).is_ok());
    assert!(decimal(&format!("1{}", "0".repeat(38)), 38, 0).is_err());
    assert!(decimal("0", 1, 0).is_ok());
}

#[test]
fn a_json_value_of_the_wrong_shape_names_the_shape_it_was() {
    let float = DataType::Float64;
    let message = |json: Json, data_type: DataType| {
        Value::from_json_typed(&json, &data_type)
            .unwrap_err()
            .to_string()
    };
    assert!(message(Json::Bool(true), float).ends_with("found JSON bool"));
    assert!(message(Json::from(1.5), DataType::Int64).ends_with("found JSON number"));
    assert!(message(Json::from(1), DataType::Bool).ends_with("found JSON integer"));
    assert!(message(Json::from("x"), DataType::Bool).ends_with("found JSON string"));
}
