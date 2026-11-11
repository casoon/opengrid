//! The case-folding cases (issue #323, rule S5a): `crates/opengrid-conformance/fold-cases/`
//! over `words.csv`, answered by the local engine — the reference the connectors
//! are compared against.

use opengrid_conformance::{block_on, check_words, words_csv, words_schema};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};

#[test]
fn the_local_engine_answers_every_case_folding_case() {
    let csv = std::fs::read(words_csv()).expect("the words dataset");
    let source = LocalDataSource::new(
        load_csv(&csv, &words_schema(), CsvOptions::default()).expect("ingest"),
    );
    let report = block_on(check_words(&source));
    println!("{report}");
    report.assert_ok();
    assert!(report.cases >= 10, "{report}");
}
