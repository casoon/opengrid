//! The list cases (issue #185, rules L1–L5): `crates/opengrid-conformance/list-cases/`
//! over `lists.csv`, answered by the local engine — the reference the
//! connectors are compared against.

use opengrid_conformance::{block_on, check_lists, lists_csv, lists_schema};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};

fn source() -> LocalDataSource {
    let csv = std::fs::read(lists_csv()).expect("the list dataset");
    LocalDataSource::new(load_csv(&csv, &lists_schema(), CsvOptions::default()).expect("ingest"))
}

#[test]
fn the_local_engine_answers_every_list_case() {
    let report = block_on(check_lists(&source()));
    println!("{report}");
    report.assert_ok();
    assert!(report.cases >= 20, "{report}");
}
