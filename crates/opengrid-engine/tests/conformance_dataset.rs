//! The point-06 closure: the same data as CSV and as JSON has to reach the
//! engine as the *same* table, and no rule may be lost on the way.
//!
//! `orders.json` is the JSON twin of `orders.csv`, generated (and checked) by
//! `crates/opengrid-conformance/data/gen-orders-json.py`. This test is the
//! authority: edit the CSV, regenerate the JSON, and it has to stay green.

mod common;

use opengrid_conformance::values_equal;
use opengrid_engine::ingest::{CsvOptions, load_csv, load_json};
use opengrid_types::{DataType, Decimal, Timestamp, Value};

/// The core of point 06: encoding must not change the data.
#[test]
fn csv_and_json_of_the_same_data_reach_the_same_table() {
    let from_csv = common::csv_table();
    let from_json = common::json_table();

    let csv_rows = common::decode(&from_csv);
    let json_rows = common::decode(&from_json);
    assert_eq!(csv_rows.len(), 50);
    assert_eq!(json_rows.len(), 50);
    assert_eq!(from_csv.schema(), from_json.schema());
    assert_eq!(from_csv.num_rows(), from_json.num_rows());

    for (index, (left, right)) in csv_rows.iter().zip(&json_rows).enumerate() {
        assert_eq!(left.len(), right.len(), "row {index}");
        for (column, (left, right)) in left.iter().zip(right).enumerate() {
            assert!(
                values_equal(left, right),
                "row {index}, column {column}: {left:?} != {right:?}"
            );
        }
    }
}

/// The cells the semantics rules are written for.
#[test]
fn the_awkward_cells_survive_ingest() {
    let table = common::csv_table();
    let rows = common::decode(&table);
    let row = |id: i64| common::row(&rows, id).clone();

    // S7: a NaN is a value, and it does not sort with the numbers by accident —
    // while the NULL next to it is a NULL.
    assert!(matches!(row(4)[5], Value::Float64(value) if value.is_nan()));
    assert_eq!(row(3)[5], Value::Float64(0.0));
    assert_eq!(row(3)[4], Value::Null);
    assert_eq!(row(47)[5], Value::Null);
    assert!(!table.column_at(5).is_null(3), "the NaN must not be a null");
    assert!(
        table.column_at(4).is_null(2),
        "the NULL stays a null (id 3 has no qty)"
    );

    // S7: -0.0 equals 0.0 but keeps its sign.
    match &row(2)[5] {
        Value::Float64(value) => {
            assert_eq!(*value, 0.0);
            assert!(value.is_sign_negative());
        }
        other => panic!("{other:?}"),
    }

    // S14: an empty cell is the empty string, not a NULL and not a zero.
    assert_eq!(row(11)[1], Value::Utf8(String::new()));
    assert_eq!(row(11)[3], Value::Decimal(Decimal::new(1111, 2)));
    assert_eq!(row(37)[2], Value::Utf8(String::new()));
    assert_eq!(row(1)[9], Value::Null);

    // S8: decimals are exact, also at the edges of the type.
    assert_eq!(row(7)[3], Value::Decimal(Decimal::new(99_999_999_999, 2)));
    assert_eq!(row(8)[3], Value::Decimal(Decimal::new(-1, 2)));
    assert_eq!(row(9)[3], Value::Decimal(Decimal::new(-1_234_567, 2)));
    assert_eq!(row(50)[3], Value::Decimal(Decimal::new(5050, 2)));

    // S9: timestamps are UTC instants, to the microsecond.
    assert_eq!(
        row(6)[8],
        Value::Timestamp(Timestamp::parse("2026-03-01T00:00:00.500000Z").unwrap())
    );
    assert_eq!(
        row(50)[8],
        Value::Timestamp(Timestamp::parse("2026-12-31T23:59:59.999999Z").unwrap())
    );
    assert_eq!(
        row(1)[8],
        Value::Timestamp(Timestamp::parse("2025-12-31T23:59:59Z").unwrap())
    );

    // S13: two spellings of the same letter stay two different values.
    assert_eq!(row(45)[9], Value::Utf8("\u{e9}".to_owned()));
    assert_eq!(row(46)[9], Value::Utf8("e\u{301}".to_owned()));
    assert_ne!(row(45)[9], row(46)[9]);

    // S12: the declared types travel with the table, down to the columns.
    let fields = table.schema().fields();
    assert_eq!(
        fields[3].data_type,
        DataType::Decimal {
            precision: 12,
            scale: 2
        }
    );
    assert_eq!(table.column_at(3).data_type(), fields[3].data_type);
    assert_eq!(fields[8].data_type, DataType::Timestamp);
    assert_eq!(table.column_at(8).data_type(), DataType::Timestamp);
    assert_eq!(fields[7].data_type, DataType::Date);
    assert!(!fields[0].nullable);
    assert!(fields[1].nullable);
    assert_eq!(common::schema().data_type("id"), Some(DataType::Int64));
    // The derivations are resolved away: once read, a derived column holds
    // values like any other.
    assert!(fields.iter().all(|field| field.from.is_none()));
    let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "id",
            "customer",
            "country",
            "amount",
            "qty",
            "ratio",
            "flag",
            "ordered_on",
            "created_at",
            "note",
            // Not in the file — computed from `ordered_on` and `created_at`
            // while reading it (plan point 54).
            "ordered_year",
            "ordered_month",
            "created_year"
        ]
    );
}

