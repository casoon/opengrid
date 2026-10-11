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
    sqlite_of(source, schema, &rows, false)
}

/// SQLite holding `rows` in the table `source`, with the tree's pushdown
/// (issue #132) or without.
fn sqlite_of(
    source: &str,
    schema: Schema,
    rows: &QueryResult,
    pushdown: bool,
) -> Arc<dyn Connector> {
    let mut connection = Connection::open_in_memory().expect("a database");
    create_table(&connection, source, &schema).expect("the table");
    insert(&mut connection, source, &schema, rows).expect("the rows");
    Arc::new(
        SqliteConnector::new(connection, source, schema)
            .expect("a connector")
            .with_tree_pushdown(pushdown),
    )
}

/// PostgreSQL, when this machine has one: the dataset in `og_test_<source>`.
async fn postgres(source: &str) -> Option<Arc<dyn Connector>> {
    let (schema, rows) = rows(source);
    postgres_of(source, schema, &rows, false).await
}

/// PostgreSQL holding `rows` in `og_test_<source>`, when this machine has one.
async fn postgres_of(
    source: &str,
    schema: Schema,
    rows: &QueryResult,
    pushdown: bool,
) -> Option<Arc<dyn Connector>> {
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
    client
        .batch_execute(&format!(
            "DROP TABLE IF EXISTS \"{table}\"; CREATE TABLE \"{table}\" ({});",
            columns.join(", ")
        ))
        .await
        .expect("the table");
    // A thousand rows a statement: a hundred thousand one by one would take
    // longer than the test.
    let rows_of: Vec<String> = (0..rows.row_count())
        .map(|row| {
            let values: Vec<String> = rows
                .columns
                .iter()
                .map(|column| match &column[row] {
                    opengrid_types::Value::Null => "NULL".to_owned(),
                    opengrid_types::Value::Utf8(text) => {
                        format!("'{}'", text.replace('\'', "''"))
                    }
                    other => opengrid_json::to_string(other),
                })
                .collect();
            format!("({})", values.join(", "))
        })
        .collect();
    for chunk in rows_of.chunks(1000) {
        client
            .batch_execute(&format!(
                "INSERT INTO \"{table}\" VALUES {};",
                chunk.join(", ")
            ))
            .await
            .expect("the rows");
    }
    Some(Arc::new(
        opengrid_datasource_postgres::PostgresDataSource::connect(&url, &table, schema)
            .expect("a source")
            .with_tree_pushdown(pushdown),
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
            aggregate_schema: tree
                .aggregates
                .as_ref()
                .map(|table| table.schema().clone())
                .unwrap_or_default(),
            aggregates: tree
                .aggregates
                .map(|table| table.to_values())
                .unwrap_or_default(),
            flat: tree.flat.map(|flat| opengrid_datasource::FlatTree {
                levels: flat.levels,
                paths: flat.paths,
                key_type: flat.key_type,
                filtered: flat.filtered,
            }),
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

/// The same cases with the tree pushed down into SQLite (issue #132).
#[tokio::test]
async fn sqlite_with_the_pushdown_answers_every_tree_case() {
    let connectors = SOURCES
        .iter()
        .map(|source| {
            let (schema, rows) = rows(source);
            (*source, sqlite_of(source, schema, &rows, true))
        })
        .collect();
    every_case("sqlite pushdown", connectors).await;
}

/// The same cases with the tree pushed down into PostgreSQL (issue #132).
#[tokio::test]
async fn postgres_with_the_pushdown_answers_every_tree_case() {
    let mut connectors = Vec::new();
    for source in SOURCES {
        let (schema, rows) = rows(source);
        // A table of its own: the test without the pushdown runs beside it.
        match postgres_of(&format!("{source}_pushdown"), schema, &rows, true).await {
            Some(connector) => connectors.push((source, connector)),
            None => return,
        }
    }
    every_case("postgres pushdown", connectors).await;
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

/// What `POST /export/{source}` answers with `token` for `query`, as text.
async fn export(app: axum::Router, token: &str, path: &str, query: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(query.to_owned()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, String::from_utf8(bytes).unwrap())
}

/// The whole tree as a file (T8, #166): depth-first, with level and path —
/// joined in CSV, an array in JSON, `match` only with a filter — over a file
/// and SQLite alike, and for a tenant only its own rows.
#[tokio::test]
async fn a_tree_exports_flat_with_level_and_path() {
    let eve = r#"{"source":"tree","select":["id","name"],"tree":{"parent":"parent_id","flat":true},
        "filter":{"field":"name","op":"eq","value":"Eve"},"sort":[{"field":"id"}]}"#;
    for (kind, connector) in [("file", file("tree")), ("sqlite", sqlite("tree"))] {
        let app = app(vec![("tree", connector)], false).await;
        let (status, csv) =
            export(app.clone(), TOKEN, "/export/tree?format=csv&bom=false", eve).await;
        assert_eq!(status, StatusCode::OK, "{kind}: {csv}");
        assert_eq!(
            csv,
            "id,name,level,path,match\r\n1,Sales,1,1,false\r\n2,North,2,1 / 2,false\r\n\
             4,Alice,3,1 / 2 / 4,false\r\n10,Eve,4,1 / 2 / 4 / 10,true\r\n",
            "{kind}"
        );
        let (_, json) = export(app.clone(), TOKEN, "/export/tree?format=json", eve).await;
        assert!(
            json.contains(r#"{"id":10,"name":"Eve","level":4,"path":[1,2,4,10],"match":true}"#),
            "{kind}: {json}"
        );
    }

    // The tenant rule holds in an export too: the US sees its two roots and
    // Dave, nothing of the EU.
    let app = app(vec![("tree", file("tree"))], true).await;
    let all = r#"{"source":"tree","select":["id"],"tree":{"parent":"parent_id","flat":true},
        "sort":[{"field":"id"}]}"#;
    let (_, csv) = export(app, US, "/export/tree?format=csv&bom=false", all).await;
    assert_eq!(csv, "id,level,path\r\n7,1,7\r\n8,2,7 / 8\r\n9,1,9\r\n");
}

/// A deterministic tree of `nodes` nodes (issue #132): five roots, every
/// other node under an earlier one — some close (long chains), some far
/// (wide levels) — fifty orphans, NULLs and repeated names, and two columns
/// called `r` and `n`, the names a statement's own helpers could clash with.
fn generated(nodes: u64) -> (Schema, QueryResult) {
    let schema: Schema = opengrid_json::from_str(
        r#"{"fields":[
            {"name":"id","type":"int64","nullable":false},
            {"name":"parent_id","type":"int64","nullable":true},
            {"name":"name","type":"utf8","nullable":true},
            {"name":"value","type":"int64","nullable":true},
            {"name":"r","type":"int64","nullable":true},
            {"name":"n","type":"utf8","nullable":true}]}"#,
    )
    .expect("a schema");
    let mut state: u64 = 0x5eed_0132;
    let mut next = move |bound: u64| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) % bound.max(1)
    };
    let mut csv = String::from("id,parent_id,name,value,r,n\n");
    for id in 1..=nodes {
        let parent = if id <= 5 {
            "\\N".to_owned()
        } else if id % 2000 == 0 {
            (nodes + 1 + next(1000)).to_string()
        } else if next(3) == 0 {
            (id - 1 - next(id.min(20) - 1)).to_string()
        } else {
            (1 + next(id - 1)).to_string()
        };
        let name = if next(40) == 0 {
            "\\N".to_owned()
        } else {
            format!("Name {}", next(500))
        };
        let value = if next(20) == 0 {
            "\\N".to_owned()
        } else {
            next(1000).to_string()
        };
        let r = next(100);
        let n = next(7);
        csv.push_str(&format!("{id},{parent},{name},{value},{r},n{n}\n"));
    }
    let table = load_csv(csv.as_bytes(), &schema, CsvOptions::default()).expect("the rows");
    let rows = QueryResult::new(schema.clone(), table.to_values(), 0);
    (schema, rows)
}

