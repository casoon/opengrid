//! JSON representation and typed round-trip for every [`Value`] variant,
//! including the boundaries of the `i128`-backed decimal.

use opengrid_types::{DataType, Date, Decimal, Timestamp, Value};

/// Serializes a value to its JSON representation and reads it back using the
/// given column type.
fn roundtrip(value: &Value, data_type: DataType) -> Value {
    let json = serde_json::to_string(value).expect("serialize");
    let mut deserializer = serde_json::Deserializer::from_str(&json);
    Value::deserialize_typed(&mut deserializer, &data_type).expect("deserialize")
}

fn assert_roundtrip(value: Value, data_type: DataType) {
    let back = roundtrip(&value, data_type);
    assert_eq!(back, value, "round-trip changed the value");
}

#[test]
fn null_roundtrips_for_every_type() {
    for ty in [
        DataType::Bool,
        DataType::Int64,
        DataType::Float64,
        DataType::decimal(10, 2).unwrap(),
        DataType::Utf8,
        DataType::Date,
        DataType::Timestamp,
    ] {
        assert_roundtrip(Value::Null, ty);
    }
}

#[test]
fn scalars_roundtrip() {
    assert_roundtrip(Value::Bool(true), DataType::Bool);
    assert_roundtrip(Value::Bool(false), DataType::Bool);
    assert_roundtrip(Value::Int64(0), DataType::Int64);
    assert_roundtrip(Value::Int64(i64::MIN), DataType::Int64);
    assert_roundtrip(Value::Int64(i64::MAX), DataType::Int64);
    assert_roundtrip(Value::Float64(1.5), DataType::Float64);
    assert_roundtrip(Value::Float64(f64::MAX), DataType::Float64);
    assert_roundtrip(Value::Float64(f64::MIN_POSITIVE), DataType::Float64);
    assert_roundtrip(Value::Utf8(String::new()), DataType::Utf8);
    assert_roundtrip(Value::Utf8("hello".to_owned()), DataType::Utf8);
}

#[test]
fn negative_zero_roundtrips_as_number() {
    let back = roundtrip(&Value::Float64(-0.0), DataType::Float64);
    match back {
        Value::Float64(f) => assert_eq!(f, 0.0),
        other => panic!("expected Float64, got {other:?}"),
    }
}

#[test]
fn decimal_boundaries_roundtrip() {
    // Largest value the 38-digit precision cap allows.
    let max = Decimal::new(10i128.pow(38) - 1, 0);
    assert_roundtrip(Value::Decimal(max), DataType::decimal(38, 0).unwrap());
    assert_roundtrip(
        Value::Decimal(Decimal::new(-(10i128.pow(38) - 1), 0)),
        DataType::decimal(38, 0).unwrap(),
    );
    // A scale equal to the precision, i.e. a pure fraction.
    assert_roundtrip(
        Value::Decimal(Decimal::new(0, 38)),
        DataType::decimal(38, 38).unwrap(),
    );
    assert_roundtrip(
        Value::Decimal(Decimal::new(12345, 2)),
        DataType::decimal(10, 2).unwrap(),
    );
    assert_roundtrip(
        Value::Decimal(Decimal::new(5, 3)),
        DataType::decimal(10, 3).unwrap(),
    );
}

#[test]
fn decimal_is_serialized_as_string() {
    let json = serde_json::to_string(&Value::Decimal(Decimal::new(12345, 2))).unwrap();
    assert_eq!(json, "\"123.45\"");
}

#[test]
fn decimal_is_rescaled_to_the_column_scale() {
    let mut deserializer = serde_json::Deserializer::from_str("\"123.450\"");
    let value = Value::deserialize_typed(&mut deserializer, &DataType::decimal(6, 2).unwrap())
        .expect("rescale down");
    assert_eq!(value, Value::Decimal(Decimal::new(12345, 2)));

    let mut deserializer = serde_json::Deserializer::from_str("\"5\"");
    let value = Value::deserialize_typed(&mut deserializer, &DataType::decimal(6, 2).unwrap())
        .expect("rescale up");
    assert_eq!(value, Value::Decimal(Decimal::new(500, 2)));
}

#[test]
fn decimal_rejects_lossy_rescale_and_precision_overflow() {
    let mut lossy = serde_json::Deserializer::from_str("\"123.456\"");
    assert!(Value::deserialize_typed(&mut lossy, &DataType::decimal(6, 2).unwrap()).is_err());

    // 39 digits exceed the 38-digit cap.
    let mut too_big =
        serde_json::Deserializer::from_str("\"999999999999999999999999999999999999999\"");
    assert!(Value::deserialize_typed(&mut too_big, &DataType::decimal(38, 0).unwrap()).is_err());
}

#[test]
fn dates_and_timestamps_roundtrip() {
    assert_roundtrip(
        Value::Date(Date::from_ymd(2024, 1, 15).unwrap()),
        DataType::Date,
    );
    assert_roundtrip(
        Value::Date(Date::from_ymd(1969, 12, 31).unwrap()),
        DataType::Date,
    );
    assert_roundtrip(
        Value::Timestamp(Timestamp::parse("2024-01-15T10:30:00Z").unwrap()),
        DataType::Timestamp,
    );
    assert_roundtrip(
        Value::Timestamp(Timestamp::parse("2024-01-15T10:30:00.123456Z").unwrap()),
        DataType::Timestamp,
    );
}

#[test]
fn date_like_string_stays_utf8_when_typed_as_utf8() {
    // The typed contract must not "sniff" a date out of a string column.
    let value = Value::Utf8("2024-01-15".to_owned());
    assert_roundtrip(value, DataType::Utf8);
}

#[test]
fn type_mismatches_are_rejected() {
    for (json, ty) in [
        ("true", DataType::Int64),
        ("1", DataType::Bool),
        ("\"x\"", DataType::Int64),
        ("1", DataType::Utf8),
        ("123.45", DataType::decimal(10, 2).unwrap()),
        ("\"not-a-date\"", DataType::Date),
        ("\"2024-01-15\"", DataType::Timestamp),
    ] {
        let mut deserializer = serde_json::Deserializer::from_str(json);
        assert!(
            Value::deserialize_typed(&mut deserializer, &ty).is_err(),
            "{json} should not be accepted as {ty}"
        );
    }
}
