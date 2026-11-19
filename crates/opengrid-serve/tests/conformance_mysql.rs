//! The conformance suite against a server started from a fresh MySQL table's
//! proposal (issue #501). Skips itself without a database
//! (`OPENGRID_TEST_MYSQL`).
//!
//! The proposal reads the table's types exactly — `DECIMAL(12,2)` stays a
//! decimal, `TINYINT(1)` a boolean, `TIMESTAMP(6)` a timestamp — so the owner's
//! only edit is the derived columns. A column the file does not name never
//! reaches a client, whatever the table holds.

#[path = "../../opengrid-connector-mysql/tests/my/mod.rs"]
mod my;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use mysql_async::prelude::Queryable;
use opengrid_conformance::{RowOrder, Table, check_dir, compare, fixture_schema};
use opengrid_datasource::wire::result_from_json;
use opengrid_json::{Json, ToJson};
use opengrid_serve::{Config, read_proposal};
use opengrid_types::{Field, Schema};
use tower::ServiceExt;

const TOKEN: &str = "conformance-token-0123456789-abcdefghij";
const TABLE: &str = "og_serve_orders";

/// The cases that read the fixture's one NaN, which a `DOUBLE` cannot hold.
const READS_THE_NAN: &[&str] = &[
    "s7-nan-is-not-null-in-comparisons",
    "s7-nan-sorts-after-numbers",
    "s7-nan-sorts-before-numbers-descending",
];

async fn post(app: &axum::Router, body: String) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/query/orders")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn the_whole_suite_passes_against_the_mysql_the_file_describes() {
    let Some(url) = my::url() else {
        return;
    };
    let schema = fixture_schema();
    let mut connection = my::connect(&url).await;
    my::create(&mut connection, &schema, TABLE, "orders.csv").await;
    // A column the owner will not serve.
    connection
        .query_drop(format!(
            "ALTER TABLE `{TABLE}` ADD COLUMN secret_note TEXT CHARACTER SET utf8mb4; \
             UPDATE `{TABLE}` SET secret_note = 'do not serve'"
        ))
        .await
        .expect("the secret column");

    let (text, _) = read_proposal(&url, "DB", "public")
        .await
        .expect("a proposal");
    let file = Json::parse(&text).expect("JSON");
    let source = file
        .get("sources")
        .and_then(|s| s.as_array())
        .and_then(|s| {
            s.iter()
                .find(|s| s.get("name").and_then(|n| n.as_str()) == Some(TABLE))
        })
        .expect("the table is proposed")
        .clone();
    assert_eq!(
        source.get("database").and_then(|d| d.as_str()),
        Some("mysql")
    );
    let proposed: Schema = opengrid_json::FromJson::from_json(source.get("schema").unwrap())
        .expect("the proposed schema");
    let mut expected: Vec<_> = schema
        .stored()
        .fields()
        .iter()
        .map(|f| (f.name.as_str().to_owned(), f.data_type))
        .collect();
    expected.push(("secret_note".to_owned(), opengrid_types::DataType::Utf8));
    assert_eq!(
        proposed
            .fields()
            .iter()
            .map(|f| (f.name.as_str().to_owned(), f.data_type))
            .collect::<Vec<_>>(),
        expected,
        "the proposal reads every type as the fixture's"
    );

    // The owner's edit: the secret column out, the derived columns in.
    let mut fields: Vec<Field> = proposed
        .fields()
        .iter()
        .filter(|f| f.name.as_str() != "secret_note")
        .cloned()
        .collect();
    fields.extend(schema.fields().iter().filter(|f| f.is_derived()).cloned());
    let mut source = source;
    let object = source.as_object_mut().unwrap();
    object.insert("schema", Schema::new(fields).to_json());
    object.insert("name", Json::from("orders"));
    let edited = Json::object([
        (
            "tokens",
            Json::parse(r#"[{"env":"TOKEN","context":{}}]"#).unwrap(),
        ),
        ("sources", Json::Array(vec![source])),
    ]);
    let env = |name: &str| match name {
        "DB" => Some(url.clone()),
        "TOKEN" => Some(TOKEN.to_owned()),
        _ => None,
    };
    let app = Config::parse(&edited.to_string())
        .expect("the file reads")
        .into_builder(env)
        .expect("a builder")
        .build()
        .await
        .expect("a server")
        .router();

    let cases = check_dir(&opengrid_conformance::suite_dir().join("cases"), &schema)
        .expect("the cases load");
    let mut failed = Vec::new();
    for case in &cases {
        if READS_THE_NAN.contains(&case.case.id.as_str()) {
            continue;
        }
        let (status, body) = post(&app, opengrid_json::to_string(&case.case.query)).await;
        if status != StatusCode::OK {
            failed.push(format!("{}: {status} {body}", case.case.id));
            continue;
        }
        let answered = result_from_json(&body).expect("a result");
        let order = if case.case.ordered {
            RowOrder::Ordered
        } else {
            RowOrder::Unordered
        };
        if let Err(mismatch) = compare(&case.expected, &Table::from(&answered), order) {
            failed.push(format!("{}: {mismatch}", case.case.id));
        }
    }

    // The column is in the table and out of the file: naming it, in a
    // selection, a sort or a filter, is refused, and its value never shows.
    for query in [
        r#"{"source":"orders","select":["id","secret_note"]}"#,
        r#"{"source":"orders","select":["id"],"sort":[{"field":"secret_note","direction":"asc"}]}"#,
        r#"{"source":"orders","select":["id"],"filter":{"field":"secret_note","op":"eq","value":"do not serve"}}"#,
    ] {
        let (status, body) = post(&app, query.to_owned()).await;
        assert_ne!(status, StatusCode::OK, "{query} was answered: {body}");
        assert!(!body.contains("do not serve"), "{body}");
    }

    my::drop_table(&mut connection, TABLE).await;
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(cases.len() >= 48, "the suite is smaller than expected");
}
