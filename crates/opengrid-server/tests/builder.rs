//! The server as a library (issue #45): a connector the application writes
//! itself, handed in through the builder.
//!
//! The connector here is deliberately not one of ours: it holds rows in
//! memory, answers through the local engine and writes down every query it is
//! asked. That record is the point — it shows what a connector gets to see,
//! which is only what the server already allowed.

use std::path::Path;
use std::sync::{Arc, Mutex};

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
use opengrid_query::ValidatedFilter;
use opengrid_server::{RowFilter, Server, SourcePolicy};
use tower::ServiceExt;

const TOKEN: &str = "builder-token";

/// The conformance fixture behind an application's own connector.
struct Recording {
    inner: FromSource<LocalDataSource>,
    asked: Arc<Mutex<Vec<ValidatedQuery>>>,
}

impl Recording {
    fn new() -> (Self, Arc<Mutex<Vec<ValidatedQuery>>>) {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../opengrid-conformance/data");
        let schema: Schema = opengrid_json::from_str(
            &std::fs::read_to_string(data.join("orders.schema.json")).unwrap(),
        )
        .unwrap();
        let csv = std::fs::read(data.join("orders.csv")).unwrap();
        let table = load_csv(&csv, &schema, CsvOptions::default()).unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner: FromSource(LocalDataSource::new(table)),
                asked: Arc::clone(&asked),
            },
            asked,
        )
    }
}

impl Connector for Recording {
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
        self.asked.lock().unwrap().push(query.clone());
        self.inner.execute(query)
    }
}

async fn post(app: axum::Router, path: &str, body: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from(body.to_owned()))
        .unwrap();
    let response = app.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Whether `filter` holds `country = DE` anywhere.
fn names_the_tenant(filter: &ValidatedFilter) -> bool {
    match filter {
        ValidatedFilter::And(parts) | ValidatedFilter::Or(parts) => {
            parts.iter().any(names_the_tenant)
        }
        ValidatedFilter::Cmp { field, value, .. } => {
            field.as_str() == "country" && format!("{value:?}").contains("DE")
        }
        _ => false,
    }
}

