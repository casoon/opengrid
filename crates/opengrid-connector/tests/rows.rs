//! The rows tier (issue #46): the whole conformance suite through a source
//! that only hands out rows.
//!
//! The source ignores the filter it is handed — the filter is a hint — and
//! hands out the fixture in pieces of seven rows, so the engine has to put the
//! table together from several pieces and apply every filter itself.

use std::path::PathBuf;

use opengrid_conformance::{block_on, load_schema};
use opengrid_connector::{
    AsSource, BoxFuture, Connector, DataSourceError, QueryResult, RowSource, RowStream, Rows,
    Schema, Value,
};
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_query::ValidatedFilter;

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance")
}

/// The fixture as plain columns — what a file or a list would hold.
struct Fixture {
    schema: Schema,
    stored: Schema,
    columns: Vec<Vec<Value>>,
}

impl Fixture {
    fn new() -> Self {
        Self::of(
            &data().join("data/orders.csv"),
            &data().join("data/orders.schema.json"),
        )
    }

    /// Any dataset of the suite — the tree's, say.
    fn of(csv: &std::path::Path, schema: &std::path::Path) -> Self {
        let schema = load_schema(schema).expect("schema");
        let stored = schema.stored();
        let csv = std::fs::read(csv).expect("the CSV");
        let columns = load_csv(&csv, &stored, CsvOptions::default())
            .expect("the fixture")
            .to_values();
        Self {
            schema,
            stored,
            columns,
        }
    }
}

impl RowSource for Fixture {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        let schema = self.schema.clone();
        Box::pin(async move { Ok(schema) })
    }

    fn scan<'a>(
        &'a self,
        _filter: Option<&'a ValidatedFilter>,
    ) -> BoxFuture<'a, Result<Box<dyn RowStream + 'a>, DataSourceError>> {
        Box::pin(async move {
            let stream: Box<dyn RowStream + 'a> = Box::new(Sevens {
                fixture: self,
                next: 0,
            });
            Ok(stream)
        })
    }
}

/// Seven rows at a time.
struct Sevens<'a> {
    fixture: &'a Fixture,
    next: usize,
}

impl RowStream for Sevens<'_> {
    fn next_piece(&mut self) -> BoxFuture<'_, Result<Option<QueryResult>, DataSourceError>> {
        Box::pin(async move {
            let rows = self.fixture.columns[0].len();
            if self.next >= rows {
                return Ok(None);
            }
            let end = (self.next + 7).min(rows);
            let columns = self
                .fixture
                .columns
                .iter()
                .map(|column| column[self.next..end].to_vec())
                .collect();
            self.next = end;
            Ok(Some(QueryResult::new(
                self.fixture.stored.clone(),
                columns,
                0,
            )))
        })
    }
}

/// The same suite with the held rows cut back after every few rows: pages
/// through top-k again and again, groups through partials merged with
/// partials — the answers must not change.
#[test]
fn cutting_back_often_changes_no_answer() {
    let connector = Rows::new(Fixture::new()).compact_at(3);
    let report = block_on(opengrid_conformance::check_source(&AsSource(&connector)));
    report.assert_ok();
}

#[test]
fn a_rows_only_source_answers_the_whole_suite() {
    let connector = Rows::new(Fixture::new());
    let report = block_on(opengrid_conformance::check_source(&AsSource(&connector)));
    println!("conformance (rows tier): {report}");
    report.assert_ok();
    assert!(report.cases >= 40);
}

/// The bound counts the rows an answer has to **hold**: every match of a
/// query without a page is over it, a page of the same rows is not — the
/// streaming path keeps only the page (issue #50).
#[test]
fn the_bound_counts_what_is_held_not_what_is_read() {
    let schema = Fixture::new().schema;
    let query = |json: &str| {
        let query: opengrid_query::Query = opengrid_json::from_str(json).unwrap();
        query
            .validate(&schema, &opengrid_query::Limits::default())
            .unwrap()
    };
    let connector = Rows::new(Fixture::new()).max_scan_rows(20);

    match block_on(connector.execute(query(r#"{"source":"orders","select":["id"]}"#))) {
        Err(DataSourceError::LimitExceeded { message }) => {
            assert!(message.contains("20"), "{message}");
        }
        other => panic!("expected a limit error, got {other:?}"),
    }

    let page = block_on(connector.execute(query(
        r#"{"source":"orders","select":["id"],"sort":[{"field":"id","direction":"desc"}],"limit":5}"#,
    )))
    .expect("a page of 5 fits a bound of 20");
    assert_eq!(page.total_count, 50, "every match is counted");
    assert_eq!(page.columns[0].first(), Some(&Value::Int64(50)));
}

/// Every tree case (E38) through the rows tier (plan point 122): the rows of
/// the tree's scope come in pieces of seven, and the engine answers the level
/// once all of them are in.
#[test]
fn a_rows_only_source_answers_every_tree_case() {
    use opengrid_conformance::{check_tree_case, tree_cases, tree_dataset};
    use opengrid_json::FromJson;
    let mut failures = Vec::new();
    for (id, case) in tree_cases() {
        let query = opengrid_query::Query::from_json(&case["query"]).expect("a query");
        let (csv, schema_path) = tree_dataset(query.source.as_str());
        let connector = Rows::new(Fixture::of(&csv, &schema_path));
        let schema = load_schema(&schema_path).expect("schema");
        let validated = query
            .validate(&schema, &opengrid_query::Limits::default())
            .expect("valid");
        let answer = block_on(connector.execute(validated)).map_err(|error| error.to_string());
        failures.extend(
            check_tree_case(&case, answer)
                .into_iter()
                .map(|problem| format!("{id}: {problem}")),
        );
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
