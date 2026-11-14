//! Results read back into tables (`load_result`, E14) and tables built from pieces
//! (`PieceBuilder`): found unchecked by mutation testing, which could replace both with an empty
//! table and see no test fail.

use opengrid_datasource::QueryResult;
use opengrid_engine::ingest::{IngestError, PieceBuilder, load_result};
use opengrid_types::{DataType, Date, DatePart, Derivation, Field, FieldName, Schema, Value};

fn name(text: &str) -> FieldName {
    FieldName::new(text).unwrap()
}

fn stored() -> Schema {
    Schema::new(vec![
        Field::required(name("id"), DataType::Int64),
        Field::new(name("born"), DataType::Date),
    ])
}

/// The stored columns and a year derived from the date.
fn with_year() -> Schema {
    let mut fields = stored().fields().to_vec();
    fields.push(Field {
        from: Some(Derivation {
            part: DatePart::Year,
            field: name("born"),
        }),
        ..Field::new(name("born_year"), DataType::Int64)
    });
    Schema::new(fields)
}

fn day(year: i32) -> Value {
    Value::Date(Date::from_ymd(year, 3, 5).unwrap())
}

fn piece(ids: &[i64], years: &[Option<i32>]) -> QueryResult {
    QueryResult {
        schema: stored(),
        columns: vec![
            ids.iter().map(|id| Value::Int64(*id)).collect(),
            years.iter().map(|y| y.map_or(Value::Null, day)).collect(),
        ],
        total_count: ids.len() as u64,
        tree: None,
    }
}

#[test]
fn a_result_is_read_back_value_for_value() {
    let result = piece(&[1, 2, 3], &[Some(1990), None, Some(2001)]);
    let table = load_result(&result).unwrap();
    assert_eq!(table.num_rows(), 3);
    for (column, values) in result.columns.iter().enumerate() {
        for (row, value) in values.iter().enumerate() {
            assert_eq!(
                &table.column_at(column).value(row),
                value,
                "column {column}, row {row}"
            );
        }
    }
}

#[test]
fn a_broken_result_says_what_is_broken() {
    let mut result = piece(&[1, 2], &[Some(1990), None]);
    result.columns.pop();
    let error = load_result(&result).unwrap_err();
    assert!(
        matches!(&error, IngestError::Schema { message } if message.contains("1 columns for 2 fields")),
        "{error}"
    );

    let mut result = piece(&[1, 2], &[Some(1990), None]);
    result.columns[1].pop();
    let error = load_result(&result).unwrap_err();
    assert!(
        matches!(&error, IngestError::Schema { message } if message.contains("column born has 1 values, the first has 2")),
        "{error}"
    );

    // A value that is not of its column's type is refused, not stored.
    let mut result = piece(&[1], &[Some(1990)]);
    result.columns[0][0] = Value::Utf8("x".to_owned());
    assert!(load_result(&result).is_err());
}

#[test]
fn pieces_make_one_table_with_the_derived_columns() {
    let schema = with_year();
    let mut builder = PieceBuilder::new(&schema);
    assert_eq!(builder.rows(), 0);
    builder.push(&piece(&[1, 2], &[Some(1990), None])).unwrap();
    assert_eq!(builder.rows(), 2);
    builder.push(&piece(&[3], &[Some(2001)])).unwrap();
    assert_eq!(builder.rows(), 3);
    let table = builder.finish().unwrap();
    assert_eq!(table.num_rows(), 3);
    let ids: Vec<Value> = (0..3).map(|row| table.column_at(0).value(row)).collect();
    assert_eq!(ids, [Value::Int64(1), Value::Int64(2), Value::Int64(3)]);
    let years: Vec<Value> = (0..3).map(|row| table.column_at(2).value(row)).collect();
    assert_eq!(years, [Value::Int64(1990), Value::Null, Value::Int64(2001)]);
}

#[test]
fn a_piece_with_other_columns_or_ragged_columns_is_refused_and_adds_nothing() {
    let schema = with_year();
    let mut builder = PieceBuilder::new(&schema);
    builder.push(&piece(&[1], &[Some(1990)])).unwrap();

    let mut other = piece(&[2], &[Some(1991)]);
    other.schema = Schema::new(vec![
        Field::required(name("id"), DataType::Int64),
        Field::new(name("born"), DataType::Timestamp),
    ]);
    let error = builder.push(&other).unwrap_err();
    assert!(
        matches!(&error, IngestError::Schema { message } if message.contains("a piece has the columns")),
        "{error}"
    );

    let mut ragged = piece(&[2, 3], &[Some(1991), Some(1992)]);
    ragged.columns[1].pop();
    let error = builder.push(&ragged).unwrap_err();
    assert!(
        matches!(&error, IngestError::Schema { message } if message.contains("different lengths")),
        "{error}"
    );

    assert_eq!(builder.rows(), 1);
    assert_eq!(builder.finish().unwrap().num_rows(), 1);
}
