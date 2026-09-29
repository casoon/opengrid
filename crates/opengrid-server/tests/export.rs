//! `POST /export/{source}` through the real HTTP stack (issue #2), over the
//! local engine — the rules an export shares with `/query`, its own bound, its
//! formats and its headers. `export_postgres.rs` runs what needs a database:
//! the cursor, the tenant filter in SQL, and a client that goes away.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_connector::LocalConnector;
use opengrid_datasource::wire::{ErrorCode, WireError, result_from_json};
use opengrid_datasource_postgres::PostgresDataSource;
use opengrid_export::{CsvOptions, CsvWriter, JsonWriter};
use opengrid_query::Limits;
use opengrid_server::{RowFilter, Server, SourcePolicy};
use tower::ServiceExt;

const DE: &str = "token-de";
const FR: &str = "token-fr";
const ALL: &str = "token-all";

fn fixture() -> LocalConnector {
    LocalConnector::from_csv(
        &opengrid_conformance::fixture_csv(),
        &opengrid_conformance::suite_dir().join("data/orders.schema.json"),
    )
    .expect("the fixture")
}

/// A token without a context.
fn no_context() -> std::iter::Empty<(&'static str, &'static str)> {
    std::iter::empty()
}

/// Two sources over the conformance data set: `orders` with a tenant filter on
/// `country` — a column its callers may **not** see, as a tenant id would be —
/// and `open`, the same rows without either, to know what the truth is.
/// `max_limit` is 10 and `max_export_rows` 45: the export has its own bound,
/// and the 50 rows are above it.
async fn app() -> axum::Router {
    Server::builder()
        .max_payload_bytes(2048)
        .limits(Limits {
            max_limit: 10,
            ..Limits::default()
        })
        .max_export_rows(45)
        .allow_origin("http://127.0.0.1:8080")
        .token(DE, [("country", "DE")])
        .token(FR, [("country", "FR")])
        .token(ALL, no_context())
        .source(
            "orders",
            fixture(),
            SourcePolicy {
                allowed_fields: ["id", "customer", "amount", "note"]
                    .map(String::from)
                    .to_vec(),
                row_filter: Some(RowFilter::new("country", "eq", ":country")),
            },
        )
        .source("open", fixture(), SourcePolicy::default())
        .build()
        .await
        .expect("a server")
        .router()
}

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    /// The body as text — lossy for a binary one, which `bytes` holds.
    body: String,
    bytes: Vec<u8>,
}

async fn send(
    app: axum::Router,
    path: &str,
    token: Option<&str>,
    extra: &[(header::HeaderName, &str)],
    body: &str,
) -> Answer {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    for (name, value) in extra {
        request = request.header(name, *value);
    }
    let response = app
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .expect("the router answers");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a whole body")
        .to_bytes();
    Answer {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        bytes: bytes.to_vec(),
    }
}

async fn export(query: &str, token: &str, body: &str) -> Answer {
    send(
        app().await,
        &format!("/export/orders{query}"),
        Some(token),
        &[],
        body,
    )
    .await
}

fn error_of(answer: &Answer) -> WireError {
    WireError::from_json(&answer.body).unwrap_or_else(|| panic!("not an error: {}", answer.body))
}

/// The ids in a CSV of `id,…` without a byte order mark.
fn ids(csv: &str) -> Vec<i64> {
    csv.lines()
        .skip(1)
        .map(|line| line.split(',').next().unwrap().parse().unwrap())
        .collect()
}

/// The ids of `country`, read from the source without a filter.
async fn truth(country: &str) -> Vec<i64> {
    let body = format!(
        r#"{{"source":"open","select":["id"],"filter":{{"field":"country","op":"eq","value":"{country}"}},"sort":[{{"field":"id","direction":"asc"}}]}}"#
    );
    let answer = send(app().await, "/export/open?bom=false", Some(ALL), &[], &body).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    ids(&answer.body)
}

const BY_ID: &str = r#"{"source":"orders","select":["id","customer","amount","note"],"sort":[{"field":"id","direction":"asc"}]}"#;

