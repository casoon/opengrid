//! The export against a real PostgreSQL (issue #2): one statement read through
//! a cursor, counted first in the same snapshot, and a connection that never
//! goes back to the pool with the export's transaction still open.
//!
//! Skips itself without a database, like the conformance run next door.

mod pg;

use std::time::{Duration, Instant};

use opengrid_datasource::{QueryResult, SendDataSource};
use opengrid_datasource_postgres::PostgresDataSource;
use opengrid_query::{Limits, Query, ValidatedQuery};
use opengrid_types::{Schema, Value};

fn validate(json: &str, schema: &Schema) -> ValidatedQuery {
    let query: Query = serde_json::from_str(json).expect("the query parses");
    query
        .validate(&schema.materialized(), &Limits::default())
        .expect("the query validates")
}

/// `url` with an `application_name`, so a test can find its own backends in
/// `pg_stat_activity` and nobody else's.
fn named(url: &str, name: &str) -> String {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        let joint = if url.contains('?') { '&' } else { '?' };
        format!("{url}{joint}application_name={name}")
    } else {
        format!("{url} application_name={name}")
    }
}

/// The states of the backends connected under `name`.
async fn backends(client: &tokio_postgres::Client, name: &str) -> Vec<String> {
    client
        .query(
            "SELECT coalesce(state, '') FROM pg_stat_activity WHERE application_name = $1",
            &[&name],
        )
        .await
        .expect("pg_stat_activity")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

/// Every piece of an export, in order, until the short one.
async fn drain(
    export: &mut opengrid_datasource_postgres::PostgresExport,
    rows: usize,
) -> (Vec<usize>, Vec<Vec<Value>>) {
    let mut sizes = Vec::new();
    let mut columns: Vec<Vec<Value>> = Vec::new();
    // Bounded: an export whose pieces never get shorter must fail, not hang.
    for _ in 0..100 {
        let piece = export.next_piece(rows).await.expect("a piece");
        if columns.is_empty() {
            columns = vec![Vec::new(); piece.columns.len()];
        }
        sizes.push(piece.row_count());
        for (column, values) in piece.columns.into_iter().enumerate() {
            columns[column].extend(values);
        }
        if sizes.last() < Some(&rows) {
            return (sizes, columns);
        }
    }
    panic!("the export did not end: {sizes:?}");
}

/// The pieces are the rows `/query` answers for the same query — the same
/// statement, only read through a cursor — and the count is theirs, window
/// included.
#[tokio::test]
async fn the_pieces_are_the_rows_the_query_answers() {
    let Some((client, url)) = pg::connect().await else {
        return;
    };
    let schema = pg::schema();
    let table = pg::create_fixture(&client, &schema, "opengrid_export_pieces").await;
    let source = PostgresDataSource::connect(&url, &table, schema.clone()).expect("a source");

    for json in [
        // Every type the data set has, NULLs and NaN included.
        r#"{"source":"orders","select":["id","customer","country","amount","qty","ratio","note","ordered_on","created_at"],"sort":[{"field":"id","direction":"asc"}]}"#,
        // A window: the count is what is left of it, not the table.
        r#"{"source":"orders","select":["id","customer"],"sort":[{"field":"customer","direction":"desc"},{"field":"id","direction":"asc"}],"offset":7,"limit":30}"#,
        // An offset past the end.
        r#"{"source":"orders","select":["id"],"sort":[{"field":"id","direction":"asc"}],"offset":80}"#,
        // Grouped: the rows are the groups.
        r#"{"source":"orders","select":["country","rows"],"group":["country"],"aggregate":[{"fn":"count","as":"rows"}],"sort":[{"field":"country","direction":"asc"}]}"#,
    ] {
        let query = validate(json, &schema);
        let answer: QueryResult = SendDataSource::execute(&source, query.clone())
            .await
            .expect("the query answers");

        let mut export = source.export(&query).await.expect("an export");
        let rows = export.count().await.expect("a count");
        assert_eq!(rows, answer.row_count() as u64, "{json}");
        let (sizes, columns) = drain(&mut export, 8).await;
        // Compared as text: `NaN` is in the data, and it is not equal to itself.
        assert!(
            format!("{columns:?}") == format!("{:?}", answer.columns),
            "the pieces differ from the answer: {json}"
        );
        assert_eq!(sizes.iter().sum::<usize>(), answer.row_count(), "{json}");
        assert!(
            sizes.iter().rev().skip(1).all(|&size| size == 8),
            "{sizes:?}"
        );
    }

    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");
}

/// A finished export gives its connection back — out of the transaction, so
/// the next query on it is an ordinary one — and an export dropped half-way
/// does not: its backend goes, and with it the transaction and the cursor.
#[tokio::test]
async fn an_export_dropped_half_way_leaves_nothing_open() {
    let Some((client, url)) = pg::connect().await else {
        return;
    };
    let schema = pg::schema();
    let table = pg::create_fixture(&client, &schema, "opengrid_export_dropped").await;
    let name = "opengrid_export_dropped_test";
    let source =
        PostgresDataSource::connect(&named(&url, name), &table, schema.clone()).expect("a source");
    let query = validate(
        r#"{"source":"orders","select":["id"],"sort":[{"field":"id","direction":"asc"}]}"#,
        &schema,
    );

    // To the end: the connection is back in the pool, idle and not in a
    // transaction, and it answers an ordinary query.
    let mut export = source.export(&query).await.expect("an export");
    export.count().await.expect("a count");
    let (sizes, _) = drain(&mut export, 20).await;
    assert_eq!(sizes, vec![20, 20, 10]);
    drop(export);
    assert_eq!(backends(&client, name).await, vec!["idle".to_owned()]);
    let answer = SendDataSource::execute(&source, query.clone())
        .await
        .expect("the pooled connection still answers");
    assert_eq!(answer.row_count(), 50);

    // Half-way: while the export is held, its transaction is open …
    let mut export = source.export(&query).await.expect("an export");
    export.count().await.expect("a count");
    assert_eq!(
        export.next_piece(20).await.expect("a piece").row_count(),
        20
    );
    assert_eq!(
        backends(&client, name).await,
        vec!["idle in transaction".to_owned()]
    );
    // … and once it is dropped, the backend is gone rather than pooled.
    drop(export);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut left = backends(&client, name).await;
    while !left.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        left = backends(&client, name).await;
    }
    assert_eq!(left, Vec::<String>::new());

    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");
}
