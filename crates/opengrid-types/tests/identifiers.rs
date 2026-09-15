//! Identifier validation (`^[A-Za-z_][A-Za-z0-9_]{0,62}$`) and its serde behaviour.

use opengrid_types::{DataSourceId, FieldName};

#[test]
fn invalid_field_names_are_rejected() {
    for bad in ["a b", "1x", "", "x;drop"] {
        assert!(FieldName::new(bad).is_err(), "{bad:?} must be rejected");
        let err = serde_json::from_str::<FieldName>(&format!("\"{bad}\""));
        assert!(err.is_err(), "{bad:?} must not deserialize");
    }
}

#[test]
fn identifier_length_boundary_is_63() {
    let ok = "a".repeat(63);
    assert!(FieldName::new(ok).is_ok());
    let too_long = "a".repeat(64);
    assert!(FieldName::new(too_long).is_err());
}

#[test]
fn valid_identifiers_roundtrip_through_serde() {
    let name = FieldName::new("amount_1").unwrap();
    let json = serde_json::to_string(&name).unwrap();
    assert_eq!(json, "\"amount_1\"");
    assert_eq!(serde_json::from_str::<FieldName>(&json).unwrap(), name);

    let source = DataSourceId::new("_orders").unwrap();
    assert_eq!(source.as_str(), "_orders");
    let json = serde_json::to_string(&source).unwrap();
    assert_eq!(serde_json::from_str::<DataSourceId>(&json).unwrap(), source);
}
