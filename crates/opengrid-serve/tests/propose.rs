//! Reading a schema and writing the proposal (issue #486).

mod support;

use opengrid_serve::{Config, read_proposal};
use support::{Database, SQLITE_TABLES};

const EXPECTED_SQLITE: &str = include_str!("expected/sqlite.json");

#[tokio::test]
async fn the_proposal_for_a_sqlite_file_is_the_expected_file() {
    let database = Database::new("propose", SQLITE_TABLES);
    let (text, skipped) = read_proposal(&database.connection(), "OPENGRID_DATABASE_URL", "public")
        .await
        .expect("a proposal");
    assert!(skipped.is_empty());
    assert_eq!(text, EXPECTED_SQLITE);
}

#[tokio::test]
async fn a_column_with_an_unsupported_type_is_named_and_left_out() {
    let database = Database::new("left-out", SQLITE_TABLES);
    let (text, _) = read_proposal(&database.connection(), "OPENGRID_DATABASE_URL", "public")
        .await
        .expect("a proposal");
    // Named in `left_out` …
    for (column, database_type) in [("amount", "DECIMAL(12,2)"), ("payload", "BLOB")] {
        assert!(
            text.contains(&format!(
                "\"column\": \"{column}\",\n          \"database_type\": \"{database_type}\""
            )),
            "{text}"
        );
    }
    // … and in no field of the schema.
    let config = Config::parse(&text).expect("the file reads");
    let names: Vec<_> = config.sources[0]
        .schema
        .fields()
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "id",
            "customer",
            "country",
            "qty",
            "ratio",
            "paid",
            "secret_note"
        ],
        "a column with no type is no field"
    );
}

#[tokio::test]
async fn a_missing_sqlite_file_is_refused_not_created() {
    let path = std::env::temp_dir().join("opengrid-serve-does-not-exist.sqlite");
    let _ = std::fs::remove_file(&path);
    let error = read_proposal(&format!("sqlite:{}", path.display()), "X", "public")
        .await
        .expect_err("no file");
    assert!(error.to_string().contains("no SQLite file"), "{error}");
    assert!(!path.exists(), "reading must not create a database");
}
