//! The endpoint, driven through the real HTTP stack (plan point 24).
//!
//! `oneshot` runs the router exactly as `axum::serve` would — routing,
//! extractors, handler, response — without binding a socket, so the tests stay
//! fast and free of port collisions.
//!
//! The data set is the conformance fixture: 50 rows with NULLs, NaN, exact
//! decimals and the NFC/NFD pair, which means the wire form gets exercised on
//! real values rather than on toy ones.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_datasource::wire::{ErrorCode, WireError, result_from_json};
use opengrid_server::{AppState, Config, Registry, router};
use tower::ServiceExt;

const TOKEN: &str = "s3cret-token";
const OTHER_TOKEN: &str = "other-token";

/// The repository root, so the fixture paths do not depend on the test's cwd.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/opengrid-server/../..")
        .to_path_buf()
}

/// A configuration over the conformance data set.
///
/// `allowed_fields` deliberately leaves out `note` and `flag`, and the mandatory
/// row filter names `country` — a column the client *may* see here, so the tests
/// can show both sides of it.
fn config_toml(row_filter: bool) -> String {
    let filter = if row_filter {
        "row_filter = { field = \"country\", op = \"eq\", value = \":country\" }"
    } else {
        ""
    };
    format!(
        r#"
[server]
max_payload_bytes = 2048
timeout_ms = 5000

[[tokens]]
value = "{TOKEN}"
context = {{ country = "DE" }}

[[tokens]]
value = "{OTHER_TOKEN}"
context = {{ country = "FR" }}

[[datasources]]
name = "orders"
type = "local-csv"
path = "crates/opengrid-conformance/data/orders.csv"
schema = "crates/opengrid-conformance/data/orders.schema.json"
allowed_fields = ["id", "customer", "country", "amount", "qty", "ordered_on", "ordered_year"]
{filter}
"#
    )
}

fn app(row_filter: bool) -> axum::Router {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);

    let root = repo_root();
    // A file per call: the tests run in parallel, and a shared path means one
    // test reads the configuration while another is still writing it.
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = root.join(format!("target/opengrid-server-test-{id}.toml"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, config_toml(row_filter)).unwrap();

    let config = Config::load(&path).expect("configuration");
    let registry = Registry::build(&config, &root).expect("registry");
    router(Arc::new(AppState::new(&config, registry)))
}

/// Sends a query and answers with the status and the body.
async fn post(
    app: axum::Router,
    source: &str,
    token: Option<&str>,
    body: &str,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/query/{source}"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn error_of(body: &str) -> WireError {
    WireError::from_json(body).unwrap_or_else(|| panic!("not an error body: {body}"))
}

#[tokio::test]
async fn a_valid_query_answers_in_the_wire_form() {
    let body = r#"{"source":"orders","select":["id","customer"],"sort":[{"field":"id","direction":"asc"}],"limit":3}"#;
    let (status, answer) = post(app(false), "orders", Some(TOKEN), body).await;

    assert_eq!(status, StatusCode::OK);
    let result = result_from_json(&answer).expect("the body is a result");
    assert_eq!(result.row_count(), 3);
    assert_eq!(result.total_count, 50, "total_count counts before paging");
    // The types come with it (point 23), so the client does not have to guess.
    assert_eq!(result.schema.len(), 2);
    assert_eq!(
        result.schema.field("id").unwrap().data_type,
        opengrid_types::DataType::Int64
    );
}

#[tokio::test]
async fn without_a_token_there_is_no_data() {
    let body = r#"{"source":"orders","select":["id"],"limit":1}"#;

    for token in [None, Some("wrong"), Some("")] {
        let (status, answer) = post(app(false), "orders", token, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token:?}");
        assert_eq!(error_of(&answer).code, ErrorCode::Unauthorized);
        // The refusal says nothing about which part was wrong.
        assert_eq!(
            error_of(&answer).message,
            "a valid bearer token is required"
        );
    }
}

/// The token must not appear in what the server says — not in the refusal, not
/// in any other error.
#[tokio::test]
async fn no_answer_carries_the_token() {
    let bodies = [
        r#"{"source":"orders","select":["id"],"limit":1}"#,
        r#"{"source":"orders","select":["nope"]}"#,
        "not json",
    ];
    for body in bodies {
        for token in [Some(TOKEN), Some("wrong"), None] {
            let (_, answer) = post(app(false), "orders", token, body).await;
            assert!(!answer.contains(TOKEN), "{answer}");
            assert!(!answer.contains(OTHER_TOKEN), "{answer}");
        }
    }
}

#[tokio::test]
async fn an_unknown_source_is_a_404() {
    let body = r#"{"source":"ghosts","select":["id"]}"#;
    let (status, answer) = post(app(false), "ghosts", Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&answer).code, ErrorCode::UnknownSource);
}

