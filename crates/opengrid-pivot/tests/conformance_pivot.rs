//! The pivot rules P1–P8 as executable cases (plan points 30 and 52).
//!
//! The cases live next to the dataset they are written against
//! (`crates/opengrid-conformance/pivot-cases/`), and they are run here against
//! the local engine. Point 31 runs the very same files against PostgreSQL —
//! that comparison is the question MVP D asks.
//!
//! A case names a pivot and the whole answer: the generated columns with the
//! values they stand for, every row including its subtotals, and the level of
//! each row. Nothing is summarised; a wrong subtotal has nowhere to hide.

use std::path::{Path, PathBuf};

use opengrid_conformance::{block_on, load_schema};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_json::{Error, FromJson, Json};
use opengrid_pivot::{PivotLimits, PivotQuery, PivotResult, execute};
use opengrid_query::Limits;
use opengrid_types::{Schema, Value};

struct PivotCase {
    id: String,
    rule: String,
    pivot: PivotQuery,
    expected: Expected,
}

struct Expected {
    columns: Vec<ExpectedColumn>,
    rows: Vec<Vec<Json>>,
    levels: Vec<u16>,
}

struct ExpectedColumn {
    path: Vec<Json>,
    measure: String,
}

/// `json[key]` read as a `T`, naming the key when it is not one.
fn read<T: FromJson>(json: &Json, key: &str) -> Result<T, Error> {
    T::from_json(&json[key]).map_err(|error| Error::new(format!("{key}: {error}")))
}

impl FromJson for PivotCase {
    fn from_json(json: &Json) -> Result<Self, Error> {
        Ok(PivotCase {
            id: read(json, "id")?,
            rule: read(json, "rule")?,
            pivot: read(json, "pivot")?,
            expected: read(json, "expected")?,
        })
    }
}

impl FromJson for Expected {
    fn from_json(json: &Json) -> Result<Self, Error> {
        Ok(Expected {
            columns: read(json, "columns")?,
            rows: read(json, "rows")?,
            levels: read(json, "levels")?,
        })
    }
}

impl FromJson for ExpectedColumn {
    fn from_json(json: &Json) -> Result<Self, Error> {
        Ok(ExpectedColumn {
            path: read(json, "path")?,
            measure: read(json, "measure")?,
        })
    }
}

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("opengrid-conformance")
}

fn schema() -> Schema {
    load_schema(&suite_dir().join("data/orders.schema.json")).expect("the dataset schema")
}

fn source() -> LocalDataSource {
    let csv = std::fs::read(suite_dir().join("data/orders.csv")).expect("the dataset");
    let table = load_csv(&csv, &schema(), CsvOptions::default()).expect("ingest");
    LocalDataSource::new(table)
}

fn cases() -> Vec<PivotCase> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(suite_dir().join("pivot-cases"))
        .expect("the pivot cases")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read case");
            opengrid_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        })
        .collect()
}

/// A value as the case file writes it. Floats compare with tolerance (S12:
/// `avg` is Float64), everything else exactly — a decimal is a string and stays
/// one.
fn matches(actual: &Value, expected: &opengrid_json::Json) -> bool {
    match (actual, expected) {
        (Value::Null, opengrid_json::Json::Null) => true,
        (Value::Float64(number), opengrid_json::Json::Number(other)) => {
            let other = other.as_f64().expect("every number is an f64");
            (number - other).abs() <= 1e-9 * other.abs().max(1.0)
        }
        _ => opengrid_json::ToJson::to_json(actual) == *expected,
    }
}

