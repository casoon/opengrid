//! `execute_columns` answers what `execute` answers, in the binary form
//! (issue #38): the whole conformance suite, compared cell for cell.

use std::path::PathBuf;

use opengrid_conformance::{RowOrder, Table, check_dir, compare, load_schema};
use opengrid_wasm::Engine;

const ORDERS_CSV: &str = include_str!("../../opengrid-conformance/data/orders.csv");
const ORDERS_SCHEMA: &str = include_str!("../../opengrid-conformance/data/orders.schema.json");

#[test]
fn both_forms_answer_the_whole_suite_the_same() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance");
    let schema = load_schema(&root.join("data/orders.schema.json")).unwrap();
    let cases = check_dir(&root.join("cases"), &schema).expect("the cases load");
    let mut engine = Engine::new();
    engine
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .expect("the dataset loads");

    for case in &cases {
        let query = opengrid_json::to_string(&case.case.query);
        let values = engine.execute_result(&query).expect("the engine answers");
        let bytes = engine.execute_columns(&query).expect("the engine answers");
        let (table, total_count) = opengrid_columns::wire::decode_result(&bytes).unwrap();
        let binary = opengrid_datasource::QueryResult::new(
            table.schema().clone(),
            table.to_values(),
            total_count,
        );
        assert_eq!(binary.total_count, values.total_count, "{}", case.case.id);
        assert_eq!(
            binary.schema,
            values.schema.materialized(),
            "{}",
            case.case.id
        );
        compare(
            &Table::from(&values),
            &Table::from(&binary),
            RowOrder::Ordered,
        )
        .unwrap_or_else(|mismatch| panic!("{}: {mismatch}", case.case.id));
    }
    assert!(cases.len() >= 48);
}

/// A query the engine refuses is refused the same way in both forms.
#[test]
fn an_error_is_the_same_error() {
    let mut engine = Engine::new();
    engine
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .unwrap();
    let query = r#"{"source":"orders","select":["nope"]}"#;
    assert_eq!(
        engine.execute_result(query).unwrap_err(),
        engine.execute_columns(query).unwrap_err()
    );
}

const ORDERS_JSON: &str = include_str!("../../opengrid-conformance/data/orders.json");

/// Rows the page holds answer the whole suite as the same data through CSV do
/// (issue #182).
#[test]
fn rows_answer_the_suite_as_csv_does() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance");
    let schema = load_schema(&root.join("data/orders.schema.json")).unwrap();
    let cases = check_dir(&root.join("cases"), &schema).expect("the cases load");
    let mut from_csv = Engine::new();
    from_csv
        .load_csv("orders", ORDERS_CSV.as_bytes(), ORDERS_SCHEMA)
        .unwrap();
    let mut from_rows = Engine::new();
    from_rows
        .load_rows("orders", ORDERS_JSON, ORDERS_SCHEMA)
        .unwrap();

    for case in &cases {
        let query = opengrid_json::to_string(&case.case.query);
        let csv = from_csv.execute_result(&query).unwrap();
        let rows = from_rows.execute_result(&query).unwrap();
        compare(&Table::from(&csv), &Table::from(&rows), RowOrder::Ordered)
            .unwrap_or_else(|mismatch| panic!("{}: {mismatch}", case.case.id));
    }
}

/// Loading under a name replaces; a refused load leaves the old source.
#[test]
fn rows_replace_a_source_all_or_nothing() {
    let schema = r#"{"fields":[{"name":"id","type":"int64","nullable":false}]}"#;
    let mut engine = Engine::new();
    engine
        .load_rows("t", r#"[{"id":1},{"id":2}]"#, schema)
        .unwrap();
    let count = r#"{"source":"t","select":["id"]}"#;
    assert_eq!(engine.execute_result(count).unwrap().total_count, 2);

    let error = engine
        .load_rows("t", r#"[{"id":1},{"id":"x"}]"#, schema)
        .unwrap_err();
    assert!(error.contains("row 1") && error.contains("id"), "{error}");
    assert_eq!(engine.execute_result(count).unwrap().total_count, 2);

    engine.load_rows("t", r#"[{"id":7}]"#, schema).unwrap();
    assert_eq!(engine.execute_result(count).unwrap().total_count, 1);
}

/// A window that starts after the last row asks for nothing and gets nothing —
/// the grid draws the rows the reader added there (issue #183).
#[test]
fn a_window_past_the_end_is_an_empty_page_with_the_count() {
    let schema = r#"{"fields":[{"name":"id","type":"int64","nullable":false}]}"#;
    let mut engine = Engine::new();
    engine
        .load_rows("t", r#"[{"id":1},{"id":2}]"#, schema)
        .unwrap();
    let result = engine
        .execute_result(
            r#"{"source":"t","select":["id"],"sort":[{"field":"id","direction":"asc"}],"offset":5,"limit":10}"#,
        )
        .unwrap();
    assert_eq!(result.total_count, 2);
    assert!(result.columns.iter().all(Vec::is_empty));
}