#[tokio::test]
async fn a_body_that_is_not_a_query_is_a_400() {
    for body in ["not json", "{}", r#"{"source":"orders","select":5}"#] {
        let (status, answer) = post(app(false), "orders", Some(TOKEN), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(error_of(&answer).code, ErrorCode::Malformed);
    }
}

#[tokio::test]
async fn an_invalid_query_names_the_place_it_broke() {
    // `ratio` exists in the schema but is not allowed, so it is as good as absent.
    let body = r#"{"source":"orders","select":["ratio"]}"#;
    let (status, answer) = post(app(false), "orders", Some(TOKEN), body).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let error = error_of(&answer);
    assert_eq!(error.code, ErrorCode::Validation);
    assert!(error.path.is_some(), "a validation error carries its path");
    // The message must not reveal that the column exists but is withheld.
    assert!(!error.message.contains("allowed"), "{}", error.message);
}

#[tokio::test]
async fn the_path_decides_which_source_is_queried() {
    // A *valid* identifier that is simply a different source — a hyphen would
    // already fail identifier parsing and never reach this check.
    let body = r#"{"source":"something_else","select":["id"]}"#;
    let (status, answer) = post(app(false), "orders", Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_of(&answer).path.as_deref(), Some("source"));
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused() {
    let padding = "x".repeat(4096);
    let body = format!(
        r#"{{"source":"orders","select":["id"],"filter":{{"field":"customer","op":"eq","value":"{padding}"}}}}"#
    );
    let (status, answer) = post(app(false), "orders", Some(TOKEN), &body).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_of(&answer).code, ErrorCode::LimitExceeded);
}

#[tokio::test]
async fn a_limit_over_the_maximum_is_a_validation_error() {
    let body = r#"{"source":"orders","select":["id"],"sort":[{"field":"id","direction":"asc"}],"limit":999999}"#;
    let (status, answer) = post(app(false), "orders", Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_of(&answer).code, ErrorCode::Validation);
}

/// E16: the mandatory filter is what separates one caller's rows from another's.
#[tokio::test]
async fn two_tokens_see_two_different_sets_of_rows() {
    let body = r#"{"source":"orders","select":["id","country"],"sort":[{"field":"id","direction":"asc"}],"limit":100}"#;

    let (status, german) = post(app(true), "orders", Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::OK);
    let german = result_from_json(&german).unwrap();

    let (_, french) = post(app(true), "orders", Some(OTHER_TOKEN), body).await;
    let french = result_from_json(&french).unwrap();

    assert!(german.total_count > 0 && french.total_count > 0);
    assert_ne!(german.total_count, french.total_count);
    // Every row the caller sees carries their own country, and nothing else.
    for (result, country) in [(&german, "DE"), (&french, "FR")] {
        for value in &result.columns[1] {
            assert_eq!(value, &opengrid_types::Value::Utf8(country.to_owned()));
        }
    }
}

/// The filter cannot be widened from outside: a request that asks for another
/// country gets the intersection — which is nothing — not the other country.
#[tokio::test]
async fn a_request_cannot_switch_the_mandatory_filter_off() {
    let body = r#"{"source":"orders","select":["id","country"],"filter":{"field":"country","op":"eq","value":"FR"},"limit":100}"#;
    let (status, answer) = post(app(true), "orders", Some(TOKEN), body).await;

    assert_eq!(status, StatusCode::OK);
    let result = result_from_json(&answer).unwrap();
    assert_eq!(
        result.total_count, 0,
        "the DE token must not be able to read FR rows"
    );
}

/// Without a row filter the same token sees everything — the difference above is
/// the filter's doing, not an accident of the data.
#[tokio::test]
async fn without_a_row_filter_a_token_sees_every_row() {
    let body = r#"{"source":"orders","select":["id"],"limit":100}"#;
    let (_, answer) = post(app(false), "orders", Some(TOKEN), body).await;
    assert_eq!(result_from_json(&answer).unwrap().total_count, 50);
}

/// Sends a `GET /source/{name}` and answers with the status and the body.
async fn describe(app: axum::Router, source: &str, token: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("GET")
        .uri(format!("/source/{source}"));
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// What a browser-side planner needs before it can split anything (point 28).
#[tokio::test]
async fn a_source_describes_its_schema_and_capabilities() {
    let (status, body) = describe(app(true), "orders", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);

    let described: serde_json::Value = serde_json::from_str(&body).expect("JSON");
    assert_eq!(described["name"], "orders");
    assert_eq!(described["capabilities"]["filter"], true);
    assert_eq!(described["capabilities"]["paging"], true);
    // Every flag is there, not only the ones that happen to be true: a planner
    // reads them all, and a missing one would silently mean "cannot".
    let flags = described["capabilities"].as_object().expect("capabilities");
    assert_eq!(flags.len(), 8, "the declaration is complete: {flags:?}");

    let fields: Vec<String> = described["schema"]["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .map(|field| field["name"].as_str().expect("a name").to_owned())
        .collect();
    assert!(fields.contains(&"id".to_owned()));
    assert!(
        !fields.contains(&"note".to_owned()),
        "a column outside allowed_fields does not exist for this caller: {fields:?}"
    );
}

/// The description is data too: no token, no answer.
#[tokio::test]
async fn describing_a_source_needs_a_token() {
    let (status, body) = describe(app(true), "orders", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_of(&body).code, ErrorCode::Unauthorized);

    let (status, body) = describe(app(true), "nope", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body).code, ErrorCode::UnknownSource);
}

/// A derived column is a column for the client — and only that (point 54).
///
/// The gateway computes it; the client sees an `int64` it can filter, group and
/// sort by, and the schema it is handed says nothing about where the values come
/// from. That is the same line the mandatory row filter (E16) sits on.
#[tokio::test]
async fn a_derived_column_works_but_does_not_announce_itself() {
    let body = r#"{"source":"orders","select":["ordered_year","rows"],
        "group":["ordered_year"],
        "aggregate":[{"fn":"count","as":"rows"}],
        "sort":[{"field":"ordered_year","direction":"asc"}]}"#;
    let (status, answer) = post(app(false), "orders", Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let result: serde_json::Value = serde_json::from_str(&answer).expect("JSON");
    assert_eq!(result["columns"][0]["name"], "ordered_year");
    assert_eq!(result["columns"][0]["values"][0], 2025);

    let (status, described) = describe(app(false), "orders", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    let described: serde_json::Value = serde_json::from_str(&described).expect("JSON");
    let year = described["schema"]["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .find(|field| field["name"] == "ordered_year")
        .expect("the client may see the column");
    assert_eq!(year["type"], "int64");
    assert!(
        year.get("from").is_none(),
        "where the values come from is the server's business: {year}"
    );
}
