//! The JSON form of `Schema` (plan point 23).
//!
//! The canonical example is the conformance data set's schema file: it has
//! carried this shape since point 05, and until now a hand-written reader in
//! `opengrid-conformance` was the only thing that could read it. These tests pin
//! the form against that very file, so the two cannot drift apart.

use opengrid_types::{DataType, Field, FieldName, Schema};

/// The schema file of the conformance data set, read through `serde` alone.
#[test]
fn the_conformance_schema_file_reads_back() {
    let json = include_str!("../../opengrid-conformance/data/orders.schema.json");
    let schema: Schema = serde_json::from_str(json).expect("the canonical schema file parses");

    assert_eq!(schema.len(), 10);
    let id = schema.field("id").expect("id");
    assert_eq!(id.data_type, DataType::Int64);
    assert!(!id.nullable, "id is the one required column");
    assert_eq!(
        schema.field("amount").expect("amount").data_type,
        DataType::Decimal {
            precision: 12,
            scale: 2
        }
    );
    assert!(schema.field("note").expect("note").nullable);
}

/// Every type survives a round trip, in schema order.
#[test]
fn a_schema_survives_a_round_trip() {
    let field =
        |name: &str, data_type: DataType| Field::new(FieldName::new(name).unwrap(), data_type);
    let schema = Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        field("flag", DataType::Bool),
        field("ratio", DataType::Float64),
        field("amount", DataType::decimal(38, 10).unwrap()),
        field("note", DataType::Utf8),
        field("ordered_on", DataType::Date),
        field("created_at", DataType::Timestamp),
    ]);

    let json = serde_json::to_string(&schema).unwrap();
    assert_eq!(serde_json::from_str::<Schema>(&json).unwrap(), schema);
    // Field order is projection order and must not be reordered by the round trip.
    let read: Schema = serde_json::from_str(&json).unwrap();
    let names: Vec<&str> = read.fields().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "id",
            "flag",
            "ratio",
            "amount",
            "note",
            "ordered_on",
            "created_at"
        ]
    );
}

/// The reader is strict: unknown keys and invalid identifiers are errors, not
/// silently dropped fields.
#[test]
fn a_malformed_schema_is_rejected() {
    for json in [
        r#"{"fields":[{"name":"id","type":"int64","nullable":false,"extra":1}]}"#,
        r#"{"fields":[{"name":"id","type":"int64"}],"extra":1}"#,
        r#"{"fields":[{"name":"","type":"int64"}]}"#,
        r#"{"fields":[{"name":"id","type":"int65"}]}"#,
    ] {
        assert!(serde_json::from_str::<Schema>(json).is_err(), "{json}");
    }
}
