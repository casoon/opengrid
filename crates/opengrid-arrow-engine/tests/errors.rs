//! Bad input has to come back as a readable error, not as a panic: ingest is the
//! path untrusted bytes take. The line number is part of the contract.

use opengrid_arrow_engine::ingest::{CsvOptions, IngestError, JsonOptions, load_csv, load_json};
use opengrid_types::{DataType, Field, FieldName, Schema};

/// `id` is non-nullable, `note` is not: enough to exercise both directions.
fn schema() -> Schema {
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("note").unwrap(), DataType::Utf8),
    ])
}

fn csv(bytes: &str) -> Result<Vec<arrow_array::RecordBatch>, IngestError> {
    load_csv(bytes.as_bytes(), &schema(), CsvOptions::default())
}

fn json(bytes: &str) -> Result<Vec<arrow_array::RecordBatch>, IngestError> {
    load_json(bytes.as_bytes(), &schema(), JsonOptions::default())
}

/// The closure of point 06: a broken record is reported with its line.
#[test]
fn a_broken_value_reports_its_line() {
    let error = csv("id,note\n1,a\n2,b\n3,c\nnope,d\n").unwrap_err();
    assert_eq!(
        error.to_string(),
        "CSV line 5, column id: \"nope\" is not an int64"
    );
    assert_eq!(
        error,
        IngestError::Row {
            line: 5,
            column: "id".to_owned(),
            message: "\"nope\" is not an int64".to_owned()
        }
    );
}

#[test]
fn a_quoted_field_over_several_lines_does_not_shift_the_numbering() {
    // This record starts on line 2 and spans lines 2 and 3, so its error is
    // reported against line 2.
    let error = csv("id,note\nnope,\"a\nb\"\n").unwrap_err();
    assert_eq!(
        error.to_string(),
        "CSV line 2, column id: \"nope\" is not an int64"
    );

    // The records after it keep counting physical lines: the last one is the
    // fifth line of the file, although it is only the fourth record.
    let error = csv("id,note\n1,\"a\nb\"\n2,c\nnope,d\n").unwrap_err();
    assert_eq!(
        error.to_string(),
        "CSV line 5, column id: \"nope\" is not an int64"
    );
}

#[test]
fn a_record_with_the_wrong_number_of_fields_reports_its_line() {
    let error = csv("id,note\n1,a\n2,b,c\n").unwrap_err();
    assert_eq!(error.to_string(), "CSV line 3: expected 2 fields, found 3");

    let error = csv("id,note\n1,a\n2\n").unwrap_err();
    assert_eq!(error.to_string(), "CSV line 3: expected 2 fields, found 1");
}

#[test]
fn a_never_closed_quote_reports_where_it_opened() {
    let error = csv("id,note\n1,a\n2,\"open\n").unwrap_err();
    assert_eq!(error.to_string(), "CSV line 3: unterminated quoted field");
}

#[test]
fn null_in_a_non_nullable_column_is_an_error() {
    assert_eq!(
        csv("id,note\n\\N,a\n").unwrap_err().to_string(),
        "CSV line 2, column id: NULL is not allowed in a non-nullable column"
    );
    assert_eq!(
        json(r#"[{"id": null, "note": "a"}]"#)
            .unwrap_err()
            .to_string(),
        "JSON row 0, field id: NULL is not allowed in a non-nullable column"
    );
    // … but an empty string is a value, also in that column's neighbour.
    assert!(json(r#"[{"id": 1, "note": ""}]"#).is_ok());
}

#[test]
fn the_header_has_to_name_the_columns() {
    assert_eq!(
        csv("note,id\n").unwrap_err().to_string(),
        "CSV line 1: header does not match the schema: expected [id, note], found [note, id]"
    );
    // A different order is not "close enough": the columns would be swapped.
    assert!(
        csv("id\n")
            .unwrap_err()
            .to_string()
            .starts_with("CSV line 1")
    );
}

#[test]
fn json_reports_the_row_and_the_field() {
    assert_eq!(
        json(r#"[{"id": 1}, {"id": "nope"}]"#)
            .unwrap_err()
            .to_string(),
        "JSON row 1, field id: expected int64, found JSON string"
    );
    assert_eq!(
        json(r#"[{"id": 1, "extra": true}]"#)
            .unwrap_err()
            .to_string(),
        "JSON row 0, field extra: unknown field"
    );
    assert_eq!(
        json(r#"[{"id": 1}, 5]"#).unwrap_err().to_string(),
        "JSON row 1: expected an object"
    );
    assert_eq!(
        json(r#"{"rows": []}"#).unwrap_err().to_string(),
        "input: expected a JSON array of objects"
    );
    assert!(
        json("[{\"id\": }]")
            .unwrap_err()
            .to_string()
            .starts_with("input: input is not JSON")
    );
}

#[test]
fn input_that_is_not_text_is_rejected() {
    let error = load_csv(&[0xff, 0xfe, b'\n'], &schema(), CsvOptions::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "input: input is not UTF-8 (invalid byte at offset 0)"
    );
}

#[test]
fn an_unusable_delimiter_is_rejected() {
    let options = CsvOptions {
        delimiter: b'"',
        ..CsvOptions::default()
    };
    let error = load_csv(b"id\n", &schema(), options).unwrap_err();
    assert!(error.to_string().starts_with("input: delimiter 0x22"));

    let options = CsvOptions {
        delimiter: 0xe4,
        ..CsvOptions::default()
    };
    assert!(load_csv(b"id\n", &schema(), options).is_err());
}

#[test]
fn a_usable_delimiter_can_be_chosen() {
    let options = CsvOptions {
        delimiter: b';',
        ..CsvOptions::default()
    };
    let batches = load_csv(b"id;note\n1;a\n", &schema(), options).unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
}

#[test]
fn a_bom_is_not_part_of_the_first_column_name() {
    let batches = csv("\u{feff}id,note\n1,a\n").unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        1
    );
}

#[test]
fn without_a_header_the_records_start_at_line_one() {
    let no_header = CsvOptions {
        has_header: false,
        ..CsvOptions::default()
    };
    let batches = load_csv(b"1,a\n2,b\n", &schema(), no_header.clone()).unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        2
    );

    let error = load_csv(b"1,a\nnope,b\n", &schema(), no_header).unwrap_err();
    assert_eq!(
        error.to_string(),
        "CSV line 2, column id: \"nope\" is not an int64"
    );
}

#[test]
fn input_without_records_yields_no_batch() {
    assert!(csv("").unwrap().is_empty());
    assert!(csv("id,note\n").unwrap().is_empty());
    assert!(json("[]").unwrap().is_empty());
}
