//! `infer_schema_csv` proposes, the caller decides. It must not claim more than
//! it can know.

use std::path::PathBuf;

use opengrid_arrow_engine::ingest::{CsvOptions, infer_schema_csv, load_csv};
use opengrid_types::{DataType, Schema};

fn data(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../opengrid-conformance/data")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn proposes_the_types_of_the_conformance_dataset() {
    let schema = infer_schema_csv(&data("orders.csv"), CsvOptions::default()).unwrap();
    let expected = [
        ("id", DataType::Int64, false),
        ("customer", DataType::Utf8, true),
        ("country", DataType::Utf8, true),
        // A text file cannot tell a decimal from a float, so it says float64 —
        // the caller confirms the schema, and the dataset schema says decimal.
        ("amount", DataType::Float64, true),
        ("qty", DataType::Int64, true),
        ("ratio", DataType::Float64, true),
        ("flag", DataType::Bool, true),
        ("ordered_on", DataType::Date, true),
        ("created_at", DataType::Timestamp, true),
        ("note", DataType::Utf8, true),
    ];
    assert_eq!(schema.len(), expected.len());
    for (index, (name, data_type, nullable)) in expected.into_iter().enumerate() {
        let field = &schema.fields()[index];
        assert_eq!(field.name.as_str(), name);
        assert_eq!(field.data_type, data_type, "{name}");
        assert_eq!(field.nullable, nullable, "{name}");
    }
}

#[test]
fn the_proposal_is_loadable() {
    let bytes = data("orders.csv");
    let proposed = infer_schema_csv(&bytes, CsvOptions::default()).unwrap();
    let batches = load_csv(&bytes, &proposed, CsvOptions::default()).unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.num_rows()).sum::<usize>(),
        50
    );
    // The proposal keeps the empty cells a value, so nothing turns into a NULL.
    // id 11 (row 10) has no customer, id 37 (row 36) has no country — both are
    // the empty string; id 40 (row 39) really has no country.
    assert!(
        !batches[0].column(1).is_null(10),
        "the empty customer is a value"
    );
    assert!(
        !batches[0].column(2).is_null(36),
        "the empty country is a value"
    );
    assert!(
        batches[0].column(2).is_null(39),
        "the \\N country is a NULL"
    );
}

#[test]
fn samples_can_be_limited() {
    let csv = "value\n1\n2\nmany\n";
    assert_eq!(
        infer_schema_csv(csv.as_bytes(), CsvOptions::default())
            .unwrap()
            .data_type("value"),
        Some(DataType::Utf8)
    );
    let options = CsvOptions {
        max_records: Some(2),
        ..CsvOptions::default()
    };
    assert_eq!(
        infer_schema_csv(csv.as_bytes(), options)
            .unwrap()
            .data_type("value"),
        Some(DataType::Int64)
    );
}

#[test]
fn a_column_of_nothing_is_text() {
    // A column of NULLs and empty cells says nothing about its type.
    let schema = infer_schema_csv(b"value\n\\N\n\n", CsvOptions::default()).unwrap();
    assert_eq!(schema.data_type("value"), Some(DataType::Utf8));
    assert!(schema.fields()[0].nullable);
}

#[test]
fn names_have_to_be_usable_as_identifiers() {
    let error = infer_schema_csv(b"not a name\n1\n", CsvOptions::default()).unwrap_err();
    assert!(error.to_string().starts_with("schema: invalid identifier"));
    assert!(error.to_string().contains("not a name"));
}

#[test]
fn inference_needs_a_header() {
    let options = CsvOptions {
        has_header: false,
        ..CsvOptions::default()
    };
    assert_eq!(
        infer_schema_csv(b"1,2\n", options).unwrap_err().to_string(),
        "schema: infer_schema_csv needs a header row to name the columns"
    );
    assert_eq!(
        infer_schema_csv(b"", CsvOptions::default())
            .unwrap_err()
            .to_string(),
        "schema: no header row to infer from"
    );
}

/// An inferred schema is a `Schema` like any other and follows the same rules:
/// the sample decides the types, a NULL makes the column nullable.
#[test]
fn the_proposal_is_an_ordinary_schema() {
    let proposed = infer_schema_csv(b"a,b\n1,x\n2,\\N\n", CsvOptions::default()).unwrap();
    let by_hand = Schema::new(vec![
        opengrid_types::Field::required(
            opengrid_types::FieldName::new("a").unwrap(),
            DataType::Int64,
        ),
        opengrid_types::Field::new(opengrid_types::FieldName::new("b").unwrap(), DataType::Utf8),
    ]);
    assert_eq!(proposed, by_hand);
}
