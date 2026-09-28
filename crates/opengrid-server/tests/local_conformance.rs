//! The local engine's reference connector against the whole suite (issue #47),
//! through the same runner a connector author uses.

use opengrid_conformance::{block_on, check_source, fixture_csv, fixture_schema};
use opengrid_connector::AsSource;
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_server::local::LocalConnector;

#[test]
fn the_local_connector_answers_every_case() {
    let csv = std::fs::read(fixture_csv()).expect("the fixture");
    let table = load_csv(&csv, &fixture_schema(), CsvOptions::default()).expect("a table");
    let connector = LocalConnector::new(LocalDataSource::new(table));
    let report = block_on(check_source(&AsSource(&connector)));
    report.assert_ok();
    assert!(report.cases >= 40, "{report}");
}
