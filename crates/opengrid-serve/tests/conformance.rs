//! The conformance suite against a server started from nothing but a
//! connection string and the edited proposal (issue #486).
//!
//! The fixture is written into a SQLite file as the connector lays a table out
//! (decimals, dates and timestamps as integers, a boolean as 0/1). The
//! proposal reads those columns as `int64`; the owner's edit gives them their
//! types and adds the derived columns, and the suite — every case, through
//! `POST /query` — has to come back as it expects.

mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_conformance::{RowOrder, Table, check_dir, compare, fixture_schema};
use opengrid_connector::QueryResult;
use opengrid_connector_sqlite::{Connection, create_table, insert};
use opengrid_datasource::wire::result_from_json;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_json::{Json, ToJson};
use opengrid_serve::{Config, read_proposal};
use tower::ServiceExt;

const TOKEN: &str = "conformance-token-0123456789-abcdefghij";

#[tokio::test]
async fn the_whole_suite_passes_against_the_server_the_file_describes() {
    let schema = fixture_schema();
    let stored = schema.stored();

    // A SQLite file from nothing: the fixture's table and rows.
    let path = std::env::temp_dir().join(format!(
        "opengrid-serve-{}-suite.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    {
        let csv = std::fs::read(opengrid_conformance::fixture_csv()).expect("the fixture");
        let rows = load_csv(&csv, &stored, CsvOptions::default()).expect("ingest");
        let mut connection = Connection::open(&path).expect("a file");
        create_table(&connection, "orders", &schema).expect("the table");
        insert(
            &mut connection,
            "orders",
            &schema,
            &QueryResult::new(stored.clone(), rows.to_values(), 0),
        )
        .expect("the rows");
    }
    let connection = format!("sqlite:{}", path.display());

    // The proposal, and the owner's edit of it.
    let (text, skipped) = read_proposal(&connection, "DB", "public")
        .await
        .expect("a proposal");
    assert!(skipped.is_empty());
    let mut file = Json::parse(&text).expect("the proposal is JSON");
    let object = file.as_object_mut().expect("an object");
    let mut sources = object.remove("sources").expect("sources");
    let mut source = sources.as_array_mut().expect("an array").remove(0);
    let proposed: Vec<String> = source
        .get("schema")
        .and_then(|s| s.get("fields"))
        .and_then(|f| f.as_array())
        .expect("fields")
        .iter()
        .map(|f| f.get("name").and_then(|n| n.as_str()).unwrap().to_owned())
        .collect();
    let columns: Vec<&str> = stored.fields().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(proposed, columns, "every column of the table is proposed");
    // The types the connector's layout cannot say, and the derived columns.
    source
        .as_object_mut()
        .expect("an object")
        .insert("schema", schema.to_json());
    object.insert("sources", Json::Array(vec![source]));
    object.insert("proposal", Json::Bool(false));
    object.insert(
        "tokens",
        Json::parse(r#"[{"env":"TOKEN","context":{}}]"#).unwrap(),
    );

    let env = |name: &str| match name {
        "DB" => Some(connection.clone()),
        "TOKEN" => Some(TOKEN.to_owned()),
        _ => None,
    };
    let app = Config::parse(&file.to_string())
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
    let _ = std::fs::remove_file(&path);
    assert!(failed.is_empty(), "{failed:#?}");
    assert!(cases.len() >= 48, "the suite is smaller than expected");
}
