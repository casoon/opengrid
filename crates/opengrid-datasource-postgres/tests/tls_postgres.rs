//! TLS to a real database (issue #220).
//!
//! Needs a PostgreSQL server that speaks TLS, and skips itself without one:
//!
//! * `OPENGRID_TEST_PG_TLS`: its connection string (`host=localhost ...`; the
//!   host name has to be one the server certificate names),
//! * `OPENGRID_TEST_PG_TLS_ROOT`: the PEM file of the authority that signed it.
//!
//! The CI job `postgres` starts such a server. The first test, which refuses a
//! downgrade, needs only the ordinary database of the other tests.

#![cfg(feature = "tls")]

mod pg;

use std::time::{Duration, Instant};

use opengrid_datasource::SendDataSource;
use opengrid_datasource_postgres::{PoolOptions, PostgresDataSource, rustls_connector};
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema};

const FAST: &str = "og_test_tls_fast";
const FAST_PLAIN: &str = "og_test_tls_plain";
const SLOW: &str = "og_test_tls_slow";
const NAME: &str = "og_tls_test";

fn schema() -> Schema {
    Schema::new(vec![Field::required(
        FieldName::new("id").unwrap(),
        DataType::Int64,
    )])
}

fn query(source: &str) -> opengrid_query::ValidatedQuery {
    let query: Query = opengrid_json::from_str(&format!(
        r#"{{"source":"{source}","select":["id"],"sort":[{{"field":"id"}}]}}"#
    ))
    .expect("a query");
    query
        .validate(&schema(), &Limits::default())
        .expect("valid")
}

fn tls_server() -> Option<(String, Vec<u8>)> {
    let url = std::env::var("OPENGRID_TEST_PG_TLS").ok()?;
    let root = std::env::var("OPENGRID_TEST_PG_TLS_ROOT").ok()?;
    let root = std::fs::read(&root).unwrap_or_else(|error| panic!("{root}: {error}"));
    Some((url, root))
}

/// A server that does not answer TLS, or whose certificate is not trusted, ends
/// in an error. It must never end in a connection in clear text.
#[tokio::test]
async fn there_is_no_fallback_to_clear_text() {
    let url = pg::connection().expect("a connection string");
    // The view exists, so a connection in clear text WOULD answer: the test
    // fails if the source ever falls back to one.
    let Ok((client, connection)) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await
    else {
        eprintln!("skipping the PostgreSQL TLS test: no database");
        return;
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(&format!(
            "DROP VIEW IF EXISTS {FAST_PLAIN};
             CREATE VIEW {FAST_PLAIN} AS SELECT g::bigint AS id FROM generate_series(1, 3) g;"
        ))
        .await
        .expect("the view");

    let tls = rustls_connector(None).expect("a connector");
    let source = PostgresDataSource::connect_with_tls(
        &url,
        FAST_PLAIN,
        schema(),
        PoolOptions::default(),
        tls,
    )
    .expect("a source");
    let answered = source.execute(query(FAST_PLAIN)).await;
    client
        .batch_execute(&format!("DROP VIEW IF EXISTS {FAST_PLAIN}"))
        .await
        .expect("drop the view");
    assert!(
        answered.is_err(),
        "a connection that cannot be encrypted must fail, not run: {answered:?}"
    );
}

#[tokio::test]
async fn a_query_and_a_cancel_run_over_tls() {
    let Some((url, root)) = tls_server() else {
        eprintln!("skipping the PostgreSQL TLS test: OPENGRID_TEST_PG_TLS is not set");
        return;
    };
    let tls = rustls_connector(Some(&root)).expect("a connector");
    let url = format!("{url} application_name={NAME}");

    let (client, connection) = tokio_postgres::connect(&url, tls.clone())
        .await
        .expect("the test connection over TLS");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(&format!(
            "DROP VIEW IF EXISTS {FAST}; DROP VIEW IF EXISTS {SLOW};
             CREATE VIEW {FAST} AS SELECT g::bigint AS id FROM generate_series(1, 3) g;
             CREATE VIEW {SLOW} AS SELECT 1::bigint AS id, pg_sleep(30)::text AS v;"
        ))
        .await
        .expect("the views");

    // A query runs, and every connection the pool made is encrypted.
    let fast = PostgresDataSource::connect_with_tls(
        &url,
        FAST,
        schema(),
        PoolOptions::default(),
        tls.clone(),
    )
    .expect("a source");
    let result = fast.execute(query(FAST)).await.expect("an answer over TLS");
    assert_eq!(result.row_count(), 3);
    let row = client
        .query_one(
            "SELECT count(*)::bigint, count(*) FILTER (WHERE s.ssl)::bigint \
             FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid) \
             WHERE a.application_name = $1 AND a.pid <> pg_backend_pid()",
            &[&NAME],
        )
        .await
        .expect("pg_stat_ssl");
    let (all, encrypted): (i64, i64) = (row.get(0), row.get(1));
    assert!(all > 0, "the source holds a connection");
    assert_eq!(all, encrypted, "every connection of the source is TLS");

    // A request that is dropped still ends its statement.
    let slow_schema = Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("v").unwrap(), DataType::Utf8),
    ]);
    let slow = PostgresDataSource::connect_with_tls(
        &url,
        SLOW,
        slow_schema.clone(),
        PoolOptions::default(),
        tls.clone(),
    )
    .expect("a source");
    let slow_query: Query =
        opengrid_json::from_str(&format!(r#"{{"source":"{SLOW}","select":["id","v"]}}"#))
            .expect("a query");
    let slow_query = slow_query
        .validate(&slow_schema, &Limits::default())
        .expect("valid");
    let answered = tokio::time::timeout(Duration::from_millis(500), slow.execute(slow_query)).await;
    assert!(answered.is_err(), "the request is dropped while it sleeps");
    let started = Instant::now();
    let running = loop {
        let row = client
            .query_one(
                &format!(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE state = 'active' AND pid <> pg_backend_pid() \
                       AND query LIKE '%{SLOW}%'"
                ),
                &[],
            )
            .await
            .expect("pg_stat_activity");
        let running: i64 = row.get(0);
        if running == 0 || started.elapsed() > Duration::from_secs(8) {
            break running;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    client
        .batch_execute(&format!(
            "DROP VIEW IF EXISTS {FAST}; DROP VIEW IF EXISTS {SLOW};"
        ))
        .await
        .expect("drop the views");
    assert_eq!(running, 0, "the statement still runs in the database");
}

/// The server certificate is verified: a private authority that the connector
/// was not given is not trusted.
#[tokio::test]
async fn a_certificate_from_an_unknown_authority_is_refused() {
    let Some((url, _root)) = tls_server() else {
        eprintln!("skipping the PostgreSQL TLS test: OPENGRID_TEST_PG_TLS is not set");
        return;
    };
    let tls = rustls_connector(None).expect("a connector");
    let source =
        PostgresDataSource::connect_with_tls(&url, FAST, schema(), PoolOptions::default(), tls)
            .expect("a source");
    let answered = source.execute(query(FAST)).await;
    assert!(answered.is_err(), "the unknown authority must be refused");
}
