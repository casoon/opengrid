//! The point-06 closure: the same data as CSV and as JSON has to reach Arrow as
//! the *same* batches, and no rule may be lost on the way.
//!
//! `orders.json` is the JSON twin of `orders.csv`, generated (and checked) by
//! `crates/opengrid-conformance/data/gen-orders-json.py`. This test is the
//! authority: edit the CSV, regenerate the JSON, and it has to stay green.

mod common;

use arrow_array::RecordBatch;
use arrow_schema::{DataType as ArrowDataType, TimeUnit};
use opengrid_arrow_engine::ingest::{CsvOptions, load_csv};
use opengrid_conformance::values_equal;
use opengrid_types::{DataType, Decimal, Timestamp, Value};

/// The core of point 06: encoding must not change the data.
#[test]
fn csv_and_json_of_the_same_data_reach_the_same_batches() {
    let from_csv = common::csv_batches();
    let from_json = common::json_batches();

    let csv_rows = common::decode(&from_csv);
    let json_rows = common::decode(&from_json);
    assert_eq!(csv_rows.len(), 50);
    assert_eq!(json_rows.len(), 50);
    assert_eq!(from_csv[0].schema(), from_json[0].schema());
    assert_eq!(
        from_csv.iter().map(RecordBatch::num_rows).sum::<usize>(),
        from_json.iter().map(RecordBatch::num_rows).sum::<usize>()
    );

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
    let batches = common::csv_batches();
    let rows = common::decode(&batches);
    let row = |id: i64| common::row(&rows, id).clone();

    // S7: a NaN is a value, and it does not sort with the numbers by accident —
    // while the NULL next to it is a NULL.
    assert!(matches!(row(4)[5], Value::Float64(value) if value.is_nan()));
    assert_eq!(row(3)[5], Value::Float64(0.0));
    assert_eq!(row(3)[4], Value::Null);
    assert_eq!(row(47)[5], Value::Null);
    assert!(
        !batches[0].column(5).is_null(3),
        "the NaN must not be a null"
    );
    assert!(
        batches[0].column(4).is_null(2),
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

    // S12: the declared types travel with the batch.
    assert_eq!(
        batches[0].schema().field(3).data_type(),
        &ArrowDataType::Decimal128(12, 2)
    );
    assert_eq!(
        batches[0].schema().field(8).data_type(),
        &ArrowDataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(
        batches[0].schema().field(7).data_type(),
        &ArrowDataType::Date32
    );
    assert!(!batches[0].schema().field(0).is_nullable());
    assert!(batches[0].schema().field(1).is_nullable());
    assert_eq!(common::schema().data_type("id"), Some(DataType::Int64));
    let arrow_schema = batches[0].schema();
    let names: Vec<&str> = arrow_schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
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
            "note"
        ]
    );
}

/// Rows are cut into batches, and the order survives the cut.
#[test]
fn batches_are_cut_at_the_requested_size() {
    let batches = common::csv_batches_with(CsvOptions {
        batch_size: 7,
        ..CsvOptions::default()
    });
    let sizes: Vec<usize> = batches.iter().map(RecordBatch::num_rows).collect();
    assert_eq!(sizes, vec![7, 7, 7, 7, 7, 7, 7, 1]);

    let rows = common::decode(&batches);
    assert_eq!(rows.len(), 50);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row[0], Value::Int64(index as i64 + 1), "row {index}");
    }
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
