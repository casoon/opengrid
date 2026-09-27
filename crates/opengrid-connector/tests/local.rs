//! The local engine's reference connector against the whole suite (issue #47),
//! through the same runner a connector author uses.

use opengrid_conformance::{block_on, check_source, fixture_csv};
use opengrid_connector::AsSource;
use opengrid_connector::LocalConnector;

#[test]
fn the_local_connector_answers_every_case() {
    let schema = opengrid_conformance::suite_dir().join("data/orders.schema.json");
    let connector = LocalConnector::from_csv(&fixture_csv(), &schema).expect("the fixture");
    let report = block_on(check_source(&AsSource(&connector)));
    report.assert_ok();
    assert!(report.cases >= 40, "{report}");
}
