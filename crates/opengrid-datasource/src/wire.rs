//! The JSON form of a result and of an error (plan point 23).
//!
//! This is the contract between the server of point 24 and every client — the
//! browser's `RestDataSource`, the demo pages, and anything else that speaks to
//! `POST /query/{source}`. It is column-oriented (E6) and Arrow-free (E14), the
//! same shape the [`QueryResult`] type has.
//!
//! ```json
//! {
//!   "total_count": 100000,
//!   "row_count": 2,
//!   "columns": [
//!     { "name": "customer", "type": "utf8", "nullable": true, "values": ["Alpha", null] },
//!     { "name": "amount", "type": { "decimal": { "precision": 12, "scale": 2 } },
//!       "nullable": true, "values": ["10.00", "20.50"] }
//!   ]
//! }
//! ```
//!
//! Two decisions are worth naming.
//!
//! * **The type sits on the column, not in a separate schema block.** A result
//!   carries its schema exactly once, next to the values it describes. The older
//!   envelope (point 10) named the columns but not their types, which is why the
//!   grid had to invent an all-`Utf8` display schema; a second, separate schema
//!   block would have reintroduced the question of which of the two wins when
//!   they disagree.
//! * **Reading needs the type.** JSON cannot tell a decimal from a string
//!   (`"10.00"`) or `NaN` from the word (E13), so values are read through
//!   [`Value::deserialize_typed`] with the column's type — which is why this
//!   module walks the JSON itself instead of deriving `Deserialize`: the derived
//!   reader would depend on `type` arriving before `values`, and JSON object keys
//!   have no order.
//!
//! The error form is the counterpart:
//!
//! ```json
//! { "error": { "code": "validation", "message": "…", "path": "filter.and[1].value" } }
//! ```

use opengrid_types::{DataType, Field, FieldName, Schema, Value};
use serde::{Deserialize, Serialize};

use crate::QueryResult;

/// What went wrong, as a closed set (plan point 23).
///
/// Closed on purpose: a client has to be able to branch on it, and a free-text
/// message is not something to branch on. The human-readable part is
/// [`WireError::message`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The query is not valid against the source's schema, or breaks a limit
    /// that belongs to the query itself (`QueryError`, including its JSON path).
    Validation,
    /// No source of that name is configured.
    UnknownSource,
    /// The request exceeded a server limit — payload size, timeout, page size.
    /// The same request fails again; a smaller one may not.
    LimitExceeded,
    /// The server is running as much of this as it runs at once — an export
    /// over `max_concurrent_exports` (issue #16). The request itself is fine:
    /// the same one may succeed later. Kept apart from
    /// [`LimitExceeded`](Self::LimitExceeded) because a client does the
    /// opposite for the two: wait and retry here, narrow the request there.
    Busy,
    /// The caller may not do this: no token, wrong token.
    Unauthorized,
    /// The source failed: engine, database, transport.
    Backend,
    /// The request body was not readable at all.
    Malformed,
}

/// A failure in the form it travels in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireError {
    /// The machine-readable class.
    pub code: ErrorCode,
    /// One sentence for a person. Never a secret, never a token.
    pub message: String,
    /// Where in the query it sits, when that is known — the JSON path that
    /// `QueryError` already carries (`filter.and[1].value`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub path: Option<String>,
}

