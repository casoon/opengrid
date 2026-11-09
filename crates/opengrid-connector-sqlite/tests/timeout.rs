//! A request that ends ends its query (issue #217).
//!
//! The server drops a query's future when its timeout runs out or the client
//! leaves. SQLite used to answer on the calling thread, to the end: the timeout
//! could not fire before the call returned, and one slow query held the async
//! worker and every other request. Now the query runs on the connection's
//! thread and dropping the future interrupts it.

use std::time::{Duration, Instant};

use opengrid_connector::Connector;
use opengrid_connector_sqlite::{Connection, SqliteConnector};
use opengrid_query::{Limits, Query};
use opengrid_types::{DataType, Field, FieldName, Schema};

fn schema() -> Schema {
    Schema::new(vec![Field::required(
        FieldName::new("id").unwrap(),
        DataType::Int64,
    )])
}

fn query(json: &str) -> opengrid_query::ValidatedQuery {
    let query: Query = opengrid_json::from_str(json).expect("a query");
    query
        .validate(&schema(), &Limits::default())
        .expect("valid")
}

#[tokio::test]
async fn a_dropped_request_interrupts_its_query_and_frees_the_database() {
    let path = std::env::temp_dir().join(format!("opengrid-timeout-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // `slow` counts to `knob.n`: two billion steps, minutes of work — as long
    // as nobody ends it.
    let writer = Connection::open(&path).expect("a database");
    writer
        .execute_batch(
            "CREATE TABLE knob (n INTEGER);
             INSERT INTO knob VALUES (2000000000);
             CREATE VIEW slow AS
               WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < (SELECT n FROM knob))
               SELECT x AS id FROM c;",
        )
        .expect("the view");
    let connector = SqliteConnector::open(&path, "slow", schema()).expect("a connector");

    let started = Instant::now();
    let slow =
        query(r#"{"source":"slow","select":["id"],"sort":[{"field":"id","direction":"desc"}]}"#);
    let answered = tokio::time::timeout(Duration::from_millis(200), connector.execute(slow)).await;
    assert!(answered.is_err(), "the timeout fires while SQLite works");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the caller was not held: {:?}",
        started.elapsed()
    );

    // The statement is gone, not merely forgotten: a writer gets the database
    // (the reader's lock would keep it out) — within a moment.
    writer
        .busy_timeout(Duration::from_secs(5))
        .expect("a busy timeout");
    writer
        .execute("UPDATE knob SET n = 10", [])
        .expect("the interrupted query let go of the database");

    // And the connector answers the next request.
    let fast =
        query(r#"{"source":"slow","select":["id"],"sort":[{"field":"id","direction":"asc"}]}"#);
    let result = tokio::time::timeout(Duration::from_secs(10), connector.execute(fast))
        .await
        .expect("answers in time")
        .expect("an answer");
    assert_eq!(result.row_count(), 10);

    drop(connector);
    let _ = std::fs::remove_file(&path);
}
