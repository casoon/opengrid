//! The tree on the server (plan point 122, E38): every tree conformance case
//! through `POST /query`, against each reference connector — a file (the
//! engine answers the tree itself), SQLite and PostgreSQL (the server asks for
//! the rows of the tree's scope and the engine answers the level) — and the
//! tenant rule: the mandatory row filter decides which rows the tree consists
//! of, so another tenant's row is never a match, context or child.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_conformance::{check_tree_case, load_schema, tree_cases, tree_dataset};
use opengrid_connector::{
    BoxFuture, Connector, DataSourceCapabilities, DataSourceError, LocalConnector, QueryResult,
    Schema, ValidatedQuery,
};
use opengrid_connector_sqlite::{Connection, SqliteConnector, create_table, insert};
use opengrid_datasource::wire::result_from_json;
use opengrid_engine::ingest::{CsvOptions, load_csv};
use opengrid_server::{RowFilter, Server, SourcePolicy};
use tower::ServiceExt;

const TOKEN: &str = "token";
const EU: &str = "token-eu";
const US: &str = "token-us";
const SOURCES: [&str; 2] = ["tree", "tree_cycle"];

/// The rows of a dataset, as a connector stores them.
fn rows(source: &str) -> (opengrid_types::Schema, QueryResult) {
    let (csv, schema) = tree_dataset(source);
    let schema = load_schema(&schema).expect("schema");
    let table =
        load_csv(&std::fs::read(csv).unwrap(), &schema, CsvOptions::default()).expect("the rows");
    let rows = QueryResult::new(schema.clone(), table.to_values(), 0);
    (schema, rows)
}

fn file(source: &str) -> Arc<dyn Connector> {
    let (csv, schema) = tree_dataset(source);
    Arc::new(LocalConnector::from_csv(&csv, &schema).expect("a file source"))
}

fn sqlite(source: &str) -> Arc<dyn Connector> {
    let (schema, rows) = rows(source);
    let mut connection = Connection::open_in_memory().expect("a database");
    create_table(&connection, source, &schema).expect("the table");
    insert(&mut connection, source, &schema, &rows).expect("the rows");
    Arc::new(SqliteConnector::new(connection, source, schema).expect("a connector"))
}

/// PostgreSQL, when this machine has one: the dataset in `og_test_<source>`.
async fn postgres(source: &str) -> Option<Arc<dyn Connector>> {
    let url = std::env::var("OPENGRID_TEST_PG")
        .unwrap_or_else(|_| "host=localhost dbname=postgres".to_owned());
    let (client, connection) = match tokio_postgres::connect(&url, tokio_postgres::NoTls).await {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("skipping the PostgreSQL tree: {error}");
            return None;
        }
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let (schema, rows) = rows(source);
    let table = format!("og_test_{source}");
    let columns: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| {
            let kind = match field.data_type {
                opengrid_types::DataType::Utf8 => "text",
                _ => "bigint",
            };
            format!("\"{}\" {kind}", field.name)
        })
        .collect();
    let mut sql = format!(
        "DROP TABLE IF EXISTS \"{table}\"; CREATE TABLE \"{table}\" ({});",
        columns.join(", ")
    );
    for row in 0..rows.row_count() {
        let values: Vec<String> = rows
            .columns
            .iter()
            .map(|column| match &column[row] {
                opengrid_types::Value::Null => "NULL".to_owned(),
                opengrid_types::Value::Utf8(text) => format!("'{}'", text.replace('\'', "''")),
                other => opengrid_json::to_string(other),
            })
            .collect();
        sql.push_str(&format!(
            "INSERT INTO \"{table}\" VALUES ({});",
            values.join(", ")
        ));
    }
    client.batch_execute(&sql).await.expect("the table");
    Some(Arc::new(
        opengrid_datasource_postgres::PostgresDataSource::connect(&url, &table, schema)
            .expect("a source"),
    ))
}

/// A connector as the builder takes one: the test picks its connectors at
/// run time, and this hands every call through.
struct Shared(Arc<dyn Connector>);

impl Connector for Shared {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        self.0.schema()
    }
    fn capabilities(&self) -> DataSourceCapabilities {
        self.0.capabilities()
    }
    fn execute(
        &self,
        query: ValidatedQuery,
    ) -> BoxFuture<'_, Result<QueryResult, DataSourceError>> {
        self.0.execute(query)
    }
    fn tree<'a>(
        &'a self,
        query: &'a ValidatedQuery,
    ) -> BoxFuture<'a, Result<QueryResult, DataSourceError>> {
        self.0.tree(query)
    }
}

async fn app(connectors: Vec<(&str, Arc<dyn Connector>)>, row_filter: bool) -> axum::Router {
    let mut builder = Server::builder()
        .timeout(Duration::from_secs(10))
        .token(TOKEN, [("region", "EU")])
        .token(EU, [("region", "EU")])
        .token(US, [("region", "US")]);
    for (name, connector) in connectors {
        builder = builder.source(
            name,
            Shared(connector),
            SourcePolicy {
                allowed_fields: Vec::new(),
                row_filter: (row_filter && name == "tree")
                    .then(|| RowFilter::new("region", "eq", ":region")),
            },
        );
    }
    builder.build().await.expect("a server").router()
}

