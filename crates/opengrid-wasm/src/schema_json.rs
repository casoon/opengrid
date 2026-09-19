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
//! `nullable` defaults to `false`, like a JSON Schema `required` field. Since
//! point 54 a field may also carry `"from"` and be computed rather than stored.
//!
//! **This is no longer a reader.** Until point 23 the module held its own structs,
//! because `opengrid_types::Schema` had no serde impls and the wire form was not
//! decided yet; the doc comment said they would collapse into one once it was.
//! E17 decided it, so what is left here is the thin part that was never shared:
//! turning a parse failure into a sentence a person can act on, and checking the
//! derivations (point 54) — a schema whose derivation is broken must not load at
//! all, or the browser shows a column full of NULL and nobody notices.

use opengrid_types::Schema;

/// Parses a schema in the `orders.schema.json` shape.
///
/// The error is the diagnosis, ready to be shown to the user: a hand-edited
/// schema is the normal input here.
pub fn from_json(json: &str) -> Result<Schema, String> {
    let schema: Schema = serde_json::from_str(json).map_err(|error| error.to_string())?;
    schema.check().map_err(|error| error.to_string())?;
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::DataType;

    /// The shape the conformance dataset ships — the reader has to accept the
    /// file the demo and the browser tests feed it.
    const ORDERS: &str = include_str!("../../opengrid-conformance/data/orders.schema.json");

    #[test]
    fn reads_the_conformance_schema() {
        let schema = from_json(ORDERS).expect("the dataset schema parses");
        assert_eq!(schema.len(), 13);
        assert_eq!(schema.index_of("id"), Some(0));
        assert_eq!(
            schema.data_type("amount"),
            Some(DataType::decimal(12, 2).unwrap())
        );
        assert!(!schema.field("id").unwrap().nullable);
        assert!(schema.field("customer").unwrap().nullable);
    }

    /// Ten columns in the file, three computed from them (point 54).
    #[test]
    fn reads_the_derived_columns() {
        let schema = from_json(ORDERS).expect("the dataset schema parses");
        assert_eq!(schema.stored().len(), 10);
        let year = schema.field("ordered_year").expect("the derived column");
        assert_eq!(year.data_type, DataType::Int64);
        assert_eq!(
            year.from.as_ref().map(|from| from.field.as_str()),
            Some("ordered_on")
        );
    }

    #[test]
    fn rejects_a_misspelled_type() {
        let error = from_json(r#"{"fields":[{"name":"a","type":"integer"}]}"#).unwrap_err();
        assert!(
            error.contains("integer"),
            "the diagnosis names what it could not read: {error}"
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

    /// A derivation that cannot work stops the schema from loading at all.
    #[test]
    fn rejects_a_broken_derivation() {
        let error = from_json(
            r#"{"fields":[{"name":"y","type":"int64","nullable":true,
                 "from":{"part":"year","field":"nope"}}]}"#,
        )
        .unwrap_err();
        assert!(error.contains("nope"), "the diagnosis names it: {error}");
    }
}