/// **The tenant test.** Every row of an export is the caller's, and every row
/// of the caller's is in it — the filter is and-ed on the server whatever the
/// request says, on a column the caller cannot even name.
#[tokio::test]
async fn a_tenant_exports_its_own_rows_and_no_other() {
    for (token, country) in [(DE, "DE"), (FR, "FR")] {
        let answer = export("?bom=false", token, BY_ID).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
        let own = truth(country).await;
        assert!(!own.is_empty());
        assert_eq!(ids(&answer.body), own, "{country}");
    }

    // Asking for the other tenant's rows gets the intersection: nothing.
    let widening = r#"{"source":"orders","select":["id"],"filter":{"field":"id","op":"gte","value":0},"sort":[{"field":"id","direction":"asc"}]}"#;
    let answer = export("?bom=false", DE, widening).await;
    assert_eq!(ids(&answer.body), truth("DE").await);
    // And the tenant column itself is not there to ask with.
    let naming = r#"{"source":"orders","select":["id"],"filter":{"field":"country","op":"eq","value":"FR"}}"#;
    let answer = export("", DE, naming).await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_of(&answer).path.as_deref(), Some("filter.field"));
}

/// `allowed_fields` holds in an export as in a query: a hidden column is an
/// unknown field, with the same diagnosis as a typo.
#[tokio::test]
async fn a_hidden_column_is_an_unknown_field() {
    for field in ["country", "ratio", "no_such_column"] {
        let body = format!(r#"{{"source":"orders","select":["id","{field}"]}}"#);
        let answer = export("", DE, &body).await;
        assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        let error = error_of(&answer);
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.path.as_deref(), Some("select[1]"), "{field}");
        assert!(error.message.contains(field), "{}", error.message);
    }
}

