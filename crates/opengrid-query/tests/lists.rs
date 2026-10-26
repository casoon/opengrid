//! The list operators and what a list is not asked (issue #185).

use opengrid_query::{Limits, Query, QueryError};
use opengrid_types::{DataType, Field, FieldName, Schema};

fn schema() -> Schema {
    Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("tags").unwrap(), DataType::ListUtf8),
        Field::new(FieldName::new("rooms").unwrap(), DataType::ListInt64),
    ])
}

fn validate(json: &str) -> Result<opengrid_query::ValidatedQuery, QueryError> {
    let query: Query = opengrid_json::from_str(json).expect("a query");
    query.validate(&schema(), &Limits::default())
}

fn filter(op: &str, field: &str, value: &str) -> String {
    format!(r#"{{"source":"t","select":["id"],"filter":{{"field":"{field}","op":"{op}"{value}}}}}"#)
}

#[test]
fn the_list_operators_validate_against_their_element_type() {
    assert!(validate(&filter("has", "tags", r#","value":"math""#)).is_ok());
    assert!(validate(&filter("has", "rooms", r#","value":101"#)).is_ok());
    assert!(validate(&filter("has_all", "tags", r#","value":["a","b"]"#)).is_ok());
    assert!(validate(&filter("has_all", "rooms", r#","value":[]"#)).is_ok());
    assert!(validate(&filter("is_empty", "tags", "")).is_ok());

    // An element of the wrong type, a NULL, a scalar where an array is due.
    assert_eq!(
        validate(&filter("has", "rooms", r#","value":"x""#))
            .unwrap_err()
            .code(),
        "ValueTypeMismatch"
    );
    // A `null` value is no value at all: the query does not even read.
    assert!(opengrid_json::from_str::<Query>(&filter("has", "tags", r#","value":null"#)).is_err());
    assert_eq!(
        validate(&filter("has_all", "tags", r#","value":["a",null]"#))
            .unwrap_err()
            .code(),
        "NullInList"
    );
    assert_eq!(
        validate(&filter("has_all", "tags", r#","value":"a""#))
            .unwrap_err()
            .code(),
        "ListExpected"
    );
}

#[test]
fn a_list_has_no_other_comparison_and_a_scalar_no_list_operator() {
    for op in ["eq", "ne", "lt", "in", "contains", "starts_with"] {
        let value = if op == "in" {
            r#","value":[]"#
        } else {
            r#","value":"a""#
        };
        assert_eq!(
            validate(&filter(op, "tags", value)).unwrap_err().code(),
            "OperatorNotSupported",
            "{op}"
        );
    }
    assert_eq!(
        validate(&filter("has", "id", r#","value":1"#))
            .unwrap_err()
            .code(),
        "OperatorNotSupported"
    );
    assert_eq!(
        validate(&filter("is_empty", "id", "")).unwrap_err().code(),
        "OperatorNotSupported"
    );
    // The null checks are for every column.
    assert!(validate(&filter("is_null", "tags", "")).is_ok());
}

#[test]
fn a_list_has_no_order_to_sort_group_or_extreme_on() {
    let sort = r#"{"source":"t","select":["id","tags"],"sort":[{"field":"tags"}]}"#;
    assert_eq!(validate(sort).unwrap_err().code(), "ListNotOrdered");
    let group = r#"{"source":"t","select":["tags"],"group":["tags"],"aggregate":[{"fn":"count","as":"n"}]}"#;
    assert_eq!(validate(group).unwrap_err().code(), "ListNotOrdered");
    let max = r#"{"source":"t","aggregate":[{"field":"tags","fn":"max","as":"m"}]}"#;
    assert_eq!(validate(max).unwrap_err().code(), "AggregateTypeMismatch");
    let count = r#"{"source":"t","aggregate":[{"field":"tags","fn":"count","as":"n"}]}"#;
    assert!(validate(count).is_ok());
}

#[test]
fn the_list_operators_survive_a_round_trip() {
    for text in [
        filter("has", "tags", r#","value":"math""#),
        filter("has_all", "rooms", r#","value":[1,2]"#),
        filter("is_empty", "tags", ""),
    ] {
        let validated = validate(&text).unwrap();
        let back = Query::from(&validated);
        let again = back.validate(&schema(), &Limits::default()).unwrap();
        assert_eq!(validated.filter, again.filter, "{text}");
    }
}