/// Every difference between what a case expects and what came out.
fn differences(case: &PivotCase, result: &PivotResult) -> Vec<String> {
    let mut problems = Vec::new();

    let columns: Vec<String> = result
        .columns
        .iter()
        .map(|column| {
            format!(
                "{}({})",
                column.measure,
                column
                    .path
                    .iter()
                    .map(opengrid_json::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    let expected_columns: Vec<String> = case
        .expected
        .columns
        .iter()
        .map(|column| {
            format!(
                "{}({})",
                column.measure,
                column
                    .path
                    .iter()
                    .map(opengrid_json::Json::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    if columns != expected_columns {
        problems.push(format!(
            "columns: expected {expected_columns:?}, got {columns:?}"
        ));
        return problems;
    }

    if result.row_levels != case.expected.levels {
        problems.push(format!(
            "levels: expected {:?}, got {:?}",
            case.expected.levels, result.row_levels
        ));
    }
    if result.row_count() != case.expected.rows.len() {
        problems.push(format!(
            "rows: expected {}, got {}",
            case.expected.rows.len(),
            result.row_count()
        ));
        return problems;
    }

    for (index, expected) in case.expected.rows.iter().enumerate() {
        for (column, cell) in expected.iter().enumerate() {
            let actual = &result.data.columns[column][index];
            if !matches(actual, cell) {
                problems.push(format!(
                    "row {index}, column {column}: expected {cell}, got {actual:?}"
                ));
            }
        }
    }
    problems
}

/// **The point of point 52:** every rule, answered by the local engine.
#[test]
fn the_local_engine_answers_every_pivot_case() {
    let schema = schema();
    let source = source();
    let cases = cases();
    assert!(cases.len() >= 5, "the pivot suite is smaller than expected");

    let mut failures = Vec::new();
    let mut rules: Vec<String> = Vec::new();
    for case in &cases {
        if !rules.contains(&case.rule) {
            rules.push(case.rule.clone());
        }
        let validated = case
            .pivot
            .validate(&schema, &PivotLimits::default(), &Limits::default())
            .unwrap_or_else(|error| panic!("{}: {error}", case.id));
        match block_on(execute(&source, &validated)) {
            Ok(result) => {
                for problem in differences(case, &result) {
                    failures.push(format!("{}: {problem}", case.id));
                }
            }
            Err(error) => failures.push(format!("{}: {error}", case.id)),
        }
    }

    println!(
        "pivot conformance (local engine): {} cases, rules {}, {} failures",
        cases.len(),
        rules.join("/"),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// One query per level and not one more — the number point 31 drives to 1.
#[test]
fn a_pivot_costs_one_query_per_level() {
    let schema = schema();
    let pivot: PivotQuery = opengrid_json::from_str(
        r#"{"source":"orders","rows":["country","customer"],"columns":["ordered_year"],
            "values":[{"fn":"count","as":"n"}]}"#,
    )
    .expect("a pivot");
    let validated = pivot
        .validate(&schema, &PivotLimits::default(), &Limits::default())
        .expect("valid");

    assert_eq!(
        validated.sets.len(),
        3,
        "two row dimensions make three levels"
    );
    // Deepest first, and every level carries the column dimension.
    let groups: Vec<Vec<&str>> = validated
        .sets
        .iter()
        .map(|set| set.group.iter().map(|field| field.as_str()).collect())
        .collect();
    assert_eq!(
        groups,
        vec![
            vec!["country", "customer", "ordered_year"],
            vec!["country", "ordered_year"],
            vec!["ordered_year"],
        ]
    );
    // Every level is sorted by its own keys — that is why the pivot engine never
    // compares two values to decide an order.
    assert!(
        validated
            .sets
            .iter()
            .all(|set| set.sort.len() == set.group.len())
    );
}

/// P3: the columns come from the data, in the order rules S3/S4 give — computed
/// by whoever answered, never sorted here.
#[test]
fn the_columns_are_the_values_that_occur_in_the_order_they_belong() {
    let pivot: PivotQuery = opengrid_json::from_str(
        r#"{"source":"orders","rows":["country"],"columns":["ordered_year"],
            "values":[{"fn":"count","as":"n"}]}"#,
    )
    .expect("a pivot");
    let validated = pivot
        .validate(&schema(), &PivotLimits::default(), &Limits::default())
        .expect("valid");
    let result = block_on(execute(&source(), &validated)).expect("the pivot runs");

    let paths: Vec<Vec<Value>> = result
        .columns
        .iter()
        .map(|column| column.path.clone())
        .collect();
    assert_eq!(
        paths,
        vec![
            vec![Value::Int64(2025)],
            vec![Value::Int64(2026)],
            vec![Value::Null],
        ],
        "ascending, NULL last (S3) — and 2027 is not a column because no row has it"
    );
}

/// P7: over a limit the pivot **fails**, and the message says what to do.
///
/// A shortened pivot shows totals that do not add up and looks complete while
/// doing it.
#[test]
fn a_pivot_that_is_too_big_is_an_error_not_a_short_answer() {
    let pivot: PivotQuery = opengrid_json::from_str(
        r#"{"source":"orders","rows":["country"],"columns":["ordered_year"],
            "values":[{"fn":"count","as":"n"}]}"#,
    )
    .expect("a pivot");

    let narrow = PivotLimits {
        max_columns: 2,
        ..PivotLimits::default()
    };
    let validated = pivot
        .validate(&schema(), &narrow, &Limits::default())
        .expect("the limit bites at execution, when the columns are known");
    let error = block_on(execute(&source(), &validated)).expect_err("three columns, two allowed");
    let message = error.to_string();
    assert!(message.contains('3') && message.contains('2'), "{message}");
    assert!(
        message.contains("narrow"),
        "the message has to say what to do: {message}"
    );

    let short = PivotLimits {
        max_rows: 3,
        ..PivotLimits::default()
    };
    let validated = pivot
        .validate(&schema(), &short, &Limits::default())
        .expect("valid");
    assert!(block_on(execute(&source(), &validated)).is_err());
}
