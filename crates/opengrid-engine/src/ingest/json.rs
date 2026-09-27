//! JSON ingest: an array of objects in the column-oriented wire format (E6/E13).
//!
//! Columns are matched by field name, not by position, so a JSON object does
//! not have to follow the schema's order. Shapes the contract does not define
//! are errors, not guesses: an unknown field is rejected the same way the query
//! reader rejects it.

use opengrid_types::{Schema, Value};
use serde_json::Value as Json;

use super::{IngestError, cell, check_null};

/// Reads a JSON array of objects into rows of typed values.
pub(crate) fn rows(text: &str, schema: &Schema) -> Result<Vec<Vec<Value>>, IngestError> {
    let parsed: Json = serde_json::from_str(text).map_err(|error| IngestError::Input {
        message: format!("input is not JSON: {error}"),
    })?;
    let Json::Array(rows) = parsed else {
        return Err(IngestError::Input {
            message: "expected a JSON array of objects".to_owned(),
        });
    };
    rows.into_iter()
        .enumerate()
        .map(|(row, value)| one(value, row, schema))
        .collect()
}

/// One row: every schema field once, nothing else.
fn one(value: Json, row: usize, schema: &Schema) -> Result<Vec<Value>, IngestError> {
    let Json::Object(mut object) = value else {
        return Err(IngestError::Json {
            row,
            field: None,
            message: "expected an object".to_owned(),
        });
    };
    let mut cells = Vec::with_capacity(schema.len());
    for field in schema.fields() {
        let name = field.name.as_str();
        let value = match object.remove(name) {
            Some(cell) => {
                cell::from_json(cell, field.data_type).map_err(|message| IngestError::Json {
                    row,
                    field: Some(name.to_owned()),
                    message,
                })?
            }
            None => Value::Null,
        };
        cells.push(
            check_null(field, value).map_err(|message| IngestError::Json {
                row,
                field: Some(name.to_owned()),
                message,
            })?,
        );
    }
    // serde_json keeps object keys sorted, so which field is reported for a row
    // with several unknown fields does not depend on hash order.
    if let Some((unknown, _)) = object.into_iter().next() {
        return Err(IngestError::Json {
            row,
            field: Some(unknown),
            message: "unknown field".to_owned(),
        });
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Field, FieldName};

    fn schema() -> Schema {
        Schema::new(vec![
            Field::required(
                FieldName::new("id").unwrap(),
                opengrid_types::DataType::Int64,
            ),
            Field::new(
                FieldName::new("note").unwrap(),
                opengrid_types::DataType::Utf8,
            ),
        ])
    }

    #[test]
    fn reads_rows_by_name() {
        let rows = rows(r#"[{"note": "a", "id": 1}, {"id": 2}]"#, &schema()).unwrap();
        assert_eq!(
            rows,
            vec![
                vec![Value::Int64(1), Value::Utf8("a".into())],
                vec![Value::Int64(2), Value::Null],
            ]
        );
    }

    #[test]
    fn rejects_a_missing_non_nullable_field() {
        let error = rows(r#"[{"note": "a"}]"#, &schema()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "JSON row 0, field id: NULL is not allowed in a non-nullable column"
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = rows(r#"[{"id": 1, "extra": 2, "another": 3}]"#, &schema()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "JSON row 0, field another: unknown field"
        );
    }

    #[test]
    fn rejects_shapes_the_contract_does_not_know() {
        assert_eq!(
            rows(r#"{"id": 1}"#, &schema()).unwrap_err().to_string(),
            "input: expected a JSON array of objects"
        );
        assert_eq!(
            rows(r#"[1, 2]"#, &schema()).unwrap_err().to_string(),
            "JSON row 0: expected an object"
        );
        assert!(
            rows(r#"[{"id": }]"#, &schema())
                .unwrap_err()
                .to_string()
                .starts_with("input: input is not JSON")
        );
    }
}
