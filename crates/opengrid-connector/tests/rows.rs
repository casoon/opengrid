//! The rows tier (issue #46): the whole conformance suite through a source
//! that only hands out rows.
//!
//! The source ignores the filter it is handed — the filter is a hint — and
//! hands out the fixture in pieces of seven rows, so the engine has to put the
//! table together from several pieces and apply every filter itself.

use std::path::PathBuf;

use opengrid_conformance::{RowOrder, Table, block_on, check_dir, compare, load_schema};
use opengrid_connector::{
    BoxFuture, Connector, DataSourceError, QueryResult, RowSource, RowStream, Rows, Schema, Value,
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
        let schema = load_schema(&data().join("data/orders.schema.json")).expect("schema");
        let stored = schema.stored();
        let csv = std::fs::read(data().join("data/orders.csv")).expect("orders.csv");
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

#[test]
fn a_rows_only_source_answers_the_whole_suite() {
    let fixture = Fixture::new();
    let schema = fixture.schema.clone();
    let connector = Rows::new(fixture);
    let checked = check_dir(&data().join("cases"), &schema).expect("the cases load");

    let mut failed = Vec::new();
    for case in &checked {
        let order = if case.case.ordered {
            RowOrder::Ordered
        } else {
            RowOrder::Unordered
        };
        match block_on(connector.execute(case.query.clone())) {
            Ok(result) => {
                if let Err(mismatch) = compare(&case.expected, &Table::from(&result), order) {
                    failed.push(format!("{}: {mismatch}", case.case.id));
                }
            }
            Err(error) => failed.push(format!("{}: {error}", case.case.id)),
        }
    }
    println!(
        "conformance (rows tier): {} cases, {} failed",
        checked.len(),
        failed.len()
    );
    assert!(failed.is_empty(), "{}", failed.join("\n"));
    assert!(checked.len() >= 40);
}

/// More rows than the bound is an error that says so, not a server out of
/// memory — and the bound counts rows read, so it stops while reading.
#[test]
fn more_rows_than_the_bound_is_a_limit_error() {
    let fixture = Fixture::new();
    let schema = fixture.schema.clone();
    let connector = Rows::new(fixture).max_scan_rows(20);
    let query = check_dir(&data().join("cases"), &schema).expect("the cases")[0]
        .query
        .clone();
    match block_on(connector.execute(query)) {
        Err(DataSourceError::LimitExceeded { message }) => {
            assert!(message.contains("20"), "{message}");
        }
        other => panic!("expected a limit error, got {other:?}"),
    }

    let fits = Rows::new(Fixture::new()).max_scan_rows(50);
    let query = check_dir(&data().join("cases"), &schema).expect("the cases")[0]
        .query
        .clone();
    assert!(
        block_on(fits.execute(query)).is_ok(),
        "50 rows fit a bound of 50"
    );
}
