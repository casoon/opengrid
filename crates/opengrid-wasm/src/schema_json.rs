//! Reading the schema-JSON form the ingest side accepts.
//!
//! The canonical shape is the one `crates/opengrid-conformance/data/orders.schema.json`
//! uses:
//!
//! ```json
//! { "fields": [ { "name": "id", "type": "int64", "nullable": false } ] }
//! ```
//!
//! `type` is a snake_case name — `bool`, `int64`, `float64`, `utf8`, `date`,
//! `timestamp` — or the object form `{ "decimal": { "precision": 12, "scale": 2 } }`.
//! `nullable` defaults to `false`, like a JSON Schema `required` field.
//!
//! Only the **reader** lives here. `opengrid_types::Schema` deliberately has no
//! serde impls, and whether this shape becomes the canonical *wire* form of a
//! schema is still open (plan/noch-zu-klaeren.md §Serialisierungsform von
//! `Schema`; it belongs to point 23, the first point that ships a schema over
//! the wire). Keeping the reader in this crate leaves that decision open and
//! adds no dependency to `opengrid-types`.
//!
//! The conformance suite reads the same shape with its own private structs
//! (`opengrid-conformance/src/case.rs`, `SchemaFile`) — it is a `publish = false`
//! dev-only crate, so this is not a shared home. The two readers are the same
//! shape by construction, and point 23 collapses them into one when the wire
//! form is decided.

use opengrid_types::{DataType, Field, FieldName, Schema};
use serde::Deserialize;

/// Parses a schema in the `orders.schema.json` shape.
///
/// The error is the diagnosis, ready to be shown to the user: it names the
/// offending field, because a hand-edited schema is the normal input here.
pub fn from_json(json: &str) -> Result<Schema, String> {
    let file: SchemaFile = serde_json::from_str(json).map_err(|error| error.to_string())?;
    file.into_schema()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaFile {
    fields: Vec<FieldRepr>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldRepr {
    name: String,
    #[serde(rename = "type")]
    data_type: TypeRepr,
    #[serde(default)]
    nullable: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum TypeRepr {
    Bool,
    Int64,
    Float64,
    Decimal { precision: u8, scale: u8 },
    Utf8,
    Date,
    Timestamp,
}

impl SchemaFile {
    fn into_schema(self) -> Result<Schema, String> {
        let mut fields = Vec::with_capacity(self.fields.len());
        for repr in self.fields {
            let name = FieldName::new(&repr.name)
                .map_err(|error| format!("field {:?}: {error}", repr.name))?;
            let data_type = repr
                .data_type
                .to_data_type()
                .map_err(|error| format!("field {:?}: {error}", repr.name))?;
            fields.push(if repr.nullable {
                Field::new(name, data_type)
            } else {
                Field::required(name, data_type)
            });
        }
        Ok(Schema::new(fields))
    }
}

impl TypeRepr {
    fn to_data_type(&self) -> Result<DataType, opengrid_types::ValueError> {
        Ok(match self {
            TypeRepr::Bool => DataType::Bool,
            TypeRepr::Int64 => DataType::Int64,
            TypeRepr::Float64 => DataType::Float64,
            TypeRepr::Decimal { precision, scale } => DataType::decimal(*precision, *scale)?,
            TypeRepr::Utf8 => DataType::Utf8,
            TypeRepr::Date => DataType::Date,
            TypeRepr::Timestamp => DataType::Timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the conformance dataset ships — the reader has to accept the
    /// file the demo and the browser tests feed it.
    const ORDERS: &str = include_str!("../../opengrid-conformance/data/orders.schema.json");

    #[test]
    fn reads_the_conformance_schema() {
        let schema = from_json(ORDERS).expect("the dataset schema parses");
        assert_eq!(schema.len(), 10);
        assert_eq!(schema.index_of("id"), Some(0));
        assert_eq!(
            schema.data_type("amount"),
            Some(DataType::decimal(12, 2).unwrap())
        );
        assert!(!schema.field("id").unwrap().nullable);
        assert!(schema.field("customer").unwrap().nullable);
    }

    #[test]
    fn rejects_a_misspelled_type() {
        let error = from_json(r#"{"fields":[{"name":"a","type":"integer"}]}"#).unwrap_err();
        assert!(
            error.contains("a"),
            "the diagnosis names the field: {error}"
        );
    }

    #[test]
    fn rejects_an_unknown_key() {
        assert!(from_json(r#"{"fields":[{"name":"a","type":"int64","extra":1}]}"#).is_err());
    }

    #[test]
    fn rejects_an_invalid_identifier() {
        assert!(from_json(r#"{"fields":[{"name":"1bad","type":"int64"}]}"#).is_err());
    }
}
