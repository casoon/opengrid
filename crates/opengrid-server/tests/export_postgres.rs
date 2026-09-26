//! `POST /export/{source}` against a real PostgreSQL (issue #2): the tenant
//! filter in the compiled statement, the count before the first byte, and a
//! client that goes away in the middle.
//!
//! Skips itself without a database, like the PostgreSQL tests of
//! `opengrid-datasource-postgres`; CI's `postgres` job runs it. The table has
//! 200 000 rows, so an export is many pieces and a client can leave between
//! them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use opengrid_server::{AppState, Config, Registry, router};
use tower::ServiceExt;

const DE: &str = "token-de";
const FR: &str = "token-fr";
const ALL: &str = "token-all";
const ROWS: i64 = 200_000;

fn connection() -> String {
    std::env::var("OPENGRID_TEST_PG")
        .unwrap_or_else(|_| "host=localhost dbname=postgres".to_owned())
}

/// `url` with an `application_name`, so the test finds the server's backends in
/// `pg_stat_activity` and nobody else's.
fn named(url: &str, name: &str) -> String {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        let joint = if url.contains('?') { '&' } else { '?' };
        format!("{url}{joint}application_name={name}")
    } else {
        format!("{url} application_name={name}")
    }
}

async fn connect() -> Option<tokio_postgres::Client> {
    match tokio_postgres::connect(&connection(), tokio_postgres::NoTls).await {
        Ok((client, connection)) => {
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Some(client)
        }
        Err(error) => {
            eprintln!(
                "skipping the PostgreSQL export tests: {error}\n\
                 set OPENGRID_TEST_PG to a connection string to run them"
            );
            None
        }
    }
}

/// The states of the backends connected under `application`.
async fn states_of(client: &tokio_postgres::Client, application: &str) -> Vec<String> {
    client
        .query(
            "SELECT coalesce(state, '') FROM pg_stat_activity WHERE application_name = $1",
            &[&application],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect()
}

/// Polls `states_of` until it is `expected`, for up to five seconds.
async fn settles_to(
    client: &tokio_postgres::Client,
    application: &str,
    expected: &[&str],
) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut states = states_of(client, application).await;
    while states != expected && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        states = states_of(client, application).await;
    }
    states
}

/// `table` with `ROWS` rows: every other one `DE`, the rest `FR`.
async fn create_table(client: &tokio_postgres::Client, table: &str) {
    client
        .batch_execute(&format!(
            "DROP TABLE IF EXISTS \"{table}\";
             CREATE TABLE \"{table}\" (id bigint NOT NULL, country text, amount numeric(12,2), note text);
             INSERT INTO \"{table}\"
               SELECT g, CASE WHEN g % 2 = 0 THEN 'DE' ELSE 'FR' END, g / 100.0, 'row ' || g
               FROM generate_series(1, {ROWS}) AS g;"
        ))
        .await
        .expect("the fixture table");
}

async fn drop_table(client: &tokio_postgres::Client, table: &str) {
    client
        .batch_execute(&format!("DROP TABLE \"{table}\";"))
        .await
        .expect("drop table");
}

/// The server over `table`, the tenant filter on `country` — a column its
/// callers cannot see — and at most 150 000 rows per export.
fn app(table: &str, application: &str) -> axum::Router {
    app_with(table, application, "timeout_ms = 10000")
}

fn app_with_timeout(table: &str, application: &str, timeout_ms: u64) -> axum::Router {
    app_with(table, application, &format!("timeout_ms = {timeout_ms}"))
}

/// The server with `settings` added to `[server]`.
fn app_with(table: &str, application: &str, settings: &str) -> axum::Router {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the repository")
        .to_path_buf();
    let dir: PathBuf = root.join(format!("target/opengrid-server-{table}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("schema.json"),
        r#"{ "fields": [
            { "name": "id", "type": "int64", "nullable": false },
            { "name": "country", "type": "utf8", "nullable": true },
            { "name": "amount", "type": { "decimal": { "precision": 12, "scale": 2 } }, "nullable": true },
            { "name": "note", "type": "utf8", "nullable": true }
        ] }"#,
    )
    .unwrap();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        format!(
            r#"
[server]
max_export_rows = 150000
{settings}

[[tokens]]
value = "{DE}"
context = {{ country = "DE" }}

[[tokens]]
value = "{FR}"
context = {{ country = "FR" }}

[[tokens]]
value = "{ALL}"
context = {{ country = "%" }}

[[datasources]]
name = "orders"
type = "postgres"
connection = {connection:?}
table = "{table}"
schema = "schema.json"
allowed_fields = ["id", "amount", "note"]
row_filter = {{ field = "country", op = "eq", value = ":country" }}
"#,
            connection = named(&connection(), application),
        ),
    )
    .unwrap();
    let config = Config::load(&path).expect("configuration");
    let registry = Registry::build(&config, &dir).expect("registry");
    router(Arc::new(AppState::new(&config, registry)))
}