/// A derived column is computed from the file, not read out of it.
///
/// The dataset crosses a year boundary on purpose — row 1 is
/// `2025-12-31T23:59:59Z`, row 2 the microsecond after — so this also pins rule
/// S9: the part is taken in UTC, never in a local zone.
#[test]
fn derived_columns_are_computed_while_reading() {
    let table = common::csv_table();
    let schema = common::schema();
    assert_eq!(schema.stored().len(), 10, "the file has ten columns");

    let read = |name: &str, row: usize| -> Option<i64> {
        match table.column(name).expect(name).value(row) {
            Value::Int64(value) => Some(value),
            Value::Null => None,
            other => panic!("{name}: {other:?}"),
        }
    };

    assert_eq!(read("ordered_year", 0), Some(2025));
    assert_eq!(read("created_year", 0), Some(2025));
    assert_eq!(read("ordered_year", 1), Some(2026));
    assert_eq!(read("created_year", 1), Some(2026), "one microsecond later");
    assert_eq!(read("ordered_month", 0), Some(12));
    assert_eq!(read("ordered_month", 1), Some(1));

    // Row 5 (index 4) has no date at all, so it has no year either.
    assert_eq!(read("ordered_year", 4), None);
    assert_eq!(read("created_year", 4), None);
}

/// An input without records is an empty *table*, not a missing one: the schema
/// survives, so a query can still be answered against it (rule S11 needs that).
#[test]
fn an_input_without_records_is_an_empty_table() {
    let header = b"id,customer,country,amount,qty,ratio,flag,ordered_on,created_at,note\n";
    let from_csv = load_csv(header.as_slice(), &common::schema(), CsvOptions::default())
        .expect("the empty CSV loads");
    let from_json = load_json(b"[]".as_slice(), &common::schema()).expect("the empty array loads");

    assert_eq!(from_csv.num_rows(), 0);
    assert_eq!(from_json.num_rows(), 0);
    assert_eq!(from_csv.schema().len(), 13, "the columns survive");
    assert_eq!(from_csv.schema(), from_json.schema());
}

/// The schema has to fit the file, otherwise ingest silently shifts columns.
#[test]
fn a_header_that_does_not_match_the_schema_is_an_error() {
    let error = load_csv(
        b"note,id\n".as_slice(),
        &common::schema(),
        CsvOptions::default(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("CSV line 1: header does not match")
    );
}