async fn ask(app: axum::Router, token: &str, query: &str) -> Result<QueryResult, String> {
    let source = opengrid_json::Json::parse(query).unwrap()["source"]
        .as_str()
        .unwrap()
        .to_owned();
    let request = Request::builder()
        .method("POST")
        .uri(format!("/query/{source}"))
        .header(header::CONTENT_TYPE, "application/json")
        // The binary form: it carries the tree's part too (#135).
        .header(header::ACCEPT, "application/vnd.opengrid.columns")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(query.to_owned()))
        .unwrap();
    let response = app.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let binary = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|kind| kind.as_bytes() == opengrid_columns::wire::MEDIA_TYPE.as_bytes());
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    if status != StatusCode::OK {
        return Err(String::from_utf8(bytes).unwrap());
    }
    if binary {
        // The binary form carries a tree's part as well (E38, #135).
        let (table, total, tree) =
            opengrid_columns::wire::decode_answer(&bytes).map_err(|e| e.to_string())?;
        let mut result = QueryResult::new(table.schema().clone(), table.to_values(), total);
        result.tree = tree.map(|tree| opengrid_datasource::TreeLevel {
            children: tree.children,
            matched: tree.matched,
            matches: tree.matches,
            orphans: tree.orphans,
        });
        return Ok(result);
    }
    let body = String::from_utf8(bytes).unwrap();
    result_from_json(&body).map_err(|error| format!("{error}: {body}"))
}

/// Every tree case against one kind of connector.
async fn every_case(kind: &str, connectors: Vec<(&str, Arc<dyn Connector>)>) {
    let app = app(connectors, false).await;
    let mut failures = Vec::new();
    for (id, case) in tree_cases() {
        let answer = ask(app.clone(), TOKEN, &case["query"].to_string()).await;
        failures.extend(
            check_tree_case(&case, answer)
                .into_iter()
                .map(|problem| format!("{kind} {id}: {problem}")),
        );
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn a_file_answers_every_tree_case() {
    every_case("file", SOURCES.iter().map(|s| (*s, file(s))).collect()).await;
}

#[tokio::test]
async fn sqlite_answers_every_tree_case() {
    every_case("sqlite", SOURCES.iter().map(|s| (*s, sqlite(s))).collect()).await;
}

#[tokio::test]
async fn postgres_answers_every_tree_case() {
    let mut connectors = Vec::new();
    for source in SOURCES {
        match postgres(source).await {
            Some(connector) => connectors.push((source, connector)),
            None => return,
        }
    }
    every_case("postgres", connectors).await;
}

/// The tenant rule (plan point 122): the row filter is the tree's scope. A
/// match in another region is no match, its ancestors no context, and a node
/// whose parent is in another region is a root of its own.
#[tokio::test]
async fn another_tenants_row_is_never_part_of_the_tree() {
    for (kind, connector) in [("file", file("tree")), ("sqlite", sqlite("tree"))] {
        let app = app(vec![("tree", connector)], true).await;
        // Dave is in the US: for the EU nothing matches, and nothing is context.
        let dave = r#"{"source":"tree","select":["id"],"tree":{"parent":"parent_id"},
            "filter":{"field":"name","op":"eq","value":"Dave"},"sort":[{"field":"id"}]}"#;
        let eu = ask(app.clone(), EU, dave).await.expect("answers");
        assert_eq!(eu.row_count(), 0, "{kind}");
        assert_eq!(eu.tree.as_ref().unwrap().matches, 0, "{kind}");

        // The US sees its two roots — Partners, and the orphan — and nothing
        // of the EU: not Sales, not as a parent, not as a count.
        let roots = r#"{"source":"tree","select":["id"],"tree":{"parent":"parent_id"},
            "sort":[{"field":"id"}]}"#;
        let us = ask(app.clone(), US, roots).await.expect("answers");
        let ids: Vec<String> = us.columns[0].iter().map(opengrid_json::to_string).collect();
        assert_eq!(ids, ["7", "9"], "{kind}");
        assert_eq!(
            us.tree.as_ref().unwrap().matches,
            3,
            "{kind}: the US has three rows"
        );

        // A hidden node between a match and its root (the row filter hides
        // North): Eve's chain stops at Alice, who becomes a root — North is
        // never shown as context, Sales never reached through it. On the
        // query's filter instead of the scope, North would appear (T5).
        let hiding = Server::builder()
            .token(TOKEN, [("hidden", "North")])
            .source(
                "tree",
                Shared(match kind {
                    "file" => file("tree"),
                    _ => sqlite("tree"),
                }),
                SourcePolicy {
                    allowed_fields: Vec::new(),
                    row_filter: Some(RowFilter::new("name", "ne", ":hidden")),
                },
            )
            .build()
            .await
            .expect("a server")
            .router();
        let eve = r#"{"source":"tree","select":["id","name"],"tree":{"parent":"parent_id"},
            "filter":{"field":"name","op":"eq","value":"Eve"},"sort":[{"field":"id"}]}"#;
        let roots = ask(hiding.clone(), TOKEN, eve).await.expect("answers");
        let names: Vec<String> = roots.columns[1]
            .iter()
            .map(opengrid_json::to_string)
            .collect();
        assert_eq!(
            names,
            ["\"Alice\""],
            "{kind}: Alice is a root now, as context"
        );
        assert_eq!(roots.tree.as_ref().unwrap().matched, [false], "{kind}");

        // Under an EU node, the US finds nothing.
        let under = r#"{"source":"tree","select":["id"],"tree":{"parent":"parent_id","under":1},
            "sort":[{"field":"id"}]}"#;
        assert_eq!(
            ask(app.clone(), US, under).await.unwrap().row_count(),
            0,
            "{kind}"
        );
    }
}