async fn export(app: axum::Router, token: &str, body: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/export/orders?bom=false")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap(),
    )
    .await
    .expect("the router answers")
}

const BY_ID: &str = r#"{"source":"orders","select":["id","amount","note"],"sort":[{"field":"id","direction":"asc"}]}"#;

/// **The tenant test, in SQL.** Each tenant's export holds exactly its rows —
/// 100 000 of them, ten pieces through the cursor — and not one of the other's.
#[tokio::test]
async fn a_tenant_exports_its_own_rows_from_postgresql_and_no_other() {
    let Some(client) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_tenant";
    create_table(&client, table).await;
    let app = app(table, "opengrid_server_export_tenant");

    for (token, country) in [(DE, "DE"), (FR, "FR")] {
        let response = export(app.clone(), token, BY_ID).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-total-count"], "100000");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let exported: Vec<i64> = text
            .lines()
            .skip(1)
            .map(|line| line.split(',').next().unwrap().parse().unwrap())
            .collect();

        let truth: Vec<i64> = client
            .query(
                &format!("SELECT id FROM \"{table}\" WHERE country = $1 ORDER BY id"),
                &[&country],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(exported.len(), 100_000, "{country}");
        assert!(
            exported == truth,
            "{country}: the export is not the tenant's rows"
        );
    }

    // A token whose context value is not a country sees nothing — the value is
    // a parameter, never SQL: `%` is not a wildcard.
    let response = export(app.clone(), ALL, BY_ID).await;
    assert_eq!(response.headers()["x-total-count"], "0");

    drop_table(&client, table).await;
}

/// The count decides before the first byte: 200 000 rows are more than the
/// 150 000 allowed, and that is a status, not a file cut short.
#[tokio::test]
async fn too_many_rows_are_refused_before_the_first_byte() {
    let Some(client) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_bound";
    create_table(&client, table).await;
    // Every row the tenant's, so the export would have all 200 000.
    client
        .batch_execute(&format!("UPDATE \"{table}\" SET country = 'DE';"))
        .await
        .unwrap();
    let application = "opengrid_server_export_bound";
    let app = app(table, application);

    let response = export(app.clone(), DE, BY_ID).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("200000 rows"), "{text}");

    // A refusal is a clean end: the connection goes back to the pool (`app`
    // is still alive, and the pool with it), out of the transaction.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut states = states_of(&client, application).await;
    while states != ["idle"] && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        states = states_of(&client, application).await;
    }
    assert_eq!(states, ["idle"]);
    drop(app);

    drop_table(&client, table).await;
}

/// **A client that goes away ends the database's work.** While the client
/// reads nothing, the export waits with its cursor open — the channel is
/// bounded, so nothing piles up. When the client drops the body, the next
/// piece cannot be sent, the export is dropped, and its backend goes: the
/// transaction is rolled back and the cursor closed with it.
#[tokio::test]
async fn a_client_that_leaves_ends_the_query() {
    let Some(client) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_abort";
    let application = "opengrid_server_export_abort";
    create_table(&client, table).await;
    let app = app(table, application);

    let backends = || async {
        client
            .query(
                "SELECT coalesce(state, '') FROM pg_stat_activity WHERE application_name = $1",
                &[&application],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>()
    };
    let within = |seconds: u64| Instant::now() + Duration::from_secs(seconds);

    // `app` stays alive to the end: with it the pool, so a connection handed
    // back would show as `idle` rather than go.
    let response = export(app.clone(), DE, BY_ID).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let first = body.frame().await.expect("a frame").expect("data");
    assert!(first.data_ref().is_some_and(|data| !data.is_empty()));

    // The export is held: its transaction is open, and it is not reading on.
    let deadline = within(5);
    let mut states = backends().await;
    while states != ["idle in transaction"] && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        states = backends().await;
    }
    assert_eq!(states, ["idle in transaction"]);

    drop(body);
    let deadline = within(5);
    let mut states = backends().await;
    while !states.is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        states = backends().await;
    }
    assert_eq!(
        states,
        Vec::<String>::new(),
        "the export's backend is still there"
    );

    drop(app);
    drop_table(&client, table).await;
}

