//! JSON: an array of row objects, keys in the order of the columns (E33).
//!
//! The values are the wire form's (E17): a decimal, a date and a timestamp as
//! strings in their exact notation, a non-finite float spelled out (E13),
//! NULL as `null`. Row objects rather than the wire's columns, because an
//! export is read row by row — and because rows can be written piece by piece.

use opengrid_datasource::QueryResult;

/// The rows of one result as objects, joined by commas; no brackets. `first`:
/// no row has been written yet, so no comma leads — not "the first piece":
/// after an empty first piece it is still true.
pub fn json_rows(result: &QueryResult, first: bool) -> String {
    let names: Vec<String> = result
        .schema
        .fields()
        .iter()
        .map(|field| serde_json::to_string(field.name.as_str()).expect("a name serializes"))
        .collect();
    let mut out = String::new();
    for row in 0..result.row_count() {
        if !(first && row == 0) {
            out.push(',');
        }
        out.push('{');
        for (col, column) in result.columns.iter().enumerate() {
            if col > 0 {
                out.push(',');
            }
            out.push_str(&names[col]);
            out.push(':');
            out.push_str(&serde_json::to_string(&column[row]).expect("a value serializes"));
        }
        out.push('}');
    }
    out
}

/// A JSON array written in pieces: `[` with the first piece, `]` at the end.
#[derive(Debug, Default)]
pub struct JsonWriter {
    started: bool,
    rows: bool,
}

impl JsonWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next piece.
    pub fn write(&mut self, result: &QueryResult) -> String {
        let mut out = String::new();
        if !self.started {
            self.started = true;
            out.push('[');
        }
        out.push_str(&json_rows(result, !self.rows));
        self.rows |= result.row_count() > 0;
        out
    }

    /// The end: `]`, or `[]` when nothing was written.
    pub fn finish(self) -> String {
        if self.started {
            "]".to_owned()
        } else {
            "[]".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{DataType, Field, FieldName, Schema, Value};

    #[test]
    fn rows_are_objects_in_the_wire_notation_and_pieces_join_up() {
        let schema = Schema::new(vec![
            Field::new(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(
                FieldName::new("amount").unwrap(),
                DataType::Decimal {
                    precision: 12,
                    scale: 2,
                },
            ),
            Field::new(FieldName::new("ratio").unwrap(), DataType::Float64),
        ]);
        let piece = |id: i64, ratio: Value| {
            QueryResult::new(
                schema.clone(),
                vec![
                    vec![Value::Int64(id)],
                    vec![
                        Value::from_wire_str(
                            "10.50",
                            &DataType::Decimal {
                                precision: 12,
                                scale: 2,
                            },
                        )
                        .unwrap(),
                    ],
                    vec![ratio],
                ],
                2,
            )
        };
        let mut writer = JsonWriter::new();
        let mut out = writer.write(&piece(1, Value::Float64(f64::INFINITY)));
        out.push_str(&writer.write(&piece(2, Value::Null)));
        out.push_str(&writer.finish());
        assert_eq!(
            out,
            r#"[{"id":1,"amount":"10.50","ratio":"Infinity"},{"id":2,"amount":"10.50","ratio":null}]"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
    }

    #[test]
    fn an_export_of_nothing_is_an_empty_array() {
        assert_eq!(JsonWriter::new().finish(), "[]");
        let empty = QueryResult::new(Schema::new(Vec::new()), Vec::new(), 0);
        let mut writer = JsonWriter::new();
        let out = writer.write(&empty) + &writer.finish();
        assert_eq!(out, "[]");
    }
}
