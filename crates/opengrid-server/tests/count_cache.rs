//! The count a window repeats (issue #457): asked once, remembered per source
//! and per query without its paging, and dropped when the source changes.
//!
//! The connector here does what a real one does with a known count: it takes it
//! instead of counting. So a count kept under the wrong key shows up as a wrong
//! `total_count`, not only as a missing statement.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_connector::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, FromSource, QueryResult, Schema,
    ValidatedQuery,
};
use opengrid_datasource::wire::result_from_json;
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_server::{RowFilter, Server, SourcePolicy};
use tower::ServiceExt;

struct Counting {
    inner: FromSource<LocalDataSource>,
    /// The `known_count` of every query asked, in order.
    known: Arc<Mutex<Vec<Option<u64>>>>,
}

impl Connector for Counting {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        self.inner.schema()
    }

    fn capabilities(&self) -> DataSourceCapabilities {
        self.inner.capabilities()
    }

    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        self.execute_counted(query, None)
    }

    fn execute_counted(
        &self,
        query: ValidatedQuery,
        known_count: Option<u64>,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        self.known.lock().unwrap().push(known_count);
        Box::pin(async move {
            let mut result = self.inner.execute(query).await?;
            if let Some(known) = known_count {
                result.total_count = known;
            }
            Ok(result)
        })
    }
}

async fn started(ttl: Option<Duration>) -> (Server, Arc<Mutex<Vec<Option<u64>>>>) {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance/data");
    let schema: Schema =
        opengrid_json::from_str(&std::fs::read_to_string(data.join("orders.schema.json")).unwrap())
            .unwrap();
    let csv = std::fs::read(data.join("orders.csv")).unwrap();
    let table = load_csv(&csv, &schema, CsvOptions::default()).unwrap();
    let known = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Server::builder()
        .source(
            "orders",
            Counting {
                inner: FromSource(LocalDataSource::new(table)),
                known: Arc::clone(&known),
            },
            SourcePolicy {
                allowed_fields: vec!["id".into(), "customer".into(), "country".into()],
                row_filter: Some(RowFilter::new("country", "eq", ":tenant")),
            },
        )
        .token("de", [("tenant", "DE")])
        .token("fr", [("tenant", "FR")]);
    if let Some(ttl) = ttl {
        builder = builder.count_cache_ttl(ttl);
    }
    (builder.build().await.expect("a server"), known)
}

async fn ask(server: &Server, token: &str, tail: &str) -> u64 {
    let body = format!(r#"{{"source":"orders","select":["id","customer"]{tail}}}"#);
    let request = Request::builder()
        .method("POST")
        .uri("/query/orders")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let response = server.router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    result_from_json(std::str::from_utf8(&bytes).unwrap())
        .expect("a result")
        .total_count
}

const FIRST: &str = r#","sort":[{"field":"id"}],"limit":5,"offset":0"#;
// Another page, and another sort: neither changes how many rows match.
const NEXT: &str =
    r#","sort":[{"field":"customer","direction":"desc"},{"field":"id"}],"limit":5,"offset":5"#;
const FILTERED: &str = r#","filter":{"field":"customer","op":"eq","value":"Alpha"},"sort":[{"field":"id"}],"limit":5,"offset":5"#;

/// Only `offset` changes: the second window does not count again, and says the
/// same number.
#[tokio::test]
async fn the_next_window_of_a_query_does_not_count_again() {
    let (server, known) = started(None).await;
    let first = ask(&server, "de", FIRST).await;
    let next = ask(&server, "de", NEXT).await;
    assert_eq!(first, 16);
    assert_eq!(next, first);
    assert_eq!(*known.lock().unwrap(), vec![None, Some(16)]);
}

/// A count belongs to one filter and to one reader's row filter: a hit for
/// another query or another tenant would be a wrong number, and a leak.
#[tokio::test]
async fn another_filter_or_another_tenant_counts_for_itself() {
    let (server, known) = started(None).await;
    assert_eq!(ask(&server, "de", FIRST).await, 16);
    let filtered = ask(&server, "de", FILTERED).await;
    assert!(filtered < 16, "the filter narrows: {filtered}");
    assert_eq!(ask(&server, "fr", FIRST).await, 10);
    assert_eq!(*known.lock().unwrap(), vec![None, None, None]);
}

/// The application says the source changed: the next window counts again.
#[tokio::test]
async fn a_change_drops_the_count() {
    let (server, known) = started(None).await;
    ask(&server, "de", FIRST).await;
    server.notify_changed("orders");
    ask(&server, "de", NEXT).await;
    assert_eq!(*known.lock().unwrap(), vec![None, None]);
}

/// A count lives its time and no longer; zero switches the cache off.
#[tokio::test]
async fn a_count_lives_its_time_and_zero_is_off() {
    let (server, known) = started(Some(Duration::from_millis(50))).await;
    ask(&server, "de", FIRST).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    ask(&server, "de", NEXT).await;
    assert_eq!(*known.lock().unwrap(), vec![None, None]);

    let (server, known) = started(Some(Duration::ZERO)).await;
    ask(&server, "de", FIRST).await;
    ask(&server, "de", NEXT).await;
    assert_eq!(*known.lock().unwrap(), vec![None, None]);
}
