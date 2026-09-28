//! The codec against `serde_json`, the oracle it replaces.

use proptest::prelude::*;

use super::*;

/// A `serde_json` document as ours, keeping serde's integer/float split.
fn from_serde(value: &serde_json::Value) -> Json {
    match value {
        serde_json::Value::Null => Json::Null,
        serde_json::Value::Bool(flag) => Json::Bool(*flag),
        serde_json::Value::Number(number) => Json::Number(if let Some(value) = number.as_u64() {
            Number::PosInt(value)
        } else if let Some(value) = number.as_i64() {
            Number::NegInt(value)
        } else {
            Number::Float(number.as_f64().unwrap())
        }),
        serde_json::Value::String(text) => Json::String(text.clone()),
        serde_json::Value::Array(items) => Json::Array(items.iter().map(from_serde).collect()),
        serde_json::Value::Object(map) => Json::object(
            map.iter()
                .map(|(key, value)| (key.clone(), from_serde(value))),
        ),
    }
}

fn documents() -> impl Strategy<Value = serde_json::Value> {
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::Bool),
        any::<i64>().prop_map(serde_json::Value::from),
        any::<u64>().prop_map(serde_json::Value::from),
        any::<f64>()
            .prop_filter("finite", |value| value.is_finite())
            .prop_map(serde_json::Value::from),
        ".*".prop_map(serde_json::Value::String),
        "[\\x00-\\x1f\"\\\\/\u{7f}\u{e9}\u{1F600}]{0,6}".prop_map(serde_json::Value::String),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..6).prop_map(serde_json::Value::Array),
            proptest::collection::btree_map("[a-z\"\\\\]{0,4}", inner, 0..6)
                .prop_map(|map| serde_json::Value::Object(map.into_iter().collect())),
        ]
    })
}

/// Our writer, with floats compared as numbers: where `serde_json` picks the
/// other of two shortest spellings, both read back to the same `f64`.
fn same_document(ours: &str, theirs: &str) -> bool {
    ours == theirs
        || serde_json::from_str::<serde_json::Value>(ours).unwrap()
            == serde_json::from_str::<serde_json::Value>(theirs).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// What `serde_json` writes, we read to the same document.
    #[test]
    fn reads_what_serde_writes(document in documents()) {
        let text = serde_json::to_string(&document).unwrap();
        prop_assert_eq!(Json::parse(&text).unwrap(), from_serde(&document));
        let pretty = serde_json::to_string_pretty(&document).unwrap();
        prop_assert_eq!(Json::parse(&pretty).unwrap(), from_serde(&document));
    }

    /// What we write, `serde_json` would have written — byte for byte but for
    /// the choice between two shortest float spellings.
    #[test]
    fn writes_what_serde_writes(document in documents()) {
        let theirs = serde_json::to_string(&document).unwrap();
        let ours = from_serde(&document).to_string();
        prop_assert!(same_document(&ours, &theirs), "{} vs {}", ours, theirs);
    }

    /// Text `serde_json` refuses, we refuse; text it reads, we read.
    #[test]
    fn refuses_what_serde_refuses(text in "[\\[\\]{}\",:0-9.eE+\\-a-z \\\\u]{0,12}") {
        let theirs = serde_json::from_str::<serde_json::Value>(&text);
        let ours = Json::parse(&text);
        prop_assert_eq!(ours.is_ok(), theirs.is_ok(), "{:?}: ours {:?}", text, ours);
        if let (Ok(ours), Ok(theirs)) = (ours, theirs) {
            prop_assert_eq!(ours, from_serde(&theirs));
        }
    }

    /// Every finite float, in `serde_json`'s layout, reads back to itself.
    #[test]
    fn floats_read_back(value in any::<f64>().prop_filter("finite", |value| value.is_finite())) {
        let ours = write_f64(value);
        prop_assert_eq!(ours.parse::<f64>().unwrap().to_bits(), value.to_bits());
        let theirs = serde_json::to_string(&value).unwrap();
        prop_assert_eq!(ours.contains('e'), theirs.contains('e'), "{} vs {}", ours, theirs);
        prop_assert_eq!(ours.len(), theirs.len(), "{} vs {}", ours, theirs);
    }
}