/// **A failure after the first byte breaks the body off.** The status is out
/// by then, so the only honest signal left is a body that does not end: the
/// client gets an error, never a shorter file that looks complete.
#[tokio::test]
async fn a_failure_in_the_middle_is_a_broken_body_not_a_short_file() {
    let Some(client) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_broken";
    let application = "opengrid_server_export_broken";
    create_table(&client, table).await;
    let app = app(table, application);

    let response = export(app, DE, BY_ID).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    body.frame().await.expect("a frame").expect("data");

    // The database goes away under the export, between two of its fetches.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let ended: Vec<bool> = client
            .query(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1",
                &[&application],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        if ended.contains(&true) || Instant::now() > deadline {
            assert!(ended.contains(&true), "no backend to end");
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let mut rows = 0;
    let outcome = loop {
        match body.frame().await {
            None => break Ok(rows),
            Some(Ok(frame)) => {
                rows += frame
                    .data_ref()
                    .map_or(0, |data| data.iter().filter(|&&b| b == b'\n').count());
            }
            Some(Err(error)) => break Err(error.to_string()),
        }
    };
    let error = outcome.expect_err("the body ended as if it were whole");
    assert!(error.contains("broke off"), "{error}");
    assert!(rows < 100_000, "{rows} lines arrived before the break");

    drop_table(&client, table).await;
}

/// `timeout_ms` bounds the time to the first byte. An export that has not
/// started by then is a `413` with a sentence — and the statement it had
/// started is cancelled in the database, not only forgotten here. The count
/// here waits on a lock another connection holds, which is a statement that
/// would otherwise run for as long as the lock is held.
#[tokio::test]
async fn an_export_that_does_not_start_in_time_is_cancelled_in_the_database() {
    let Some(client) = connect().await else {
        return;
    };
    let Some(locker) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_slow";
    let application = "opengrid_server_export_slow";
    create_table(&client, table).await;
    let app = app_with_timeout(table, application, 500);

    locker
        .batch_execute(&format!(
            "BEGIN; LOCK TABLE \"{table}\" IN ACCESS EXCLUSIVE MODE;"
        ))
        .await
        .unwrap();

    let response = export(app, DE, BY_ID).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("longer than 500 ms to start"), "{text}");

    // The lock is still held: a count that was not cancelled would still be
    // waiting for it.
    let waiting = || async {
        client
            .query(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1 AND state = 'active'",
                &[&application],
            )
            .await
            .unwrap()[0]
            .get::<_, i64>(0)
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut left = waiting().await;
    while left > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        left = waiting().await;
    }
    assert_eq!(
        left, 0,
        "the export's count is still waiting in the database"
    );

    locker.batch_execute("ROLLBACK;").await.unwrap();
    drop_table(&client, table).await;
}

/// **A client that stops reading is cut off.** It keeps the channel full, so
/// the next piece waits; after `timeout_ms` the body is broken off and the
/// export dropped — its connection and transaction gone while the client
/// still holds the body, not whenever it lets go.
#[tokio::test]
async fn a_client_that_stops_reading_is_cut_off_within_the_deadline() {
    let Some(client) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_stalled";
    let application = "opengrid_server_export_stalled";
    create_table(&client, table).await;
    let app = app_with_timeout(table, application, 500);

    let response = export(app.clone(), DE, BY_ID).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    body.frame().await.expect("a frame").expect("data");
    let stalled = Instant::now();

    // The client reads nothing more, and holds on to the body.
    assert_eq!(
        settles_to(&client, application, &[]).await,
        Vec::<String>::new()
    );
    assert!(
        stalled.elapsed() < Duration::from_secs(3),
        "{:?}",
        stalled.elapsed()
    );

    // What it reads when it comes back: what was sent, then the break.
    let error = loop {
        match body.frame().await {
            Some(Ok(_)) => continue,
            Some(Err(error)) => break error.to_string(),
            None => panic!("the body ended as if it were whole"),
        }
    };
    assert!(error.contains("took no piece for 500 ms"), "{error}");

    drop(app);
    drop_table(&client, table).await;
}

/// **One export more than `max_concurrent_exports` is a 503**, before any
/// database work — and the place is free again once an export ends.
#[tokio::test]
async fn one_export_too_many_is_turned_away_before_the_database() {
    let Some(client) = connect().await else {
        return;
    };
    let table = "opengrid_server_export_crowded";
    let application = "opengrid_server_export_crowded";
    create_table(&client, table).await;
    let app = app_with(table, application, "max_concurrent_exports = 1");

    // One export, held: its client reads a piece and waits.
    let held = export(app.clone(), DE, BY_ID).await;
    assert_eq!(held.status(), StatusCode::OK);
    let mut held = held.into_body();
    held.frame().await.expect("a frame").expect("data");

    let turned = export(app.clone(), FR, BY_ID).await;
    assert_eq!(turned.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(turned.headers()[header::CONTENT_TYPE], "application/json");
    let bytes = turned.into_body().collect().await.unwrap().to_bytes();
    let error =
        opengrid_datasource::wire::WireError::from_json(std::str::from_utf8(&bytes).unwrap())
            .expect("the error form");
    // `busy`, not `limit_exceeded`: the export is fine, the server is full.
    assert_eq!(error.code, opengrid_datasource::wire::ErrorCode::Busy);
    assert!(
        error.message.contains("max_concurrent_exports"),
        "{}",
        error.message
    );
    // Turned away before the database: still the one connection of the first,
    // which settles waiting for its client.
    assert_eq!(
        settles_to(&client, application, &["idle in transaction"]).await,
        ["idle in transaction"]
    );

    // The first ends; its place is free.
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        let response = export(app.clone(), FR, BY_ID).await;
        if response.status() != StatusCode::SERVICE_UNAVAILABLE || Instant::now() > deadline {
            break response.status();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(status, StatusCode::OK);

    drop(app);
    drop_table(&client, table).await;
}