impl WireError {
    /// A failure without a position.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
        }
    }

    /// A failure that names the place in the query it came from.
    pub fn at(code: ErrorCode, message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: Some(path.into()),
        }
    }

    /// The envelope: `{ "error": { … } }`.
    pub fn to_json(&self) -> String {
        serde_json::json!({ "error": self }).to_string()
    }

    /// Reads the envelope back, or `None` when this is not an error body.
    pub fn from_json(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        serde_json::from_value(value.get("error")?.clone()).ok()
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.path {
            Some(path) => write!(f, "{} ({path})", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for WireError {}

/// A result body that could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    message: String,
}

impl ReadError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// What was wrong with the body.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ReadError {}

/// One column on the wire: its name, its type, and its values.
#[derive(Serialize)]
struct ColumnOut<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    data_type: DataType,
    nullable: bool,
    values: &'a [Value],
}

/// Writes a result in the wire form.
pub fn result_to_json(result: &QueryResult) -> String {
    let columns: Vec<ColumnOut<'_>> = result
        .schema
        .fields()
        .iter()
        .zip(&result.columns)
        .map(|(field, values)| ColumnOut {
            name: field.name.as_str(),
            data_type: field.data_type,
            nullable: field.nullable,
            values,
        })
        .collect();
    serde_json::json!({
        "total_count": result.total_count,
        "row_count": result.row_count(),
        "columns": columns,
    })
    .to_string()
}

/// Reads a result from the wire form.
///
/// Strict on purpose: a column without a type, a value that does not fit its
/// column, or columns of differing length are errors here rather than surprises
/// later. The invariant of [`QueryResult`] holds for everything this returns.
pub fn result_from_json(json: &str) -> Result<QueryResult, ReadError> {
    let body: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| ReadError::new(format!("result JSON: {error}")))?;

    let total_count = body
        .get("total_count")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ReadError::new("result has no total_count"))?;
    let raw_columns = body
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ReadError::new("result has no columns"))?;

    let mut fields = Vec::with_capacity(raw_columns.len());
    let mut columns = Vec::with_capacity(raw_columns.len());
    for column in raw_columns {
        let name = column
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ReadError::new("a column has no name"))?;
        let name = FieldName::new(name)
            .map_err(|error| ReadError::new(format!("column {name:?}: {error}")))?;
        let data_type: DataType = column
            .get("type")
            .ok_or_else(|| ReadError::new(format!("column {name} has no type")))
            .and_then(|raw| {
                serde_json::from_value(raw.clone())
                    .map_err(|error| ReadError::new(format!("column {name}: {error}")))
            })?;
        let nullable = column
            .get("nullable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let raw_values = column
            .get("values")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ReadError::new(format!("column {name} has no values")))?;

        let mut values = Vec::with_capacity(raw_values.len());
        for raw in raw_values {
            let value = Value::deserialize_typed(raw.clone(), &data_type).map_err(
                |error: serde_json::Error| ReadError::new(format!("column {name}: {error}")),
            )?;
            values.push(value);
        }

        fields.push(Field {
            name,
            data_type,
            nullable,
            // A result column holds values; where they came from is the source's
            // business and does not travel (point 54).
            from: None,
        });
        columns.push(values);
    }

    if let Some(first) = columns.first() {
        let rows = first.len();
        if let Some(bad) = columns.iter().position(|column| column.len() != rows) {
            return Err(ReadError::new(format!(
                "column {} has {} values, the first has {rows}",
                fields[bad].name,
                columns[bad].len()
            )));
        }
    }

    Ok(QueryResult::new(Schema::new(fields), columns, total_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Date, Decimal, Timestamp};

    fn field(name: &str, data_type: DataType, nullable: bool) -> Field {
        let name = FieldName::new(name).unwrap();
        if nullable {
            Field::new(name, data_type)
        } else {
            Field::required(name, data_type)
        }
    }

    /// Every type of the type system, with its hard values, survives the trip.
    #[test]
    fn a_result_survives_a_round_trip() {
        let schema = Schema::new(vec![
            field("id", DataType::Int64, false),
            field("note", DataType::Utf8, true),
            field("amount", DataType::decimal(12, 2).unwrap(), true),
            field("ratio", DataType::Float64, true),
            field("flag", DataType::Bool, true),
            field("ordered_on", DataType::Date, true),
            field("created_at", DataType::Timestamp, true),
        ]);
        let result = QueryResult::new(
            schema,
            vec![
                vec![Value::Int64(1), Value::Int64(-2)],
                // A string that looks like a number, and an empty one: both stay Utf8.
                vec![Value::Utf8("10.00".to_owned()), Value::Utf8(String::new())],
                vec![Value::Decimal(Decimal::new(-1050, 2)), Value::Null],
                vec![Value::Float64(f64::NAN), Value::Float64(-0.0)],
                vec![Value::Bool(true), Value::Null],
                vec![
                    Value::Date(Date::from_ymd(2026, 1, 1).unwrap()),
                    Value::Null,
                ],
                vec![
                    Value::Timestamp(Timestamp::from_micros(1_767_225_600_000_001)),
                    Value::Null,
                ],
            ],
            9_999,
        );

        let json = result_to_json(&result);
        let read = result_from_json(&json).expect("round trip");

        assert_eq!(read.schema, result.schema);
        assert_eq!(read.total_count, 9_999);
        // NaN != NaN, so the float column is compared by hand.
        assert!(matches!(read.columns[3][0], Value::Float64(f) if f.is_nan()));
        assert!(
            matches!(read.columns[3][1], Value::Float64(f) if f == 0.0 && f.is_sign_negative())
        );
        for index in [0, 1, 2, 4, 5, 6] {
            assert_eq!(read.columns[index], result.columns[index], "column {index}");
        }
    }

    /// The types are what the older envelope lacked: without them a decimal
    /// would come back as a string.
    #[test]
    fn a_decimal_comes_back_as_a_decimal() {
        let result = QueryResult::new(
            Schema::new(vec![field(
                "amount",
                DataType::decimal(12, 2).unwrap(),
                true,
            )]),
            vec![vec![Value::Decimal(Decimal::new(1000, 2))]],
            1,
        );
        let json = result_to_json(&result);
        assert!(json.contains("\"10.00\""), "{json}");
        assert!(json.contains("\"decimal\""), "{json}");
        assert_eq!(result_from_json(&json).unwrap(), result);
    }

    /// An empty result still carries its schema — the grid draws its header from
    /// it even when nothing matched.
    #[test]
    fn an_empty_result_still_carries_its_schema() {
        let schema = Schema::new(vec![field("id", DataType::Int64, false)]);
        let result = QueryResult::new(schema.clone(), vec![vec![]], 0);
        let read = result_from_json(&result_to_json(&result)).unwrap();
        assert_eq!(read.schema, schema);
        assert_eq!(read.total_count, 0);
        assert_eq!(read.row_count(), 0);
    }

    #[test]
    fn a_malformed_result_is_an_error() {
        for (json, expected) in [
            ("{", "result JSON"),
            (r#"{"columns":[]}"#, "total_count"),
            (r#"{"total_count":0}"#, "columns"),
            (
                r#"{"total_count":0,"columns":[{"type":"int64","values":[]}]}"#,
                "name",
            ),
            (
                r#"{"total_count":0,"columns":[{"name":"id","values":[]}]}"#,
                "type",
            ),
            (
                r#"{"total_count":0,"columns":[{"name":"id","type":"int64"}]}"#,
                "values",
            ),
            // A value that does not fit its column.
            (
                r#"{"total_count":1,"columns":[{"name":"id","type":"int64","values":["nope"]}]}"#,
                "id",
            ),
            // Columns of different lengths break the invariant of QueryResult.
            (
                r#"{"total_count":1,"columns":[{"name":"a","type":"int64","values":[1]},{"name":"b","type":"int64","values":[]}]}"#,
                "values",
            ),
        ] {
            let error = result_from_json(json).expect_err(json);
            assert!(
                error.message().contains(expected),
                "{json}: {} does not mention {expected}",
                error.message()
            );
        }
    }

    /// The error envelope keeps the class and the place, so a client can branch
    /// on the one and point at the other.
    #[test]
    fn an_error_keeps_its_code_and_path() {
        let error = WireError::at(
            ErrorCode::Validation,
            "value 100 is not a decimal",
            "filter.and[1].value",
        );
        let json = error.to_json();
        let read = WireError::from_json(&json).expect("envelope");
        assert_eq!(read, error);
        assert_eq!(read.code, ErrorCode::Validation);
        assert_eq!(
            read.to_string(),
            "value 100 is not a decimal (filter.and[1].value)"
        );

        let plain = WireError::new(ErrorCode::UnknownSource, "unknown source \"orders\"");
        assert_eq!(WireError::from_json(&plain.to_json()).unwrap(), plain);
        assert!(!plain.to_json().contains("path"), "no empty path key");

        // A result body is not an error body.
        assert!(WireError::from_json(r#"{"total_count":0,"columns":[]}"#).is_none());
    }

    /// Every code travels as its snake_case name and reads back as itself —
    /// `busy` (issue #16) included, apart from `limit_exceeded`. The names are
    /// what a page branches on, so they are spelled out here, not derived.
    #[test]
    fn every_code_travels_under_its_name() {
        for (code, name) in [
            (ErrorCode::Validation, "validation"),
            (ErrorCode::UnknownSource, "unknown_source"),
            (ErrorCode::LimitExceeded, "limit_exceeded"),
            (ErrorCode::Busy, "busy"),
            (ErrorCode::Unauthorized, "unauthorized"),
            (ErrorCode::Backend, "backend"),
            (ErrorCode::Malformed, "malformed"),
        ] {
            let error = WireError::new(code, "a sentence");
            let json = error.to_json();
            assert!(json.contains(&format!(r#""code":"{name}""#)), "{json}");
            assert_eq!(WireError::from_json(&json).unwrap().code, code, "{json}");
        }
        let busy = r#"{"error":{"code":"busy","message":"try again later"}}"#;
        assert_eq!(WireError::from_json(busy).unwrap().code, ErrorCode::Busy);
    }
}
