//! The binary result form over HTTP (issue #38, decision E35).
//!
//! `POST /query` and `POST /pivot` answer JSON unless `Accept` asks for
//! `application/vnd.opengrid.columns`. Both forms carry the same answer: the
//! **whole conformance suite** runs through the endpoint twice, once per form,
//! and every case has to come back the same both ways — and as the suite
//! expects.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_columns::wire::{MEDIA_TYPE, decode_result};
use opengrid_conformance::{RowOrder, Table, check_dir, compare, load_schema};
use opengrid_datasource::QueryResult;
use opengrid_datasource::wire::result_from_json;
use opengrid_server::{AppState, Config, Registry, router};
use tower::ServiceExt;

const TOKEN: &str = "binary-token";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/opengrid-server/../..")
        .to_path_buf()
}

/// The fixture with every column open and no row filter: the suite's queries
/// name all of them.
fn app() -> axum::Router {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);

    let root = repo_root();
    // A file per call: the tests run in parallel, and a shared path means one
    // test reads the configuration while another is still writing it.
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = root.join(format!("target/opengrid-server-binary-test-{id}.toml"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!(
            r#"
[server]
max_payload_bytes = 65536
timeout_ms = 5000

[[tokens]]
value = "{TOKEN}"

[[datasources]]
name = "orders"
type = "local-csv"
path = "crates/opengrid-conformance/data/orders.csv"
schema = "crates/opengrid-conformance/data/orders.schema.json"
"#
        ),
    )
    .unwrap();
    let config = Config::load(&path).expect("configuration");
    let registry = Registry::build(&config, &root).expect("registry");
    router(Arc::new(AppState::new(&config, registry)))
}

/// Posts `body` to `path`, with `accept` if given; the status, the content
/// type, `Vary` and the body.
async fn post(
    app: &axum::Router,
    path: &str,
    body: &str,
    accept: Option<&str>,
) -> (StatusCode, String, Option<String>, Vec<u8>) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    if let Some(accept) = accept {
        request = request.header(header::ACCEPT, accept);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .expect("the router answers");
    let status = response.status();
    let header_of = |name| {
        response
            .headers()
            .get(name)
            .map(|value: &header::HeaderValue| value.to_str().unwrap().to_owned())
    };
    let content_type = header_of(header::CONTENT_TYPE).unwrap_or_default();
    let vary = header_of(header::VARY);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, content_type, vary, bytes.to_vec())
}

fn from_bytes(bytes: &[u8]) -> QueryResult {
    let (table, total_count) = decode_result(bytes).expect("the body is the binary form");
    QueryResult::new(table.schema().clone(), table.to_values(), total_count)
}

#[tokio::test]
async fn the_whole_suite_answers_the_same_in_both_forms() {
    let root = repo_root().join("crates/opengrid-conformance");
    let schema = load_schema(&root.join("data/orders.schema.json")).unwrap();
    let cases = check_dir(&root.join("cases"), &schema).expect("the cases load");
    let app = app();

    let mut failed = Vec::new();
    for case in &cases {
        let body = serde_json::to_string(&case.case.query).unwrap();
        let (json_status, json_type, _, json) = post(&app, "/query/orders", &body, None).await;
        let (status, content_type, _, bytes) =
            post(&app, "/query/orders", &body, Some(MEDIA_TYPE)).await;
        assert_eq!(
            (json_status, status),
            (StatusCode::OK, StatusCode::OK),
            "{}",
            case.case.id
        );
        assert_eq!(json_type, "application/json");
        assert_eq!(content_type, MEDIA_TYPE);

        let from_json = result_from_json(std::str::from_utf8(&json).unwrap()).unwrap();
        let from_binary = from_bytes(&bytes);
        let order = if case.case.ordered {
            RowOrder::Ordered
        } else {
            RowOrder::Unordered
        };
        // The two forms agree cell for cell, in order — and with the suite.
        if let Err(mismatch) = compare(
            &Table::from(&from_json),
            &Table::from(&from_binary),
            RowOrder::Ordered,
        )
        .and_then(|()| compare(&case.expected, &Table::from(&from_binary), order))
        {
            failed.push(format!("{}: {mismatch}", case.case.id));
        }
        assert_eq!(from_json.total_count, from_binary.total_count);
        assert_eq!(from_json.schema, from_binary.schema, "{}", case.case.id);
    }
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(cases.len() >= 48, "the suite is smaller than expected");
}

/// Only the media type asks for bytes; `*/*`, JSON or a refusal with `q=0`
/// answer JSON. Either way `Vary: Accept` tells a cache the two apart.
#[tokio::test]
async fn only_the_media_type_asks_for_bytes() {
    let app = app();
    let body = r#"{"source":"orders","select":["id"],"sort":[{"field":"id"}],"limit":2}"#;
    for (accept, binary) in [
        (None, false),
        (Some("*/*"), false),
        (Some("application/json"), false),
        (Some(&format!("{MEDIA_TYPE};q=0") as &str), false),
        (
            Some(&format!("application/json, {MEDIA_TYPE}; q=0.9") as &str),
            true,
        ),
        (Some(MEDIA_TYPE), true),
    ] {
        let (status, content_type, vary, _) = post(&app, "/query/orders", body, accept).await;
        assert_eq!(status, StatusCode::OK);
        let expected = if binary {
            MEDIA_TYPE
        } else {
            "application/json"
        };
        assert_eq!(content_type, expected, "Accept: {accept:?}");
        assert_eq!(vary.as_deref(), Some("Accept"));
    }
}

/// An error stays the JSON error form, whatever `Accept` asked: a client has
/// to read the sentence.
#[tokio::test]
async fn an_error_stays_json() {
    let (status, content_type, _, body) = post(
        &app(),
        "/query/orders",
        r#"{"source":"orders","select":["nope"]}"#,
        Some(MEDIA_TYPE),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(content_type, "application/json");
    assert!(String::from_utf8(body).unwrap().contains("\"error\""));
}

/// The pivot answers the same in both forms.
#[tokio::test]
async fn a_pivot_answers_the_same_in_both_forms() {
    let app = app();
    let body = r#"{"source":"orders","rows":["country"],"columns":["ordered_year"],
        "values":[{"field":"qty","fn":"sum","as":"total"}]}"#;
    let (_, _, _, json) = post(&app, "/pivot/orders", body, None).await;
    let (status, content_type, _, bytes) =
        post(&app, "/pivot/orders", body, Some(MEDIA_TYPE)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, MEDIA_TYPE);
    let (from_json, rows) = opengrid_pivot::pivot_from_json(std::str::from_utf8(&json).unwrap())
        .expect("the JSON pivot");
    let (from_binary, binary_rows) = opengrid_pivot::pivot_from_bytes(&bytes).expect("the bytes");
    assert_eq!(rows, binary_rows);
    assert_eq!(from_json.row_levels, from_binary.row_levels);
    assert_eq!(from_json.data.columns, from_binary.data.columns);
    // The JSON form writes a path untyped; written again from the binary
    // answer's typed path, it is the same text.
    assert_eq!(
        opengrid_pivot::pivot_to_json(&from_binary, &binary_rows),
        std::str::from_utf8(&json).unwrap()
    );
}
