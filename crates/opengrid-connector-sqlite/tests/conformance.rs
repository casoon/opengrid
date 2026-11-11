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

/// The list cases (issue #185): a list is its JSON array in a TEXT column and
/// `json_each` reads the elements — `has`, `has_all` and `is_empty` answer as
/// the engine does, a NULL list unknown, the empty one empty.
#[test]
fn sqlite_answers_every_list_case() {
    use opengrid_conformance::{check_lists, lists_csv, lists_schema};

    let schema = lists_schema();
    let csv = std::fs::read(lists_csv()).expect("the list dataset");
    let table = load_csv(&csv, &schema, CsvOptions::default()).expect("the rows");
    let rows = QueryResult::new(schema.clone(), table.to_values(), 0);
    let mut connection = Connection::open_in_memory().expect("an in-memory database");
    create_table(&connection, "lists", &schema).expect("the table");
    insert(&mut connection, "lists", &schema, &rows).expect("the rows go in");
    let connector = SqliteConnector::new(connection, "lists", schema).expect("a connector");

    let report = block_on(check_lists(&AsSource(&connector)));
    println!("list conformance (SQLite): {report}");
    report.assert_ok();
    assert!(report.cases >= 20, "{report}");
}

/// The case-folding cases (issue #323, rule S5a): `og_fold` gives SQLite the
/// engine's Unicode lowercase, which its own `lower` (ASCII only) does not.
#[test]
fn sqlite_folds_case_like_the_engine() {
    use opengrid_conformance::{check_words, words_csv, words_schema};

    let schema = words_schema();
    let csv = std::fs::read(words_csv()).expect("the words dataset");
    let table = load_csv(&csv, &schema, CsvOptions::default()).expect("the rows");
    let rows = QueryResult::new(schema.clone(), table.to_values(), 0);
    let mut connection = Connection::open_in_memory().expect("an in-memory database");
    create_table(&connection, "words", &schema).expect("the table");
    insert(&mut connection, "words", &schema, &rows).expect("the rows go in");
    let connector = SqliteConnector::new(connection, "words", schema).expect("a connector");

    let report = block_on(check_words(&AsSource(&connector)));
    println!("case-folding conformance (SQLite): {report}");
    report.assert_ok();
    assert!(report.cases >= 10, "{report}");
}

/// The pivot cases (rules P1–P12): SQLite answers the grouping sets, and the
/// pivot — its subtotals, its order, a share or a running total over its
/// cells — comes out as the engine's does, cell by cell. The engine is held
/// against the expected answers by `opengrid-pivot`'s own suite.
#[test]
fn sqlite_answers_every_pivot_case_like_the_engine() {
    use opengrid_conformance::{FLOAT_TOLERANCE, suite_dir};
    use opengrid_connector::{Connector, LocalConnector};
    use opengrid_pivot::{PivotLimits, PivotQuery};
    use opengrid_types::Value;

    let sqlite = connector();
    let engine =
        LocalConnector::from_csv(&fixture_csv(), &suite_dir().join("data/orders.schema.json"))
            .expect("the engine");
    let schema = fixture_schema();
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(suite_dir().join("pivot-cases"))
        .expect("the pivot cases")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();

    let mut differences = Vec::new();
    let mut calculated = 0;
    for path in &paths {
        let text = std::fs::read_to_string(path).expect("read case");
        let case: opengrid_json::Json = opengrid_json::from_str(&text).expect("JSON");
        let id = case["id"].as_str().expect("an id");
        let pivot: PivotQuery = opengrid_json::FromJson::from_json(&case["pivot"])
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        if pivot.values.iter().any(|value| value.calc.is_some()) {
            calculated += 1;
        }
        let validated = pivot
            .validate(
                &schema,
                &PivotLimits::default(),
                &opengrid_query::Limits::default(),
            )
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        let expected = block_on(engine.pivot(&validated)).expect("the engine answers");
        let actual = block_on(sqlite.pivot(&validated))
            .unwrap_or_else(|error| panic!("{id}: SQLite: {error}"));

        if actual.columns != expected.columns || actual.row_levels != expected.row_levels {
            differences.push(format!("{id}: the columns or the levels differ"));
            continue;
        }
        let types = |result: &opengrid_connector::PivotResult| -> Vec<_> {
            let fields = result.data.schema.fields();
            fields.iter().map(|field| field.data_type).collect()
        };
        if types(&actual) != types(&expected) {
            differences.push(format!("{id}: the column types differ"));
        }
        for (column, (ours, theirs)) in actual
            .data
            .columns
            .iter()
            .zip(&expected.data.columns)
            .enumerate()
        {
            for (row, (ours, theirs)) in ours.iter().zip(theirs).enumerate() {
                let same = match (ours, theirs) {
                    (Value::Float64(a), Value::Float64(b)) => {
                        (a.is_nan() && b.is_nan())
                            || (a - b).abs() <= FLOAT_TOLERANCE * b.abs().max(1.0)
                    }
                    _ => ours == theirs,
                };
                if !same {
                    differences.push(format!(
                        "{id}, row {row}, column {column}: engine {theirs:?}, SQLite {ours:?}"
                    ));
                }
            }
        }
    }
    println!(
        "pivot conformance (SQLite against the engine): {} cases, {calculated} calculated, {} differences",
        paths.len(),
        differences.len()
    );
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert!(paths.len() >= 20 && calculated >= 10, "the suite shrank");
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
        let query: opengrid_query::Query = opengrid_json::from_str(json).unwrap();
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

/// Issue #222: a decimal is stored as a scaled 64-bit integer; one that does
/// not fit is a backend error, where it used to wrap to another number.
#[test]
fn a_decimal_too_big_for_a_64_bit_integer_is_refused() {
    use opengrid_types::{DataType, Decimal, Field, FieldName, Schema, Value};

    let schema = Schema::new(vec![Field::required(
        FieldName::new("amount").unwrap(),
        DataType::Decimal {
            precision: 38,
            scale: 2,
        },
    )]);
    let mut connection = Connection::open_in_memory().expect("an in-memory database");
    create_table(&connection, "t", &schema).expect("the table");
    let fits = QueryResult::new(
        schema.clone(),
        vec![vec![Value::Decimal(Decimal::new(12_345, 2))]],
        0,
    );
    insert(&mut connection, "t", &schema, &fits).expect("a small decimal goes in");
    // 10^20 at scale 2 is 10^22 stored: past i64.
    let big = QueryResult::new(
        schema.clone(),
        vec![vec![Value::Decimal(Decimal::new(10i128.pow(20), 0))]],
        0,
    );
    let error = insert(&mut connection, "t", &schema, &big).expect_err("does not fit");
    assert!(error.to_string().contains("64-bit"), "{error}");
}
