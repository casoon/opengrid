//! Runs the conformance suite over the fixture dataset.
//!
//! Point 05 has no engine yet, so this is **validation mode**: every case must
//! load, its query must validate against the dataset schema, and its expected
//! table must be readable against the query's output types. The two checks that
//! would compare an engine's result against the expectation arrive with the
//! engines in points 07, 08 and 26 — the comparison itself is already
//! implemented and unit-tested in `opengrid_conformance::compare`.

use std::path::PathBuf;

use opengrid_conformance::{
    RowOrder, Table, block_on, check_dir, compare, load_schema, rules_covered,
};
use opengrid_datasource::{DataSource, DataSourceCapabilities, DataSourceError, QueryResult};
use opengrid_query::ValidatedQuery;
use opengrid_types::Schema;

/// The rules the specification lists in 02-query-modell.md §Semantik-Regeln.
const RULES: [&str; 14] = [
    "S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8", "S9", "S10", "S11", "S12", "S13", "S14",
];

/// `\N` marks NULL, the way PostgreSQL's `COPY` writes it.
const NULL: &str = r"\N";

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn every_case_validates_and_is_typed() {
    let schema = load_schema(&crate_dir().join("data/orders.schema.json")).expect("schema");
    let checked = check_dir(&crate_dir().join("cases"), &schema).expect("cases");

    assert!(
        checked.len() >= 40,
        "point 05 asks for at least 40 cases, found {}",
        checked.len()
    );

    let rules = rules_covered(&checked);
    for rule in RULES {
        assert!(rules.contains(rule), "no case covers rule {rule}");
    }

    let ordered = checked.iter().filter(|case| case.case.ordered).count();
    assert!(
        ordered >= 15,
        "expected at least 15 cases that pin row order down, found {ordered}"
    );

    for case in &checked {
        if case.case.ordered {
            assert!(
                !case.query.sort.is_empty(),
                "{}: `ordered` needs a `sort` key",
                case.case.id
            );
        }
        // In validation mode the expectation is the only table available; it must
        // survive the comparison against itself without an engine in between.
        compare(&case.expected, &case.expected, RowOrder::Ordered)
            .unwrap_or_else(|mismatch| panic!("{}: {mismatch}", case.case.id));
    }
}

#[test]
fn dataset_matches_the_schema_and_carries_the_special_values() {
    let schema = load_schema(&crate_dir().join("data/orders.schema.json")).expect("schema");
    let text = std::fs::read_to_string(crate_dir().join("data/orders.csv")).expect("orders.csv");
    let table = parse_csv(&text);

    // The file holds the **stored** columns; `ordered_year` and its two siblings
    // are computed while reading it (plan point 54) and must not be in here.
    let stored = schema.stored();
    let header = &table[0];
    let expected: Vec<&str> = stored
        .fields()
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(header, &expected, "CSV header and schema disagree");
    assert!(schema.has_derived() && stored.len() < schema.len());

    let rows = &table[1..];
    assert_eq!(rows.len(), 50, "the fixture is 50 hand-built rows");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            row.len(),
            header.len(),
            "CSV line {} has {} fields, expected {}",
            i + 2,
            row.len(),
            header.len()
        );
    }

    // "NULL in jeder Spalte" (point 05, step 2): every nullable column has one.
    for (column, field) in stored.fields().iter().enumerate() {
        if !field.nullable {
            continue;
        }
        assert!(
            rows.iter().any(|row| row[column] == NULL),
            "column {} carries no NULL",
            field.name
        );
    }
    assert!(
        rows.iter().all(|row| row[0] != NULL),
        "`id` is not nullable, but a row has no id"
    );

    let values = |name: &str| -> Vec<&str> {
        let column = header
            .iter()
            .position(|field| field == name)
            .expect("column present");
        rows.iter().map(|row| row[column].as_str()).collect()
    };

    // The special values point 05 asks the dataset to contain.
    let note = values("note");
    for needle in ["", "\u{00e4}", "z", "Z", "\u{00e9}", "e\u{0301}"] {
        assert!(note.contains(&needle), "no row has note {needle:?}");
    }
    assert_eq!(
        note.iter().filter(|value| **value == "e\u{0301}").count(),
        1,
        "the NFD and NFC strings must each appear exactly once (rule S13)"
    );

    let ratio = values("ratio");
    for needle in ["NaN", "0.0", "-0.0"] {
        assert!(ratio.contains(&needle), "no row has ratio {needle}");
    }

    let amount = values("amount");
    for needle in ["999999999.99", "-12345.67"] {
        assert!(amount.contains(&needle), "no row has amount {needle}");
    }

    let ordered_on = values("ordered_on");
    for needle in ["2025-12-31", "2026-01-01"] {
        assert!(ordered_on.contains(&needle), "no row is on {needle}");
    }

    let created_at = values("created_at");
    assert!(
        created_at.iter().any(|value| value.ends_with(".500000Z")),
        "no timestamp has a microsecond fraction"
    );
}

#[test]
fn the_engine_docking_point_is_usable() {
    /// A stand-in for the engines of points 07, 08 and 26.
    ///
    /// It implements the **base** trait, the browser variant without `Send` — the
    /// server variant is generated from it (E5), and a local engine implements the
    /// generated one. The stub carries no data of its own, so its answers are empty
    /// on purpose: what this test pins down is that the docking point can be
    /// implemented and driven, and that an empty answer never passes a non-empty
    /// expectation.
    struct Stub;

    impl DataSource for Stub {
        async fn schema(&self) -> Result<Schema, DataSourceError> {
            Ok(Schema::default())
        }

        async fn execute(&self, query: ValidatedQuery) -> Result<QueryResult, DataSourceError> {
            let schema = query.output_schema.clone();
            let columns = schema.fields().iter().map(|_| Vec::new()).collect();
            Ok(QueryResult::new(schema, columns, 0))
        }

        fn capabilities(&self) -> DataSourceCapabilities {
            DataSourceCapabilities::ALL
        }
    }

    let schema = load_schema(&crate_dir().join("data/orders.schema.json")).expect("schema");
    let checked = check_dir(&crate_dir().join("cases"), &schema).expect("cases");
    let engine = Stub;

    assert_eq!(
        engine.capabilities(),
        DataSourceCapabilities::ALL,
        "the stub answers everything, the suite does not consult capabilities yet"
    );

    for case in checked.iter().take(5) {
        let result = block_on(engine.execute(case.query.clone())).expect("stub never fails");
        let table = Table::from(&result);
        assert_eq!(table.columns, case.expected.columns, "{}", case.case.id);
        // An empty result does not match a non-empty expectation — the comparison
        // is what catches a wrong engine, and it must say so.
        if !case.expected.rows.is_empty() {
            assert!(
                compare(&case.expected, &table, RowOrder::Ordered).is_err(),
                "{}: an empty result must not pass",
                case.case.id
            );
        }
    }
}

/// Splits the fixture CSV.
///
/// The fixture deliberately keeps every field free of separators, quotes and line
/// breaks, so a plain split is enough here — this is a fixture integrity check,
/// not a general CSV reader. `arrow-csv` reads the file for real in point 06.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split(',').map(str::to_owned).collect())
        .collect()
}
