//! The conformance suite against a server started from a fresh PostgreSQL
//! table's proposal (issue #486). Skips itself without a database
//! (`OPENGRID_TEST_PG`).
//!
//! The proposal reads the table's types exactly — `numeric(12,2)` stays a
//! decimal, `date` a date, `timestamptz` a timestamp — so the owner's only edit
//! is the derived columns.

#[path = "../../opengrid-datasource-postgres/tests/pg/mod.rs"]
mod pg;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_conformance::{RowOrder, Table, check_dir, compare, fixture_schema};
use opengrid_datasource::wire::result_from_json;
use opengrid_json::{Json, ToJson};
use opengrid_serve::{Config, read_proposal};
use opengrid_types::{Field, Schema};
use tower::ServiceExt;

const TOKEN: &str = "conformance-token-0123456789-abcdefghij";
const TABLE: &str = "og_serve_orders";

#[tokio::test]
async fn the_whole_suite_passes_against_the_postgres_the_file_describes() {
    let url = pg::connection().expect("a connection string");
    let (client, connection) = match tokio_postgres::connect(&url, tokio_postgres::NoTls).await {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("skipping the PostgreSQL server test: {error}");
            return;
        }
    };
    tokio::spawn(connection);
    let schema = fixture_schema();
    pg::create_fixture(&client, &schema, TABLE).await;

    let proposal = read_proposal(&url, "DB", "public").await;
    let (text, _) = proposal.expect("a proposal");

    // The proposal for this table, read as the owner would: its types are the
    // fixture's, and only the derived columns are left to add.
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
    let proposed: Schema = opengrid_json::FromJson::from_json(source.get("schema").unwrap())
        .expect("the proposed schema");
    let stored = schema.stored();
    assert_eq!(
        proposed
            .fields()
            .iter()
            .map(|f| (f.name.as_str(), f.data_type))
            .collect::<Vec<_>>(),
        stored
            .fields()
            .iter()
            .map(|f| (f.name.as_str(), f.data_type))
            .collect::<Vec<_>>(),
        "the proposal reads every type as the fixture's"
    );
    let mut fields: Vec<Field> = proposed.fields().to_vec();
    fields.extend(schema.fields().iter().filter(|f| f.is_derived()).cloned());

    let mut source = source;
    // The source is called what the suite's queries call it.
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
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/query/orders")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .body(Body::from(opengrid_json::to_string(&case.case.query)))
                    .unwrap(),
            )
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
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
    let _ = client
        .batch_execute(&format!("DROP TABLE IF EXISTS \"{TABLE}\""))
        .await;
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(cases.len() >= 48, "the suite is smaller than expected");
}