#[test]
fn edge_cases_read_as_serde_reads_them() {
    for text in [
        "0",
        "-0",
        "-0.0",
        "1e400",
        "-1e400",
        "18446744073709551615",
        "18446744073709551616",
        "-9223372036854775808",
        "-9223372036854775809",
        "1E2",
        "1e-2",
        "\"\\ud83d\\ude00\"",
        "\"\\ud83d\"",
        "\"\\ude00\"",
        "\"\\u00e9\"",
        "\"a\\/b\"",
        "01",
        "1.",
        ".5",
        "+1",
        "[1,]",
        "{\"a\":1,}",
        "{\"a\" 1}",
        "[1 2]",
        "nul",
        "truex",
        "\"\t\"",
        "",
        " ",
        "{}",
        "[]",
        "{\"a\":{\"b\":[null,true,false]}}",
        "\"\\x\"",
        "\"\\u12\"",
    ] {
        let theirs = serde_json::from_str::<serde_json::Value>(text);
        let ours = Json::parse(text);
        assert_eq!(ours.is_ok(), theirs.is_ok(), "{text:?}: ours {ours:?}");
        if let (Ok(ours), Ok(theirs)) = (ours, theirs) {
            assert_eq!(ours, from_serde(&theirs), "{text:?}");
        }
    }
}

#[test]
fn nesting_is_bounded() {
    let deep = format!("{}{}", "[".repeat(128), "]".repeat(128));
    assert!(Json::parse(&deep).is_ok());
    let deeper = format!("{}{}", "[".repeat(129), "]".repeat(129));
    assert!(
        Json::parse(&deeper)
            .unwrap_err()
            .message()
            .starts_with("recursion limit exceeded")
    );
}

#[test]
fn an_error_says_where() {
    let error = Json::parse("{\"a\":\n  tru }").unwrap_err();
    assert_eq!(error.message(), "expected ident at line 2 column 6");
    assert_eq!(
        Json::parse("[1] x").unwrap_err().message(),
        "trailing characters at line 1 column 5"
    );
}

#[test]
fn fields_refuse_unknown_and_duplicate_keys() {
    let known = ["source", "select"];
    let json = Json::parse(r#"{"source":"a","columns":1}"#).unwrap();
    assert_eq!(
        Fields::of(&json, "a query", &known)
            .err()
            .unwrap()
            .message(),
        "unknown field `columns`, expected one of `source`, `select`"
    );
    let json = Json::parse(r#"{"source":"a","source":"b"}"#).unwrap();
    assert_eq!(
        Fields::of(&json, "a query", &known)
            .err()
            .unwrap()
            .message(),
        "duplicate field `source`"
    );
    let json = Json::parse(r#""x""#).unwrap();
    assert_eq!(
        Fields::of(&json, "struct Query", &known)
            .err()
            .unwrap()
            .message(),
        "invalid type: string \"x\", expected struct Query"
    );
    let json = Json::parse(r#"{"select":null}"#).unwrap();
    let fields = Fields::of(&json, "a query", &known).unwrap();
    assert_eq!(
        fields.required("source").unwrap_err().message(),
        "missing field `source`"
    );
    assert!(fields.optional("select").is_none(), "null reads as absent");
    assert!(fields.get("select").is_some());
}

#[test]
fn integers_are_checked_against_their_type() {
    assert_eq!(u8::from_json(&Json::from(255u64)).unwrap(), 255);
    assert_eq!(
        u8::from_json(&Json::from(256u64)).unwrap_err().message(),
        "invalid value: integer `256`, expected u8"
    );
    assert_eq!(
        u64::from_json(&Json::from(-1i64)).unwrap_err().message(),
        "invalid value: integer `-1`, expected u64"
    );
    assert!(u64::from_json(&Json::from(1.0)).is_err());
    assert_eq!(i64::from_json(&Json::from(-5i64)).unwrap(), -5);
}

#[test]
fn non_finite_floats_are_null() {
    assert_eq!(Json::from(f64::NAN), Json::Null);
    assert_eq!(Json::from(f64::INFINITY).to_string(), "null");
}
