//! A request that ends ends its statement (issue #217).
//!
//! The server drops a query's future when its timeout runs out or the client
//! leaves. That alone leaves the statement running in the database; the source
//! sends a cancel request when a future is dropped halfway. The test uses a
//! view whose column sleeps for thirty seconds, drops the request after half a
//! second, and watches `pg_stat_activity` for the statement to go.
//!
//! Skips itself without a database, like every test in this crate.

mod pg;

use std::time::{Duration, Instant};

use opengrid_datasource::SendDataSource;
use opengrid_datasource_postgres::PostgresDataSource;
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema};
use tokio_postgres::NoTls;

const VIEW: &str = "og_test_cancel_slow";

#[tokio::test]
async fn a_dropped_request_cancels_its_statement() {
    let url = pg::connection().expect("a connection string");
    let (client, connection) = match tokio_postgres::connect(&url, NoTls).await {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("skipping the PostgreSQL cancel test: {error}");
            return;
        }
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .batch_execute(&format!(
            "DROP VIEW IF EXISTS {VIEW};
             CREATE VIEW {VIEW} AS SELECT 1::bigint AS id, pg_sleep(30)::text AS v;"
        ))
        .await
        .expect("the view");

    let schema = Schema::new(vec![
        Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        Field::new(FieldName::new("v").unwrap(), DataType::Utf8),
    ]);
    let source = PostgresDataSource::connect(&url, VIEW, schema.clone()).expect("a source");
    let query: Query =
        opengrid_json::from_str(&format!(r#"{{"source":"{VIEW}","select":["id","v"]}}"#))
            .expect("a query");
    let query = query.validate(&schema, &Limits::default()).expect("valid");

    let answered = tokio::time::timeout(Duration::from_millis(500), source.execute(query)).await;
    assert!(answered.is_err(), "the request is dropped while it sleeps");

    // The statement is gone from the database within a moment — not after the
    // thirty seconds it would have slept.
    let started = Instant::now();
    let running = loop {
        let row = client
            .query_one(
                &format!(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE state = 'active' AND pid <> pg_backend_pid() \
                       AND query LIKE '%{VIEW}%'"
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
        .batch_execute(&format!("DROP VIEW IF EXISTS {VIEW}"))
        .await
        .expect("drop the view");
    assert_eq!(running, 0, "the statement still runs in the database");
}
