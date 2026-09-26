//! A pivot exported as it is shown, over the pivot conformance cases (issue #3).
//!
//! Each case of `crates/opengrid-conformance/pivot-cases/` is answered by the
//! local engine and written as CSV with the element's English labels. Every
//! case has to hold the shape — one header line, one line per row the element
//! shows, as wide as the header, every data row's dimensions named, every
//! total labelled across the columns it spans. Two cases are written out in
//! full: P1 for the column dimension with its NULL value and the empty-string
//! group, P6 for subtotals next to a real NULL group.

use std::path::{Path, PathBuf};

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::ingest::{CsvOptions as IngestOptions, load_csv};
use opengrid_conformance::{block_on, load_schema};
use opengrid_export::{CsvOptions, PivotLabels, pivot_csv};
use opengrid_pivot::{PivotLimits, PivotQuery, PivotResult, execute};
use opengrid_query::Limits;
use opengrid_types::Schema;

/// The element's English defaults (`GridTexts::default`).
struct English;

impl PivotLabels for English {
    fn dimension(&self, value: Option<&str>) -> String {
        match value {
            None => "(no value)".to_owned(),
            Some("") => "(empty)".to_owned(),
            Some(text) => text.to_owned(),
        }
    }
    fn total(&self) -> String {
        "Total".to_owned()
    }
    fn subtotal(&self, value: &str) -> String {
        format!("Total {value}")
    }
}

fn suite_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("opengrid-conformance")
}

fn schema() -> Schema {
    load_schema(&suite_dir().join("data/orders.schema.json")).expect("the dataset schema")
}

fn source() -> LocalDataSource {
    let csv = std::fs::read(suite_dir().join("data/orders.csv")).expect("the dataset");
    let batches = load_csv(&csv, &schema(), IngestOptions::default()).expect("ingest");
    LocalDataSource::new(batches).expect("a local source")
}

/// Every case: its id and its pivot.
fn cases() -> Vec<(String, PivotQuery)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(suite_dir().join("pivot-cases"))
        .expect("the pivot cases")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read case");
            let case: serde_json::Value = serde_json::from_str(&text).expect("a case is JSON");
            let pivot = serde_json::from_value(case["pivot"].clone())
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            (case["id"].as_str().expect("an id").to_owned(), pivot)
        })
        .collect()
}

fn answer(pivot: &PivotQuery) -> PivotResult {
    let validated = pivot
        .validate(&schema(), &PivotLimits::default(), &Limits::default())
        .expect("a valid pivot");
    block_on(execute(&source(), &validated)).expect("the engine answers")
}

fn case(id: &str) -> PivotResult {
    let (_, pivot) = cases()
        .into_iter()
        .find(|(case, _)| case == id)
        .unwrap_or_else(|| panic!("no case {id}"));
    answer(&pivot)
}

fn plain_options() -> CsvOptions {
    CsvOptions {
        bom: false,
        ..CsvOptions::default()
    }
}

#[test]
fn every_pivot_case_is_exported_row_for_row() {
    let cases = cases();
    assert!(cases.len() >= 5, "the pivot suite is smaller than expected");
    for (id, pivot) in &cases {
        let result = answer(pivot);
        let csv = pivot_csv(&result, &English, &plain_options());
        let dimensions = pivot.rows.len();

        assert!(csv.ends_with("\r\n"), "{id}: {csv}");
        let lines: Vec<&str> = csv.trim_end_matches("\r\n").split("\r\n").collect();
        // The header, then exactly the rows the element shows.
        assert_eq!(lines.len(), result.row_count() + 1, "{id}: {csv}");
        let width = lines[0].split(',').count();
        assert_eq!(width, dimensions + result.columns.len(), "{id}: {csv}");

        for (line, level) in lines[1..].iter().zip(&result.row_levels) {
            // No value in the conformance data holds a comma or a quote.
            let cells: Vec<&str> = line.split(',').collect();
            assert_eq!(cells.len(), width, "{id}: {line}");
            let level = usize::from(*level);
            if level < dimensions {
                let label = if level == 0 { "Total" } else { "Total " };
                assert!(cells[0].starts_with(label), "{id}: {line}");
                assert!(
                    cells[1..dimensions].iter().all(|cell| cell.is_empty()),
                    "{id}: a total spans its dimension columns: {line}"
                );
            } else {
                assert!(
                    cells[..dimensions].iter().all(|cell| !cell.is_empty()),
                    "{id}: a row header is never blank: {line}"
                );
            }
        }
    }
}

/// P1: the NULL year is a column with a name, the empty country a row with
/// one, the NULL country a row that is not the total.
#[test]
fn p1_a_column_dimension_is_one_composed_header() {
    let csv = pivot_csv(
        &case("p1-a-pivot-is-rows-crossed-with-columns"),
        &English,
        &CsvOptions::default(),
    );
    assert_eq!(
        csv,
        "\u{FEFF}country,2025 · total,2026 · total,(no value) · total\r\n\
         (empty),,114,\r\n\
         DE,1,258,\r\n\
         FR,,172,5\r\n\
         GB,,195,\r\n\
         US,,210,\r\n\
         (no value),,269,\r\n\
         Total,1,1218,5\r\n"
    );
}

/// P6: a subtotal says which group it closes and spans the customer column;
/// the NULL customer inside a country is a row of its own.
#[test]
fn p6_subtotals_carry_their_label() {
    let csv = pivot_csv(
        &case("p6-a-subtotal-follows-the-rows-it-sums"),
        &English,
        &plain_options(),
    );
    assert_eq!(
        csv,
        "country,customer,n\r\n\
         DE,(empty),1\r\n\
         DE,Alpha,3\r\n\
         DE,Beta,1\r\n\
         DE,Epsilon,1\r\n\
         Total DE,,6\r\n\
         FR,Beta,1\r\n\
         FR,Gamma,1\r\n\
         Total FR,,2\r\n\
         GB,(no value),1\r\n\
         Total GB,,1\r\n\
         US,Delta,1\r\n\
         US,Gamma,1\r\n\
         Total US,,2\r\n\
         (no value),Epsilon,1\r\n\
         Total (no value),,1\r\n\
         Total,,12\r\n"
    );
}

/// P5: an average is written in the wire notation — the shortest text that
/// reads back to the same float — and an empty average is NULL, not zero.
#[test]
fn p5_values_are_in_the_wire_notation() {
    let options = CsvOptions {
        null: "\\N".to_owned(),
        ..plain_options()
    };
    let csv = pivot_csv(&case("p5-a-subtotal-is-a-real-average"), &English, &options);
    let lines: Vec<&str> = csv.split("\r\n").collect();
    assert_eq!(lines[3], "DE,Beta,\\N", "{csv}");
    assert_eq!(lines[5], "Total DE,,7.2", "{csv}");
    assert_eq!(lines[16], "Total,,6.818181818181818", "{csv}");
}