/// The engine over the same rows: what every pushdown is compared with.
fn engine_of(schema: &Schema, rows: &QueryResult) -> Arc<dyn Connector> {
    let table = opengrid_columns::Table::from_values(schema, &rows.columns).expect("a table");
    Arc::new(LocalConnector::new(
        opengrid_engine::datasource::LocalDataSource::new(table),
    ))
}

/// The queries the pushdown is compared on: levels at the roots and under
/// wide and deep nodes, with filters that leave context, a scope, sorts in
/// both directions with NULLs, paging, aggregates over subtrees, and a node
/// that does not exist.
fn differential_queries() -> Vec<String> {
    let select = r#""select":["id","name","value","r","n"]"#;
    let aggregate = r#""aggregate":[{"field":"value","fn":"sum","as":"total"},{"fn":"count","as":"nodes"},{"field":"value","fn":"count","as":"valued"},{"field":"value","fn":"avg","as":"mean"},{"field":"value","fn":"min","as":"low"},{"field":"value","fn":"max","as":"high"}]"#;
    vec![
        format!(
            r#"{{"source":"big",{select},"tree":{{"parent":"parent_id",{aggregate}}},"sort":[{{"field":"name"}}],"limit":40}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"tree":{{"parent":"parent_id","under":1,{aggregate}}},"sort":[{{"field":"value","direction":"desc"}}],"offset":7,"limit":25}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"tree":{{"parent":"parent_id","under":3}},"sort":[{{"field":"name","direction":"desc","nulls":"first"}},{{"field":"r"}}],"limit":100}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"filter":{{"field":"value","op":"gt","value":990}},"tree":{{"parent":"parent_id",{aggregate}}},"sort":[{{"field":"id"}}],"limit":50}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"filter":{{"field":"name","op":"eq","value":"Name 42"}},"tree":{{"parent":"parent_id","under":2,{aggregate}}},"sort":[{{"field":"id"}}],"limit":50}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"filter":{{"field":"n","op":"contains","value":"3"}},"tree":{{"parent":"parent_id","under":17}},"sort":[{{"field":"n"}},{{"field":"id","direction":"desc"}}],"limit":30}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"tree":{{"parent":"parent_id","scope":{{"field":"value","op":"is_not_null"}},{aggregate}}},"sort":[{{"field":"id"}}],"limit":60}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"tree":{{"parent":"parent_id","under":4000}},"sort":[{{"field":"id"}}],"limit":20}}"#
        ),
        format!(
            r#"{{"source":"big",{select},"tree":{{"parent":"parent_id","under":999999,{aggregate}}},"sort":[{{"field":"id"}}],"limit":20}}"#
        ),
    ]
}

