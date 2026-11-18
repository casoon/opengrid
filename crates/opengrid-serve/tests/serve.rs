//! Starting the server from the edited file (issue #486): only what the file
//! names is served, and the security model is the builder's own.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_serve::{Config, read_proposal};
use support::{Database, SQLITE_TABLES};
use tower::ServiceExt;

const TOKEN_DE: &str = "token-for-germany-0123456789abcdef-xyz";
const TOKEN_ALL: &str = "token-for-everyone-0123456789abcdef-xyz";

/// The proposal for `database`, read by the owner: no longer a proposal, the
/// secret note and the tenant column handled as the owner decides.
async fn edited(database: &Database, edit: impl Fn(String) -> String) -> String {
    let (text, _) = read_proposal(&database.connection(), "DB", "public")
        .await
        .expect("a proposal");
    edit(text.replace("\"proposal\": true", "\"proposal\": false"))
}

fn environment<'a>(
    database: &'a Database,
    extra: &'a [(&'a str, &'a str)],
) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| match name {
        "DB" => Some(database.connection()),
        other => extra
            .iter()
            .find(|(key, _)| *key == other)
            .map(|(_, value)| (*value).to_owned()),
    }
}

async fn router(text: &str, env: impl Fn(&str) -> Option<String>) -> axum::Router {
    Config::parse(text)
        .expect("the file reads")
        .into_builder(env)
        .expect("a builder")
        .build()
        .await
        .expect("a server")
        .router()
}

async fn query(app: axum::Router, token: &str, body: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query/orders")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// The owner deletes the secret note's field from the file.
fn without_the_note(text: String) -> String {
    text.replace(
        ",\n          {\"name\":\"secret_note\",\"type\":\"utf8\",\"nullable\":true}",
        "",
    )
}

fn with_token(text: String) -> String {
    text.replace(
        "\"tokens\": [{\"env\":\"OPENGRID_TOKEN\",\"context\":{}}]",
        "\"tokens\": [{\"env\":\"TOKEN_ALL\",\"context\":{}}]",
    )
}

#[tokio::test]
async fn the_edited_file_serves_what_it_names() {
    let database = Database::new("serve", SQLITE_TABLES);
    let text = edited(&database, |t| with_token(without_the_note(t))).await;
    let env = [("TOKEN_ALL", TOKEN_ALL)];
    let app = router(&text, environment(&database, &env)).await;

    let (status, body) = query(
        app,
        TOKEN_ALL,
        r#"{"source":"orders","select":["id","customer","qty"],"sort":[{"field":"id","direction":"asc"}]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Ada") && body.contains("Bo"), "{body}");
}

#[tokio::test]
async fn a_column_the_file_does_not_name_does_not_exist_for_a_client() {
    let database = Database::new("left-out-column", SQLITE_TABLES);
    let text = edited(&database, |t| with_token(without_the_note(t))).await;
    let env = [("TOKEN_ALL", TOKEN_ALL)];

    // The column is in the table, left out of the file: naming it, in a
    // select, a sort or a filter, is a refused query, not a leak.
    for body in [
        r#"{"source":"orders","select":["id","secret_note"]}"#,
        r#"{"source":"orders","select":["id"],"sort":[{"field":"secret_note","direction":"asc"}]}"#,
        r#"{"source":"orders","select":["id"],"filter":{"field":"secret_note","op":"eq","value":"do not serve"}}"#,
        // A column the proposal left out for its type.
        r#"{"source":"orders","select":["id","amount"]}"#,
    ] {
        let app = router(&text, environment(&database, &env)).await;
        let (status, answer) = query(app, TOKEN_ALL, body).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body} -> {answer}"
        );
        assert!(answer.contains("\"validation\""), "{answer}");
        assert!(!answer.contains("do not serve"), "{answer}");
    }
}

#[tokio::test]
async fn allowed_fields_and_the_tenant_row_filter_are_the_builders_own() {
    let database = Database::new("policy", SQLITE_TABLES);
    let text = edited(&database, |t| {
        with_token(t)
            .replace(
                "\"table\": \"orders\",",
                "\"table\": \"orders\",\n      \"allowed_fields\": [\"id\", \"customer\"],\n      \
                 \"row_filter\": {\"field\": \"country\", \"op\": \"eq\", \"value\": \":country\"},",
            )
            .replace(
                "\"context\":{}",
                "\"context\":{\"country\":\"DE\"}",
            )
    })
    .await;
    let env = [("TOKEN_ALL", TOKEN_ALL)];

    // The file names `qty`, but the policy does not allow it to a client.
    let app = router(&text, environment(&database, &env)).await;
    let (status, _) = query(
        app,
        TOKEN_ALL,
        r#"{"source":"orders","select":["id","qty"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // The tenant's rows only: Bo is in FR.
    let app = router(&text, environment(&database, &env)).await;
    let (status, body) = query(
        app,
        TOKEN_ALL,
        r#"{"source":"orders","select":["id","customer"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("Ada") && !body.contains("Bo"), "{body}");
}

#[tokio::test]
async fn a_proposal_that_was_not_read_does_not_start() {
    let database = Database::new("still-proposal", SQLITE_TABLES);
    let (text, _) = read_proposal(&database.connection(), "DB", "public")
        .await
        .expect("a proposal");
    let error = Config::parse(&text)
        .expect("the file reads")
        .into_builder(environment(&database, &[]))
        .err()
        .expect("refused");
    assert!(error.to_string().contains("still a proposal"), "{error}");
}

#[tokio::test]
async fn secrets_come_from_the_environment_and_are_checked() {
    let database = Database::new("secrets", SQLITE_TABLES);
    let text = edited(&database, with_token).await;
    let config = Config::parse(&text).expect("the file reads");

    // A token variable that is not set.
    let error = config
        .into_builder(environment(&database, &[]))
        .err()
        .expect("refused");
    assert!(
        error.to_string().contains("TOKEN_ALL is not set"),
        "{error}"
    );
    // A token that is too short.
    let error = config
        .into_builder(environment(&database, &[("TOKEN_ALL", "short")]))
        .err()
        .expect("refused");
    assert!(error.to_string().contains("shorter than"), "{error}");
    // A connection string that is not set.
    let error = config
        .into_builder(|name| (name == "TOKEN_ALL").then(|| TOKEN_ALL.to_owned()))
        .err()
        .expect("refused");
    assert!(error.to_string().contains("DB is not set"), "{error}");
    // And neither is in the file.
    assert!(!text.contains(TOKEN_ALL) && !text.contains(&database.connection()));
}

#[test]
fn a_key_the_file_does_not_know_is_refused() {
    for text in [
        r#"{"sources": [], "alow_origins": []}"#,
        r#"{"tokens": [{"env": "T", "secret": "x"}]}"#,
        r#"{"limits": {"timeout": 5}}"#,
    ] {
        assert!(Config::parse(text).is_err(), "{text}");
    }
}

#[tokio::test]
async fn another_token_is_a_401() {
    let database = Database::new("unauthorized", SQLITE_TABLES);
    let text = edited(&database, |t| with_token(without_the_note(t))).await;
    let env = [("TOKEN_ALL", TOKEN_ALL)];
    let app = router(&text, environment(&database, &env)).await;
    let (status, _) = query(app, TOKEN_DE, r#"{"source":"orders","select":["id"]}"#).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
