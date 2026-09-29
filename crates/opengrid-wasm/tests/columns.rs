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
