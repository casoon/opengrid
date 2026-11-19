//! The conformance suite against a real MySQL 8 (issue #501): the same cases
//! and expectations as for the engine, PostgreSQL and SQLite, through the
//! connector.

mod my;

use opengrid_conformance::{Report, block_on};
use opengrid_connector::{AsSource, Connector, LocalConnector};
use opengrid_connector_mysql::MySqlConnector;

/// The cases that read the fixture's one NaN, which a `DOUBLE` cannot hold
/// (it goes in as NULL). Every other case has to agree.
const READS_THE_NAN: &[&str] = &[
    "s7-nan-is-not-null-in-comparisons",
    "s7-nan-sorts-after-numbers",
    "s7-nan-sorts-before-numbers-descending",
    "p11-a-nan-total-is-not-a-null-total",
];

/// The report, without the failures of the cases named in `excused`.
fn without(report: Report, excused: &[&str]) -> Report {
    let failures = report
        .failures
        .into_iter()
        .filter(|failure| !excused.iter().any(|id| failure.starts_with(id)))
        .collect();
    Report { failures, ..report }
}

async fn connector(table: &str, schema_file: &str, csv: &str) -> Option<MySqlConnector> {
    let url = my::url()?;
    let schema = my::schema_of(schema_file);
    let mut connection = my::connect(&url).await;
    my::create(&mut connection, &schema, table, csv).await;
    Some(MySqlConnector::connect(&url, table, schema).expect("a connector"))
}

#[tokio::test]
async fn mysql_answers_every_case() {
    let Some(mysql) = connector("og_conformance_orders", "orders.schema.json", "orders.csv").await
    else {
        return;
    };
    let report = without(
        opengrid_conformance::check_source(&AsSource(&mysql)).await,
        READS_THE_NAN,
    );
    println!("conformance (MySQL): {report}");
    report.assert_ok();
    assert!(report.cases >= 50, "{report}");
}

/// A list is a JSON array: `MEMBER OF`, `JSON_CONTAINS` and `JSON_LENGTH`
/// answer `has`, `has_all` and `is_empty` as the engine does (L1–L6).
#[tokio::test]
async fn mysql_answers_every_list_case() {
    let Some(mysql) = connector("og_conformance_lists", "lists.schema.json", "lists.csv").await
    else {
        return;
    };
    let report = opengrid_conformance::check_lists(&AsSource(&mysql)).await;
    println!("list conformance (MySQL): {report}");
    report.assert_ok();
    assert!(report.cases >= 20, "{report}");
}

/// `icontains` and `istarts_with` are the engine's over MySQL's rows (S5a).
#[tokio::test]
async fn mysql_folds_case_like_the_engine() {
    let Some(mysql) = connector("og_conformance_words", "words.schema.json", "words.csv").await
    else {
        return;
    };
    let report = opengrid_conformance::check_words(&AsSource(&mysql)).await;
    println!("case-folding conformance (MySQL): {report}");
    report.assert_ok();
    assert!(report.cases >= 10, "{report}");
}

/// A sort by a locale (S4a): MySQL's collation where it has one, ties by bytes.
#[tokio::test]
async fn mysql_sorts_by_a_locale_like_the_engine() {
    let Some(mysql) = connector(
        "og_conformance_collation",
        "collation.schema.json",
        "collation.csv",
    )
    .await
    else {
        return;
    };
    let report = opengrid_conformance::check_collation(&AsSource(&mysql)).await;
    println!("collation conformance (MySQL): {report}");
    report.assert_ok();
    assert!(report.cases >= 18, "{report}");
}

/// The pivot cases, level by level, against the engine, cell by cell.
#[tokio::test]
async fn mysql_answers_every_pivot_case_like_the_engine() {
    use opengrid_conformance::{FLOAT_TOLERANCE, fixture_csv, fixture_schema, suite_dir};
    use opengrid_pivot::{PivotLimits, PivotQuery};
    use opengrid_types::Value;

    let Some(mysql) = connector("og_conformance_pivot", "orders.schema.json", "orders.csv").await
    else {
        return;
    };
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
    for path in &paths {
        let text = std::fs::read_to_string(path).expect("read case");
        let case: opengrid_json::Json = opengrid_json::from_str(&text).expect("JSON");
        let id = case["id"].as_str().expect("an id");
        if READS_THE_NAN.contains(&id) {
            continue;
        }
        let pivot: PivotQuery = opengrid_json::FromJson::from_json(&case["pivot"])
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        let validated = pivot
            .validate(
                &schema,
                &PivotLimits::default(),
                &opengrid_query::Limits::default(),
            )
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        let expected = block_on(engine.pivot(&validated)).expect("the engine answers");
        let actual = mysql
            .pivot(&validated)
            .await
            .unwrap_or_else(|error| panic!("{id}: MySQL: {error}"));
        if actual.columns != expected.columns || actual.row_levels != expected.row_levels {
            differences.push(format!("{id}: the columns or the levels differ"));
            continue;
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
                        "{id}, row {row}, column {column}: engine {theirs:?}, MySQL {ours:?}"
                    ));
                }
            }
        }
    }
    println!(
        "pivot conformance (MySQL against the engine): {} cases, {} differences",
        paths.len(),
        differences.len()
    );
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert!(paths.len() >= 20, "the suite shrank");
}
