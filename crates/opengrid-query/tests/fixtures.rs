//! Fixture-driven tests for the JSON contract and validation
//! (specification/02-query-modell.md).
//!
//! `tests/fixtures/valid/*.json` are queries that must parse *and* validate.
//! `tests/fixtures/invalid/*.json` wrap a query in `{ "expect": <code>, "query": … }`;
//! the query must parse but validation must fail with exactly that
//! [`QueryError::code`].

use std::fs;
use std::path::{Path, PathBuf};

use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema};

fn name(raw: &str) -> FieldName {
    FieldName::new(raw).expect("test field name")
}

/// The data source the fixtures are written against.
fn orders() -> Schema {
    Schema::new(vec![
        Field::required(name("customer"), DataType::Utf8),
        Field::required(name("country"), DataType::Utf8),
        Field::required(name("amount"), DataType::Int64),
        Field::new(name("note"), DataType::Utf8),
    ])
}

fn fixture_dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(kind)
}

fn fixtures(kind: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(kind))
        .unwrap_or_else(|error| panic!("read fixtures/{kind}: {error}"))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn valid_fixtures_parse_and_validate() {
    let paths = fixtures("valid");
    assert!(!paths.is_empty(), "no valid fixtures found");
    for path in paths {
        let json = fs::read_to_string(&path).expect("read fixture");
        let query: Query = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("{}: does not parse: {error}", path.display()));
        query
            .validate(&orders(), &Limits::default())
            .unwrap_or_else(|error| panic!("{}: does not validate: {error}", path.display()));
    }
}

#[test]
fn invalid_fixtures_fail_with_expected_code() {
    #[derive(serde::Deserialize)]
    struct Case {
        expect: String,
        query: serde_json::Value,
    }

    let paths = fixtures("invalid");
    assert!(
        paths.len() >= 15,
        "expected at least 15 invalid fixtures, found {}",
        paths.len()
    );
    for path in paths {
        let json = fs::read_to_string(&path).expect("read fixture");
        let case: Case = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("{}: bad fixture: {error}", path.display()));
        // The query itself must parse: we are testing validation, not the parser.
        let query: Query = serde_json::from_value(case.query)
            .unwrap_or_else(|error| panic!("{}: query does not parse: {error}", path.display()));
        let error = query
            .validate(&orders(), &Limits::default())
            .expect_err(&format!("{}: expected validation to fail", path.display()));
        assert_eq!(
            error.code(),
            case.expect,
            "{}: wrong error ({error})",
            path.display()
        );
    }
}

/// The JSON example in the specification must parse and validate as written.
/// Skipped when `plan/` is absent, e.g. in CI, because it is not committed.
#[test]
fn specification_example_validates_unchanged() {
    let spec =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/spezifikation/02-query-modell.md");
    let Ok(text) = fs::read_to_string(&spec) else {
        eprintln!("skipped: {} not present", spec.display());
        return;
    };
    let block = text
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("a ```json block in the query model spec");
    let query: Query = serde_json::from_str(block).expect("spec example parses");
    let validated = query
        .validate(&orders(), &Limits::default())
        .expect("spec example validates");
    assert_eq!(
        validated
            .output_schema
            .fields()
            .iter()
            .map(|field| field.name.to_string())
            .collect::<Vec<_>>(),
        ["customer", "revenue"]
    );
}

/// The AST survives a serialize/parse round-trip, i.e. the JSON we emit is the
/// JSON we accept.
#[test]
fn filter_survives_a_json_round_trip() {
    let json = fmt_fixture(&fixture_dir("valid").join("spec-example.json"));
    let query: Query = serde_json::from_str(&json).expect("parse");
    let again: Query =
        serde_json::from_str(&serde_json::to_string(&query).expect("serialize")).expect("re-parse");
    assert_eq!(query, again);
}

/// A filter object with more than one logical key is ambiguous and must not
/// silently drop one of them.
#[test]
fn filter_with_two_logical_keys_is_rejected() {
    for json in [
        r#"{"source":"orders","select":["customer"],"filter":{"and":[],"or":[]}}"#,
        r#"{"source":"orders","select":["customer"],"filter":{"or":[],"not":{"field":"note","op":"is_null"}}}"#,
    ] {
        assert!(
            serde_json::from_str::<Query>(json).is_err(),
            "accepted ambiguous filter: {json}"
        );
    }
}

fn fmt_fixture(path: &Path) -> String {
    fs::read_to_string(path).expect("read fixture")
}
