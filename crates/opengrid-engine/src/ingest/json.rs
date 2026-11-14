//! JSON ingest: an array of objects in the column-oriented wire format (E6/E13).
//!
//! Columns are matched by field name, not by position, so a JSON object does
//! not have to follow the schema's order. Shapes the contract does not define
//! are errors, not guesses: an unknown field is rejected the same way the query
//! reader rejects it.

use opengrid_columns::Table;
use opengrid_json::{Json, Number};
use opengrid_types::{DataType, Schema, Value};

use super::{Builders, IngestError, Layout, cell, check_null};

/// Reads a JSON array of objects into rows of typed values.
pub(crate) fn rows(text: &str, schema: &Schema) -> Result<Vec<Vec<Value>>, IngestError> {
    let parsed = Json::parse(text).map_err(|error| IngestError::Input {
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
        // A key given twice counts once, the last time — as a JSON reader
        // customarily takes it.
        let given = object.remove(name);
        while object.remove(name).is_some() {}
        let value = match given {
            Some(cell) => {
                cell::from_json(&cell, field.data_type).map_err(|message| IngestError::Json {
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
    // Of several unknown fields the first in name order is reported, so the
    // message does not depend on how the object was written.
    if let Some(unknown) = object.keys().min() {
        return Err(IngestError::Json {
            row,
            field: Some(unknown.to_owned()),
            message: "unknown field".to_owned(),
        });
    }
    Ok(cells)
}

/// The fast path of [`super::load_json`] (issue #426): one pass over the text, straight into the
/// column builders — no JSON tree of a hundred thousand objects, no row of values per row.
///
/// It reads only what is plain: an array of objects whose keys are unescaped names of the
/// schema, each at most once, and scalar values that need no explanation. **Anything else —
/// an escape in a key, an unknown or repeated key, a nested value for a scalar column, a value
/// the column refuses, a malformed text — ends it with `None`**, and the caller reads the text
/// again the general way, which owns every error message. So it can only ever produce the table
/// the general path would produce; `a_fast_table_is_the_general_table` holds the two together.
pub(super) fn fast(text: &str, schema: &Schema, stored: &Schema, layout: &Layout) -> Option<Table> {
    let bytes = text.as_bytes();
    let fields = stored.fields();
    let mut at = skip_whitespace(bytes, 0);
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    at = skip_whitespace(bytes, at + 1);
    if bytes.get(at) == Some(&b']') {
        // An empty array is an empty table: the general path says so, with its schema checks.
        return None;
    }
    let mut builders = Builders::new(schema, 0);
    let mut seen = vec![false; fields.len()];
    loop {
        if bytes.get(at) != Some(&b'{') {
            return None;
        }
        seen.fill(false);
        let mut cells = vec![Value::Null; fields.len()];
        at = skip_whitespace(bytes, at + 1);
        if bytes.get(at) == Some(&b'}') {
            at += 1;
        } else {
            // Rows mostly repeat the schema's order, so the next key is looked for there first.
            let mut hint = 0;
            loop {
                if bytes.get(at) != Some(&b'"') {
                    return None;
                }
                let (key, next) = plain_string(bytes, text, at)?;
                let index = field_index(fields, key, hint)?;
                if seen[index] {
                    return None;
                }
                seen[index] = true;
                hint = index + 1;
                at = skip_whitespace(bytes, next);
                if bytes.get(at) != Some(&b':') {
                    return None;
                }
                at = skip_whitespace(bytes, at + 1);
                let (value, next) = scalar(bytes, text, at, fields[index].data_type)?;
                cells[index] = check_null(&fields[index], value).ok()?;
                at = skip_whitespace(bytes, next);
                match bytes.get(at) {
                    Some(b',') => at = skip_whitespace(bytes, at + 1),
                    Some(b'}') => {
                        at += 1;
                        break;
                    }
                    _ => return None,
                }
            }
        }
        for (index, field) in fields.iter().enumerate() {
            if !seen[index] {
                check_null(field, Value::Null).ok()?;
            }
        }
        builders.push(&layout.complete(cells)).ok()?;
        at = skip_whitespace(bytes, at);
        match bytes.get(at) {
            Some(b',') => at = skip_whitespace(bytes, at + 1),
            Some(b']') => {
                at += 1;
                break;
            }
            _ => return None,
        }
    }
    if skip_whitespace(bytes, at) != bytes.len() {
        return None;
    }
    builders.finish().ok()
}

fn skip_whitespace(bytes: &[u8], mut at: usize) -> usize {
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        at += 1;
    }
    at
}

/// A string with no escape and no control character, from its opening quote: its text and the
/// position after its closing quote. Anything with a backslash is the general path's.
fn plain_string<'a>(bytes: &[u8], text: &'a str, open: usize) -> Option<(&'a str, usize)> {
    let start = open + 1;
    let mut at = start;
    loop {
        let byte = *bytes.get(at)?;
        if byte == b'"' {
            // `start` and `at` sit next to ASCII quotes, so they are character boundaries.
            return Some((&text[start..at], at + 1));
        }
        if byte == b'\\' || byte < 0x20 {
            return None;
        }
        at += 1;
    }
}

fn field_index(fields: &[opengrid_types::Field], key: &str, hint: usize) -> Option<usize> {
    if fields
        .get(hint)
        .is_some_and(|field| field.name.as_str() == key)
    {
        return Some(hint);
    }
    fields.iter().position(|field| field.name.as_str() == key)
}

/// One scalar cell, read as the general path reads it: the same JSON value, the same coercion.
fn scalar(bytes: &[u8], text: &str, at: usize, data_type: DataType) -> Option<(Value, usize)> {
    let (json, next) = match *bytes.get(at)? {
        b'"' => {
            let (body, next) = plain_string(bytes, text, at)?;
            if data_type == DataType::Utf8 {
                // The coercion of a string into a text column is the string itself.
                return Some((Value::Utf8(body.to_owned()), next));
            }
            (Json::String(body.to_owned()), next)
        }
        b'n' => (Json::Null, literal(bytes, at, b"null")?),
        b't' => (Json::Bool(true), literal(bytes, at, b"true")?),
        b'f' => (Json::Bool(false), literal(bytes, at, b"false")?),
        b'-' | b'0'..=b'9' => number(bytes, at)?,
        _ => return None,
    };
    Some((cell::from_json(&json, data_type).ok()?, next))
}

fn literal(bytes: &[u8], at: usize, word: &[u8]) -> Option<usize> {
    (bytes.get(at..at + word.len())? == word).then_some(at + word.len())
}

/// A number, classified as `opengrid-json` classifies it: a non-negative integer is a `u64`, a
/// negative one an `i64` (`-0` is the float), anything with a fraction or an exponent — or past
/// 64 bits — a float. A shape the parser would refuse is `None`.
fn number(bytes: &[u8], start: usize) -> Option<(Json, usize)> {
    let digits = |mut at: usize| {
        while matches!(bytes.get(at), Some(b'0'..=b'9')) {
            at += 1;
        }
        at
    };
    let mut at = start;
    let negative = bytes[at] == b'-';
    if negative {
        at += 1;
    }
    match bytes.get(at)? {
        b'0' => {
            at += 1;
            if matches!(bytes.get(at), Some(b'0'..=b'9')) {
                return None;
            }
        }
        b'1'..=b'9' => at = digits(at),
        _ => return None,
    }
    let mut float = false;
    if bytes.get(at) == Some(&b'.') {
        float = true;
        at += 1;
        if !matches!(bytes.get(at), Some(b'0'..=b'9')) {
            return None;
        }
        at = digits(at);
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        float = true;
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        if !matches!(bytes.get(at), Some(b'0'..=b'9')) {
            return None;
        }
        at = digits(at);
    }
    let text = std::str::from_utf8(&bytes[start..at]).ok()?;
    if !float {
        let integer = if negative {
            text.parse::<i64>()
                .ok()
                .filter(|value| *value != 0)
                .map(Number::NegInt)
        } else {
            text.parse::<u64>().ok().map(Number::PosInt)
        };
        if let Some(number) = integer {
            return Some((Json::Number(number), at));
        }
    }
    let value: f64 = text.parse().ok()?;
    value
        .is_finite()
        .then_some((Json::Number(Number::Float(value)), at))
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

    mod fast_path {
        use super::*;
        use crate::ingest::{Layout, load_json, load_json_general, text};
        use opengrid_types::{DataType, Field, FieldName, Schema};
        use proptest::prelude::*;

        fn schema() -> Schema {
            let f = |n: &str, t: DataType| Field::new(FieldName::new(n).unwrap(), t);
            Schema::new(vec![
                Field::required(FieldName::new("id").unwrap(), DataType::Int64),
                f("name", DataType::Utf8),
                f("amount", DataType::Float64),
                f("flag", DataType::Bool),
                f("day", DataType::Date),
            ])
        }

        /// Both ways, on the same text: `None` when the fast path declines, `Some(general)` when it
        /// answers, so a test can say what must hold when it does.
        fn both(input: &str) -> (Option<Table>, Result<Table, IngestError>) {
            let schema = schema();
            let stored = schema.stored();
            let layout = Layout::of(&schema, &stored);
            let text = text(input.as_bytes()).unwrap();
            (
                fast(text, &schema, &stored, &layout),
                load_json_general(text, &schema, &stored, &layout),
            )
        }

        /// Equal **to the bit**: `0.0 == -0.0` and `NaN != NaN` in the language, and neither is
        /// what "the same table" means here; the debug form tells them apart.
        fn same(a: &Table, b: &Table) -> bool {
            a.num_rows() == b.num_rows()
                && format!("{:?}", a.to_values()) == format!("{:?}", b.to_values())
        }

        #[test]
        fn plain_text_takes_the_fast_path() {
            let (fast, general) = both(
                r#" [ {"id": 1, "name": "a b", "amount": 1.5, "flag": true, "day": "2026-03-05"},
                      {"day": null, "flag": false, "amount": -2e3, "name": null, "id": 9223372036854775807} ] "#,
            );
            let fast = fast.expect("plain JSON is the fast path's");
            assert!(same(&fast, &general.unwrap()));
        }

        /// What the fast path does not read, it hands over — and the general path then says what
        /// it always said.
        #[test]
        fn what_it_will_not_read_goes_to_the_general_path() {
            for input in [
                "[]",
                "{}",
                r#"[{"id": 1, "extra": 2}]"#,
                r#"[{"id": 1, "id": 2}]"#,
                r#"[{"id": 1, "na\u006de": "a"}]"#,
                r#"[{"id": 1, "name": "a\nb"}]"#,
                r#"[{"id": 1, "name": ["x"]}]"#,
                r#"[{"name": "a"}]"#,
                r#"[{"id": 01}]"#,
                r#"[{"id": 1} {"id": 2}]"#,
                r#"[{"id": 1},]"#,
                r#"[{"id": 1}] x"#,
                "[{\"id\": 1, \"name\": \"a\u{1}b\"}]",
                r#"[{"id": 1, "amount": 1e999}]"#,
                r#"[{"id": 1.5}]"#,
                r#"[{"id": 1, "flag": "true"}]"#,
            ] {
                let (fast, general) = both(input);
                assert!(fast.is_none(), "the fast path read {input:?}");
                // Whatever the general path makes of it, it still makes the same: no panic.
                let _ = general;
            }
        }

        #[test]
        fn load_json_gives_the_general_error_for_what_the_fast_path_declines() {
            let error = load_json(br#"[{"id": 1, "extra": 2}]"#, &schema()).unwrap_err();
            assert_eq!(error.to_string(), "JSON row 0, field extra: unknown field");
        }

        fn number() -> impl Strategy<Value = String> {
            prop_oneof![
                any::<i64>().prop_map(|v| v.to_string()),
                any::<u64>().prop_map(|v| v.to_string()),
                any::<f64>()
                    .prop_filter("finite", |v| v.is_finite())
                    .prop_map(|v| format!("{v:?}")),
                any::<f64>()
                    .prop_filter("finite", |v| v.is_finite())
                    .prop_map(|v| format!("{v:e}")),
                Just("-0".to_owned()),
                Just("-0.0".to_owned()),
                Just("0".to_owned()),
                Just("1E5".to_owned()),
                Just("1.0".to_owned()),
                Just("00".to_owned()),
                Just("1.".to_owned()),
                Just("-".to_owned()),
                Just("99999999999999999999999".to_owned()),
            ]
        }

        fn text_value() -> impl Strategy<Value = String> {
            prop_oneof![
                "[a-zA-Z0-9 äöüß€😀]{0,12}".prop_map(|s| format!("\"{s}\"")),
                Just("\" a \"".to_owned()),
                Just("\"  \"".to_owned()),
                Just("\"\"".to_owned()),
                Just("\"ä \"".to_owned()),
                Just(r#""a\"b""#.to_owned()),
                Just(r#""\u00e9""#.to_owned()),
                Just("\"\t\"".to_owned()),
                Just("null".to_owned()),
                Just("1".to_owned()),
                Just(r#""2026-03-05""#.to_owned()),
                Just(r#""2026-13-45""#.to_owned()),
                Just("true".to_owned()),
                Just("[]".to_owned()),
            ]
        }

        fn row() -> impl Strategy<Value = String> {
            let cell = |key: &'static str, value: BoxedStrategy<String>| {
                (Just(key), value, 0u8..10)
                    .prop_map(|(k, v, drop)| (drop != 0).then(|| format!("\"{k}\": {v}")))
            };
            (
                cell("id", number().boxed()),
                cell("name", text_value().boxed()),
                cell("amount", number().boxed()),
                cell(
                    "flag",
                    prop_oneof![
                        Just("true".to_owned()),
                        Just("false".to_owned()),
                        Just("null".to_owned()),
                        Just("1".to_owned())
                    ]
                    .boxed(),
                ),
                cell("day", text_value().boxed()),
                prop_oneof![Just(""), Just(", \"extra\": 1"), Just(", \"id\": 2")],
                prop_oneof![Just(" "), Just("\n"), Just("")],
            )
                .prop_map(|(a, b, c, d, e, tail, space)| {
                    let cells: Vec<String> = [a, b, c, d, e].into_iter().flatten().collect();
                    format!(
                        "{{{space}{}{tail}{space}}}",
                        cells.join(&format!(",{space}"))
                    )
                })
        }

        /// A row that is plain in every respect, with the awkward-but-valid spellings: keys in any
        /// order, any number form, text with spaces at its edges and non-ASCII characters, nulls.
        fn valid_row() -> impl Strategy<Value = String> {
            let numbers = prop_oneof![
                any::<i64>().prop_map(|v| v.to_string()),
                (0..=i64::MAX).prop_map(|v| v.to_string()),
                any::<f64>()
                    .prop_filter("finite", |v| v.is_finite())
                    .prop_map(|v| format!("{v:?}")),
                any::<f64>()
                    .prop_filter("finite", |v| v.is_finite())
                    .prop_map(|v| format!("{v:e}")),
                Just("-0".to_owned()),
                Just("-0.0".to_owned()),
                Just("0".to_owned()),
                Just("1E5".to_owned()),
                Just("1.0".to_owned()),
                Just("18446744073709551615".to_owned()),
                Just("-9223372036854775808".to_owned()),
            ];
            let texts = prop_oneof![
                "[a-zA-Z0-9 äöüß€😀]{0,12}".prop_map(|s| format!("\"{s}\"")),
                Just("\" a \"".to_owned()),
                Just("\"  \"".to_owned()),
                Just("\"\"".to_owned()),
                Just("null".to_owned()),
            ];
            let days = prop_oneof![
                Just("\"2026-03-05\"".to_owned()),
                Just("null".to_owned()),
                Just("\"1970-01-01\"".to_owned())
            ];
            let flags = prop_oneof![
                Just("true".to_owned()),
                Just("false".to_owned()),
                Just("null".to_owned())
            ];
            (
                (0..=i64::MAX).prop_map(|v| v.to_string()),
                texts,
                numbers,
                flags,
                days,
                0usize..5,
                prop_oneof![Just(" "), Just("\n"), Just("")],
            )
                .prop_map(|(id, name, amount, flag, day, shift, space)| {
                    let mut cells = [
                        format!("\"id\": {id}"),
                        format!("\"name\": {name}"),
                        format!("\"amount\": {amount}"),
                        format!("\"flag\": {flag}"),
                        format!("\"day\": {day}"),
                    ];
                    cells.rotate_left(shift);
                    format!("{{{space}{}{space}}}", cells.join(&format!(",{space}")))
                })
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(600))]

            /// What is plain is read by the fast path — a path that always declined would pass
            /// every other test — and read as the general path reads it, to the bit.
            #[test]
            fn plain_rows_are_read_fast_and_the_same(rows in prop::collection::vec(valid_row(), 1..8)) {
                let input = format!("[{}]", rows.join(", "));
                let (fast, general) = both(&input);
                let fast = fast.expect("a plain text must take the fast path");
                let general = general.expect("a plain text is valid");
                prop_assert!(same(&fast, &general), "different tables for {input}");
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(400))]

            /// The fast path is the general path, or declines: never a different table, never a
            /// table where the general path refuses.
            #[test]
            fn a_fast_table_is_the_general_table(rows in prop::collection::vec(row(), 1..6)) {
                let input = format!("[{}]", rows.join(", "));
                let (fast, general) = both(&input);
                if let Some(fast) = fast {
                    let general = general.expect("the fast path read what the general path refuses");
                    prop_assert!(same(&fast, &general), "different tables for {input}");
                }
            }
        }
    }
}