/// The export is exactly what `opengrid-export` writes for the rows `/query`
/// answers — the same notation as every other export.
#[tokio::test]
async fn the_file_is_the_query_answer_in_the_export_notation() {
    let page = r#"{"source":"orders","select":["id","customer","amount","note"],"sort":[{"field":"id","direction":"asc"}],"limit":10}"#;
    let queried = send(app().await, "/query/orders", Some(DE), &[], page).await;
    let result = result_from_json(&queried.body).expect("a result");
    let windowed = page.replace(r#""limit":10"#, r#""limit":10,"offset":0"#);

    let csv = export("", DE, &windowed).await;
    assert_eq!(csv.status, StatusCode::OK, "{}", csv.body);
    assert_eq!(
        csv.body,
        CsvWriter::new(CsvOptions::default()).write(&result)
    );
    assert_eq!(csv.headers[header::CONTENT_TYPE], "text/csv; charset=utf-8");
    assert_eq!(
        csv.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"orders.csv\"; filename*=UTF-8''orders.csv"
    );
    assert_eq!(csv.headers["x-total-count"], "10");

    let options = CsvOptions {
        delimiter: ';',
        bom: false,
        protect_formulas: false,
        null: "\\N".to_owned(),
    };
    let spelled = export(
        "?delimiter=%3B&bom=false&protectFormulas=false&null=%5CN",
        DE,
        &windowed,
    )
    .await;
    assert_eq!(spelled.body, CsvWriter::new(options).write(&result));

    let json = export("?format=json", DE, &windowed).await;
    let mut writer = JsonWriter::new();
    let expected = writer.write(&result) + &writer.finish();
    assert_eq!(json.body, expected);
    assert_eq!(json.headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(
        json.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"orders.json\"; filename*=UTF-8''orders.json"
    );

    // `Accept` chooses when the parameter does not.
    let accepted = send(
        app().await,
        "/export/orders",
        Some(DE),
        &[(header::ACCEPT, "application/json")],
        &windowed,
    )
    .await;
    assert_eq!(accepted.body, expected);
}

/// An empty export still says what it would have held.
#[tokio::test]
async fn nothing_to_export_is_a_header_or_an_empty_array() {
    let none = r#"{"source":"orders","select":["id","customer"],"filter":{"field":"id","op":"lt","value":0}}"#;
    let csv = export("?bom=false", DE, none).await;
    assert_eq!(
        (csv.status, csv.body.as_str()),
        (StatusCode::OK, "id,customer\r\n")
    );
    assert_eq!(csv.headers["x-total-count"], "0");
    let json = export("?format=json", DE, none).await;
    assert_eq!(json.body, "[]");
}

/// `max_export_rows` is the export's bound, not `max_limit`: 25 rows pass
/// although a page may have 10, and more rows than 45 are a `413` with a
/// sentence — before the first byte, so there is no file, not a short one.
#[tokio::test]
async fn the_export_has_its_own_bound_and_says_so_before_the_first_byte() {
    let everything =
        r#"{"source":"open","select":["id"],"sort":[{"field":"id","direction":"asc"}]}"#;
    let refused = send(app().await, "/export/open", Some(ALL), &[], everything).await;
    assert_eq!(refused.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(refused.headers[header::CONTENT_TYPE], "application/json");
    let error = error_of(&refused);
    assert_eq!(error.code, ErrorCode::LimitExceeded);
    assert!(error.message.contains("50 rows"), "{}", error.message);
    assert!(
        error.message.contains("max_export_rows"),
        "{}",
        error.message
    );

    let fits = r#"{"source":"open","select":["id"],"filter":{"field":"id","op":"lte","value":25},"sort":[{"field":"id","direction":"asc"}]}"#;
    let answer = send(app().await, "/export/open?bom=false", Some(ALL), &[], fits).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(ids(&answer.body), (1..=25).collect::<Vec<_>>());

    // The bound counts what the export has, after its window.
    let windowed = r#"{"source":"open","select":["id"],"sort":[{"field":"id","direction":"asc"}],"offset":10}"#;
    let windowed = send(app().await, "/export/open", Some(ALL), &[], windowed).await;
    assert_eq!(windowed.status, StatusCode::OK);
    assert_eq!(windowed.headers["x-total-count"], "40");

    // A `limit` above the bound is refused by validation, like one above
    // `max_limit` on `/query`.
    let asked =
        r#"{"source":"open","select":["id"],"sort":[{"field":"id","direction":"asc"}],"limit":46}"#;
    let asked = send(app().await, "/export/open", Some(ALL), &[], asked).await;
    assert_eq!(asked.status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// The rest of `/query`'s door, and the options checked before anything runs.
#[tokio::test]
async fn an_export_is_guarded_like_a_query() {
    for token in [None, Some("wrong")] {
        let answer = send(app().await, "/export/orders", token, &[], BY_ID).await;
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    }
    let answer = send(app().await, "/export/nothing", Some(DE), &[], BY_ID).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    let answer = send(app().await, "/export/open", Some(ALL), &[], BY_ID).await;
    assert_eq!(
        answer.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "the path names the source"
    );
    let big = format!(
        r#"{{"source":"orders","select":["id"],"filter":{{"field":"customer","op":"eq","value":"{}"}}}}"#,
        "x".repeat(4096)
    );
    let answer = export("", DE, &big).await;
    assert_eq!(answer.status, StatusCode::PAYLOAD_TOO_LARGE);

    for query in [
        "?format=xls",
        "?delimiter=ab",
        "?format=json&bom=false",
        "?filename=x",
    ] {
        let answer = export(query, DE, BY_ID).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(error_of(&answer).code, ErrorCode::Malformed, "{query}");
    }
}

/// A page on another origin can read what it needs of the answer: a browser
/// hides every header of a cross-origin response that is not exposed.
#[tokio::test]
async fn a_browser_on_an_allowed_origin_may_read_the_name_and_the_count() {
    let answer = send(
        app().await,
        "/export/orders",
        Some(DE),
        &[(header::ORIGIN, "http://127.0.0.1:8080")],
        BY_ID,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    let exposed = answer.headers[header::ACCESS_CONTROL_EXPOSE_HEADERS]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(exposed.contains("content-disposition"), "{exposed}");
    assert!(exposed.contains("x-total-count"), "{exposed}");
}

/// Unset, `max_concurrent_exports` is half the smallest PostgreSQL pool — below
/// it, so exports never take every connection `/query` needs. The pool
/// connects lazily, so this needs no database.
#[tokio::test]
async fn the_default_leaves_half_of_every_pool_to_queries() {
    let schema = opengrid_conformance::fixture_schema();
    let source = PostgresDataSource::connect(
        "host=localhost dbname=opengrid_never_connected",
        "orders",
        schema,
    )
    .expect("a lazy pool");
    let pool = source.pool_size();
    let server = Server::builder()
        .source("orders", source, SourcePolicy::default())
        .build()
        .await
        .expect("a server");
    let exports = server.state().max_concurrent_exports;
    assert!(exports >= 1);
    assert!(exports < pool, "{exports} exports for a pool of {pool}");
    assert_eq!(exports, (pool / 2).max(1));
}

/// **One export more than `max_concurrent_exports` is a `503` with the code
/// `busy`** (issue #16) — not `limit_exceeded`, which says the request itself
/// is too big: a page waits and retries on the one and narrows on the other.
/// Over the local engine, so it runs without a database: 60 000 rows are six
/// pieces, more than the body holds ahead, so an export whose client does not
/// read keeps its place.
#[tokio::test]
async fn one_export_too_many_is_busy() {
    let dir = opengrid_conformance::suite_dir().join("../../target/opengrid-server-export-busy");
    std::fs::create_dir_all(&dir).unwrap();
    let mut csv = String::from("id\n");
    for id in 0..60_000 {
        csv.push_str(&format!("{id}\n"));
    }
    std::fs::write(dir.join("rows.csv"), csv).unwrap();
    std::fs::write(
        dir.join("rows.schema.json"),
        r#"{"fields":[{"name":"id","type":"int64","nullable":false}]}"#,
    )
    .unwrap();
    let rows = LocalConnector::from_csv(&dir.join("rows.csv"), &dir.join("rows.schema.json"))
        .expect("the rows");
    let app = Server::builder()
        .max_concurrent_exports(1)
        .token(ALL, no_context())
        .source("rows", rows, SourcePolicy::default())
        .build()
        .await
        .expect("a server")
        .router();
    let body = r#"{"source":"rows","select":["id"],"sort":[{"field":"id","direction":"asc"}]}"#;

    // One export, held: its body is never read.
    let held = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/export/rows")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {ALL}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("the router answers");
    assert_eq!(held.status(), StatusCode::OK);

    let turned = send(app.clone(), "/export/rows", Some(ALL), &[], body).await;
    assert_eq!(
        turned.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        turned.body
    );
    assert_eq!(turned.headers[header::CONTENT_TYPE], "application/json");
    assert!(turned.body.contains(r#""code":"busy""#), "{}", turned.body);
    let error = error_of(&turned);
    assert_eq!(error.code, ErrorCode::Busy);
    assert!(
        error.message.contains("max_concurrent_exports"),
        "{}",
        error.message
    );
    drop(held);
}

/// XLSX (issue #72): a workbook with the export's headers, and Excel's row
/// bound before the first byte. The cells themselves are pinned in
/// `opengrid-export`'s tests.
#[tokio::test]
async fn an_xlsx_export_is_a_workbook() {
    let body = r#"{"source":"orders","select":["id","customer"],"sort":[{"field":"id","direction":"asc"}]}"#;
    let answer = export("?format=xlsx", DE, body).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert_eq!(
        answer.headers[header::CONTENT_TYPE],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert!(
        answer.headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("orders.xlsx")
    );
    assert!(answer.headers.contains_key("x-total-count"));
    assert_eq!(&answer.bytes[..2], b"PK", "a ZIP archive");

    let refused = export("?format=xlsx&delimiter=%3B", DE, body).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(error_of(&refused).message.contains("this export is XLSX"));
}
