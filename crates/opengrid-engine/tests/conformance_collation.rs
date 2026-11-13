//! The collation cases (issue #391, rule S4a): `crates/opengrid-conformance/collation-cases/`
//! over `collation.csv`, answered by the local engine with ICU4X's sort keys.
//! The expected orders were written by PostgreSQL, so this is the engine
//! against a database, not against itself.
#![cfg(feature = "icu")]

use opengrid_conformance::{block_on, check_collation, collation_csv, collation_schema};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};

#[test]
fn the_local_engine_answers_every_collation_case() {
    let csv = std::fs::read(collation_csv()).expect("the collation dataset");
    let source = LocalDataSource::new(
        load_csv(&csv, &collation_schema(), CsvOptions::default()).expect("ingest"),
    );
    let report = block_on(check_collation(&source));
    println!("{report}");
    report.assert_ok();
    assert!(report.cases >= 18, "{report}");
}