/// **The connector never sees a query the server did not already restrict**:
/// the mandatory row filter is in every query it gets, the rows that come back
/// are the tenant's, and a column outside the allowed fields is refused before
/// the connector is asked at all.
#[tokio::test]
async fn a_connector_only_sees_what_the_server_allowed() {
    let (connector, asked) = Recording::new();
    let server = Server::builder()
        .source(
            "orders",
            connector,
            SourcePolicy {
                allowed_fields: vec!["id".into(), "customer".into(), "amount".into()],
                row_filter: Some(RowFilter::new("country", "eq", ":tenant")),
            },
        )
        .token(TOKEN, [("tenant", "DE")])
        .build()
        .await
        .expect("a server");

    let (status, body) = post(
        server.router(),
        "/query/orders",
        r#"{"source":"orders","select":["id","customer"],"sort":[{"field":"id","direction":"asc"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = result_from_json(&body).expect("a result");
    // The fixture has 50 rows; the tenant's are fewer.
    assert!(
        result.total_count > 0 && result.total_count < 50,
        "{}",
        result.total_count
    );

    let first = asked
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the connector was asked");
    let filter = first
        .filter
        .as_ref()
        .expect("the row filter is in the query");
    assert!(names_the_tenant(filter), "{filter:?}");

    let before = asked.lock().unwrap().len();
    let (status, _) = post(
        server.router(),
        "/query/orders",
        r#"{"source":"orders","select":["country"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        asked.lock().unwrap().len(),
        before,
        "refused before the connector"
    );
}

/// A connector that implements only the three required methods gets the pivot
/// and the export from the contract's defaults.
#[tokio::test]
async fn pivot_and_export_come_from_the_defaults() {
    let (connector, _) = Recording::new();
    let server = Server::builder()
        .source("orders", connector, SourcePolicy::default())
        .token(TOKEN, [("unused", "")])
        .build()
        .await
        .expect("a server");

    let (status, body) = post(
        server.router(),
        "/pivot/orders",
        r#"{"source":"orders","rows":["country"],"columns":[],"values":[{"field":"qty","fn":"sum","as":"total"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = post(
        server.router(),
        "/export/orders?format=csv&bom=false",
        r#"{"source":"orders","select":["id"],"sort":[{"field":"id","direction":"asc"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // A header line and the fixture's 50 rows.
    assert_eq!(body.lines().count(), 51, "{body}");
}

/// Everything that can be wrong with what was handed in is a build error.
#[tokio::test]
async fn a_wrong_description_does_not_build() {
    let empty = Server::builder().build().await;
    assert!(empty.is_err(), "no sources");

    let (one, _) = Recording::new();
    let (two, _) = Recording::new();
    let twice = Server::builder()
        .source("orders", one, SourcePolicy::default())
        .source("orders", two, SourcePolicy::default())
        .build()
        .await;
    assert!(twice.is_err(), "a name twice");

    let (connector, _) = Recording::new();
    let unknown = Server::builder()
        .source(
            "orders",
            connector,
            SourcePolicy {
                allowed_fields: vec!["nope".into()],
                row_filter: None,
            },
        )
        .build()
        .await;
    let message = unknown
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("nope"), "{message}");
}

/// A source of the rows tier (issue #46): only schema and rows. Its bound
/// reaches the client as `limit_exceeded` (413), a query it can narrow.
#[tokio::test]
async fn a_rows_source_over_its_bound_is_a_413() {
    use opengrid_connector::{RowSource, RowStream, Rows};
    use opengrid_query::ValidatedFilter;

    struct Numbers;
    impl RowSource for Numbers {
        fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
            Box::pin(async {
                Ok(opengrid_json::from_str(
                    r#"{"fields":[{"name":"n","type":"int64","nullable":false}]}"#,
                )
                .unwrap())
            })
        }
        fn scan<'a>(
            &'a self,
            _filter: Option<&'a ValidatedFilter>,
        ) -> BoxFuture<'a, Result<Box<dyn RowStream + 'a>, DataSourceError>> {
            Box::pin(async {
                let stream: Box<dyn RowStream + 'a> = Box::new(Once(false));
                Ok(stream)
            })
        }
    }
    struct Once(bool);
    impl RowStream for Once {
        fn next_piece(&mut self) -> BoxFuture<'_, Result<Option<QueryResult>, DataSourceError>> {
            let done = std::mem::replace(&mut self.0, true);
            Box::pin(async move {
                if done {
                    return Ok(None);
                }
                let schema: Schema = opengrid_json::from_str(
                    r#"{"fields":[{"name":"n","type":"int64","nullable":false}]}"#,
                )
                .unwrap();
                let values = (0..100).map(opengrid_types::Value::Int64).collect();
                Ok(Some(QueryResult::new(schema, vec![values], 0)))
            })
        }
    }

    let server = Server::builder()
        .source(
            "numbers",
            Rows::new(Numbers).max_scan_rows(10),
            SourcePolicy::default(),
        )
        .token(TOKEN, [("unused", "")])
        .build()
        .await
        .expect("a server");
    let (status, body) = post(
        server.router(),
        "/query/numbers",
        r#"{"source":"numbers","select":["n"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(body.contains("max_scan_rows"), "{body}");
}

/// A rows source whose pieces are always ready — endless here — still meets
/// the server's timeout (issue #50): the scan gives the runtime its turn
/// between pieces, so the timeout can fire instead of the request running on.
#[tokio::test(flavor = "multi_thread")]
async fn an_endless_ready_source_meets_the_timeout() {
    use opengrid_connector::{RowSource, RowStream, Rows};
    use opengrid_query::ValidatedFilter;

    fn schema() -> Schema {
        opengrid_json::from_str(r#"{"fields":[{"name":"n","type":"int64","nullable":false}]}"#)
            .unwrap()
    }
    struct Endless;
    impl RowSource for Endless {
        fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
            Box::pin(async { Ok(schema()) })
        }
        fn scan<'a>(
            &'a self,
            _filter: Option<&'a ValidatedFilter>,
        ) -> BoxFuture<'a, Result<Box<dyn RowStream + 'a>, DataSourceError>> {
            Box::pin(async {
                let stream: Box<dyn RowStream + 'a> = Box::new(Forever);
                Ok(stream)
            })
        }
    }
    struct Forever;
    impl RowStream for Forever {
        fn next_piece(&mut self) -> BoxFuture<'_, Result<Option<QueryResult>, DataSourceError>> {
            Box::pin(async {
                let values = (0..10_000).map(opengrid_types::Value::Int64).collect();
                Ok(Some(QueryResult::new(schema(), vec![values], 0)))
            })
        }
    }

    let server = Server::builder()
        .timeout(std::time::Duration::from_millis(200))
        .source("endless", Rows::new(Endless), SourcePolicy::default())
        .token(TOKEN, [("unused", "")])
        .build()
        .await
        .expect("a server");
    let asked = post(
        server.router(),
        "/query/endless",
        r#"{"source":"endless","aggregate":[{"fn":"count","as":"n_rows"}]}"#,
    );
    let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(10), asked)
        .await
        .expect("the server answers instead of scanning on");
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(body.contains("took longer"), "{body}");
}