/// The pushdown answers what the engine answers (issue #132): every query of
/// [`differential_queries`] over a generated tree of 100 000 nodes, in SQLite
/// and — when this machine has one — PostgreSQL, compared value for value with
/// the engine over the same rows: the level, its total, and its tree part.
#[tokio::test]
async fn the_pushdown_answers_what_the_engine_answers() {
    let (schema, rows) = generated(100_000);
    let engine = engine_of(&schema, &rows);
    let mut databases = vec![("sqlite", sqlite_of("big", schema.clone(), &rows, true))];
    if let Some(postgres) = postgres_of("big", schema.clone(), &rows, true).await {
        databases.push(("postgres", postgres));
    }
    let mut failures = Vec::new();
    for (index, text) in differential_queries().iter().enumerate() {
        let query: opengrid_query::Query = opengrid_json::from_str(text).expect("a query");
        let validated = query
            .validate(&schema, &opengrid_query::Limits::default())
            .expect("a valid query");
        let expected = engine
            .execute(validated.clone())
            .await
            .expect("the engine answers");
        assert!(
            expected.tree.is_some(),
            "query {index}: the engine answers a tree"
        );
        for (kind, database) in &databases {
            assert!(database.capabilities().tree, "{kind} pushes trees down");
            match database.execute(validated.clone()).await {
                Ok(answer) if answer == expected => {}
                Ok(answer) => failures.push(format!(
                    "{kind} query {index}: rows {} vs {}, total {} vs {}, tree differs: {}",
                    answer.row_count(),
                    expected.row_count(),
                    answer.total_count,
                    expected.total_count,
                    answer.tree != expected.tree
                )),
                Err(error) => failures.push(format!("{kind} query {index}: {error}")),
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// What the pushdown saves (issue #132): one level of the generated tree of
/// 100 000 nodes, the engine's way (`Connector::tree`: every row of the scope,
/// then the engine) against the database's (`execute`), the median of five.
/// Ignored, since it measures rather than checks:
/// `cargo test --release -p opengrid-server --test tree -- --ignored --nocapture`.
#[tokio::test]
#[ignore]
async fn measure_a_level_before_and_after() {
    let (schema, rows) = generated(100_000);
    let mut databases = vec![("sqlite", sqlite_of("big", schema.clone(), &rows, true))];
    if let Some(postgres) = postgres_of("big", schema.clone(), &rows, true).await {
        databases.push(("postgres", postgres));
    }
    let levels = [
        ("roots", r#""tree":{"parent":"parent_id"}"#),
        ("under 1", r#""tree":{"parent":"parent_id","under":1}"#),
        (
            "under 1, sum",
            r#""tree":{"parent":"parent_id","under":1,"aggregate":[{"field":"value","fn":"sum","as":"total"}]}"#,
        ),
    ];
    for (kind, database) in &databases {
        for (name, tree) in levels {
            let text = format!(
                r#"{{"source":"big","select":["id","name","value"],{tree},"sort":[{{"field":"id"}}],"limit":50}}"#
            );
            let query: opengrid_query::Query = opengrid_json::from_str(&text).expect("a query");
            let validated = query
                .validate(&schema, &opengrid_query::Limits::default())
                .expect("a valid query");
            let mut before = Vec::new();
            let mut after = Vec::new();
            for _ in 0..5 {
                let clock = std::time::Instant::now();
                database.tree(&validated).await.expect("the engine's way");
                before.push(clock.elapsed());
                let clock = std::time::Instant::now();
                database
                    .execute(validated.clone())
                    .await
                    .expect("the pushdown");
                after.push(clock.elapsed());
            }
            before.sort();
            after.sort();
            println!(
                "{kind:9} {name:13} before {:>8.1} ms   after {:>8.1} ms",
                before[2].as_secs_f64() * 1000.0,
                after[2].as_secs_f64() * 1000.0
            );
        }
    }
}

/// The pushdown is the operator's choice, never the default (issue #132): it
/// is slower where the rows are close, so a connector reports `tree` only when
/// asked to push trees down.
#[tokio::test]
async fn the_pushdown_is_off_unless_asked_for() {
    let (schema, rows) = rows("tree");
    assert!(
        !sqlite_of("tree", schema.clone(), &rows, false)
            .capabilities()
            .tree
    );
    assert!(
        sqlite_of("tree", schema.clone(), &rows, true)
            .capabilities()
            .tree
    );
    if let Some(postgres) = postgres_of("tree_default", schema, &rows, false).await {
        assert!(!postgres.capabilities().tree);
    }
}
