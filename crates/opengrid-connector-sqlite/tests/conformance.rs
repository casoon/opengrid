//! The whole conformance suite against SQLite (issue #51): the fixture in an
//! in-memory database, every case through the connector.

use opengrid_conformance::{block_on, check_source, fixture_csv, fixture_schema};
use opengrid_connector::{AsSource, QueryResult};
use opengrid_connector_sqlite::{Connection, SqliteConnector, create_table, insert};
use opengrid_engine::ingest::{CsvOptions, load_csv};

fn connector() -> SqliteConnector {
    let schema = fixture_schema();
    let stored = schema.stored();
    let csv = std::fs::read(fixture_csv()).expect("the fixture");
    let table = load_csv(&csv, &stored, CsvOptions::default()).expect("the rows");
    let rows = QueryResult::new(stored.clone(), table.to_values(), 0);

    let mut connection = Connection::open_in_memory().expect("an in-memory database");
    create_table(&connection, "orders", &schema).expect("the table");
    insert(&mut connection, "orders", &schema, &rows).expect("the rows go in");
    SqliteConnector::new(connection, "orders", schema).expect("a connector")
}

#[test]
fn sqlite_answers_every_case() {
    let connector = connector();
    let report = block_on(check_source(&AsSource(&connector)));
    println!("conformance (SQLite {}): {report}", rusqlite_version());
    report.assert_ok();
    assert!(report.cases >= 40, "{report}");
}

fn rusqlite_version() -> String {
    Connection::open_in_memory()
        .and_then(|connection| {
            connection.query_row("select sqlite_version()", [], |row| row.get(0))
        })
        .unwrap_or_default()
}

/// Beyond the suite: the queries where SQLite's own habits differ most from
/// the rules — NaN in sums, averages and extremes, NaN and -0.0 as groups,
/// NaN in a sort, a decimal average, derived columns — against the engine.
#[test]
fn sqlite_answers_like_the_engine_where_it_differs_most() {
    use opengrid_conformance::{RowOrder, Table, compare};
    use opengrid_connector::{Connector, LocalConnector};

    let sqlite = connector();
    let engine = LocalConnector::from_csv(
        &fixture_csv(),
        &opengrid_conformance::suite_dir().join("data/orders.schema.json"),
    )
    .expect("the engine");
    let schema = fixture_schema();
    let queries = [
        r#"{"source":"orders","aggregate":[{"field":"ratio","fn":"sum","as":"s"},{"field":"ratio","fn":"avg","as":"a"},{"field":"ratio","fn":"max","as":"hi"},{"field":"ratio","fn":"min","as":"lo"}]}"#,
        r#"{"source":"orders","select":["ratio"],"group":["ratio"],"aggregate":[{"fn":"count","as":"n"}],"sort":[{"field":"ratio","direction":"desc","nulls":"first"}]}"#,
        r#"{"source":"orders","select":["id","ratio"],"sort":[{"field":"ratio","direction":"asc"},{"field":"id"}]}"#,
        r#"{"source":"orders","select":["country"],"group":["country"],"aggregate":[{"field":"amount","fn":"avg","as":"a"},{"field":"amount","fn":"sum","as":"s"}],"sort":[{"field":"country"}]}"#,
        r#"{"source":"orders","select":["id"],"filter":{"field":"ratio","op":"gt","value":4.0},"sort":[{"field":"id"}]}"#,
        r#"{"source":"orders","select":["ordered_year"],"group":["ordered_year"],"aggregate":[{"fn":"count","as":"n"}],"sort":[{"field":"ordered_year"}]}"#,
    ];
    for json in queries {
        let query: opengrid_query::Query = serde_json::from_str(json).unwrap();
        let query = query
            .validate(&schema, &opengrid_query::Limits::default())
            .unwrap_or_else(|error| panic!("{json}: {error}"));
        let expected = block_on(engine.execute(query.clone())).expect("the engine answers");
        let actual = block_on(sqlite.execute(query)).expect("SQLite answers");
        assert_eq!(actual.total_count, expected.total_count, "{json}");
        compare(
            &Table::from(&expected),
            &Table::from(&actual),
            RowOrder::Ordered,
        )
        .unwrap_or_else(|difference| panic!("{json}: {difference}"));
    }
}
