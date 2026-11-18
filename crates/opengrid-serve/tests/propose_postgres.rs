//! The proposal for a fresh PostgreSQL schema (issue #486). Skips itself
//! without a database (`OPENGRID_TEST_PG`), like the other PostgreSQL tests.

use opengrid_serve::read_proposal;

const SCHEMA: &str = "opengrid_serve_test";
const EXPECTED: &str = include_str!("expected/postgres.json");

fn url() -> String {
    std::env::var("OPENGRID_TEST_PG")
        .unwrap_or_else(|_| "host=localhost dbname=postgres".to_owned())
}

#[tokio::test]
async fn the_proposal_for_a_postgres_schema_is_the_expected_file() {
    let (client, connection) = match tokio_postgres::connect(&url(), tokio_postgres::NoTls).await {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("skipping the PostgreSQL proposal test: {error}");
            return;
        }
    };
    tokio::spawn(connection);
    client
        .batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {SCHEMA} CASCADE; CREATE SCHEMA {SCHEMA};
             CREATE TABLE {SCHEMA}.orders (
                id bigint PRIMARY KEY,
                customer text,
                country char(2) NOT NULL,
                qty integer,
                ratio double precision,
                paid boolean,
                amount numeric(12,2),
                ordered_on date,
                created_at timestamptz,
                tags text[],
                counts bigint[],
                payload jsonb,
                wall timestamp
             );"
        ))
        .await
        .expect("fixture");

    let proposal = read_proposal(&url(), "OPENGRID_DATABASE_URL", SCHEMA).await;
    client
        .batch_execute(&format!("DROP SCHEMA {SCHEMA} CASCADE"))
        .await
        .expect("cleanup");
    let (text, skipped) = proposal.expect("a proposal");

    assert!(skipped.is_empty());
    assert_eq!(text, EXPECTED);
}
