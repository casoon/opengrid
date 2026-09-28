//! An opengrid server around the library (issue #49): configuration is code.
//!
//! ```sh
//! cargo run -p opengrid-example-server -- demo            # :8081, the remote demo
//! cargo run -p opengrid-example-server -- demo-postgres   # :8081, the same over PostgreSQL
//! cargo run -p opengrid-example-server -- demo-sqlite     # :8081, the same from a SQLite file
//! cargo run -p opengrid-example-server -- e2e             # :8082, the Playwright suite
//! ```
//!
//! Each preset is a function that builds a server: its sources as connectors,
//! its tokens, what each caller may see. An application writes one of these
//! for its own data — the server itself knows no database.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use opengrid_connector::{Connector, LocalConnector};
use opengrid_datasource_postgres::PostgresDataSource;
use opengrid_server::{RowFilter, Server, ServerBuilder, SourcePolicy};
use opengrid_types::Schema;

type Error = Box<dyn std::error::Error>;

/// The demo's orders: where they come from, and the columns a client may name.
struct Orders {
    connector: Arc<dyn Connector>,
    allowed: Vec<&'static str>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let preset = std::env::args().nth(1).unwrap_or_default();
    let (address, server) = match preset.as_str() {
        "demo" => ("127.0.0.1:8081", demo(csv_orders()?)),
        "demo-postgres" => ("127.0.0.1:8081", demo(postgres_orders()?)),
        "demo-sqlite" => ("127.0.0.1:8081", demo(sqlite_orders()?)),
        "e2e" => ("127.0.0.1:8082", e2e()?),
        _ => {
            eprintln!("usage: opengrid-example-server demo | demo-postgres | demo-sqlite | e2e");
            std::process::exit(2);
        }
    };
    let server = server.build().await?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!(
        "opengrid-example-server {preset} on http://{} — sources: {}",
        listener.local_addr()?,
        server.state().registry.names().join(", ")
    );
    axum::serve(listener, server.router()).await?;
    Ok(())
}

/// The repository, so the fixture paths do not depend on the working directory.
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_schema_path() -> PathBuf {
    repo().join("crates/opengrid-conformance/data/orders.schema.json")
}

/// A token from the environment, or the demo's value — in a real deployment
/// the secret comes from the environment only.
fn token(variable: &str, demo: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| demo.to_owned())
}

/// The remote demo (examples/remote-demo): two tenants, each sees its country.
///
/// `allowed_fields` are the only columns a client may name at all; the row
/// filter is appended to every query with the value from the caller's token.
fn demo(orders: Orders) -> ServerBuilder {
    Server::builder()
        // `just serve-demo` serves the page from :8080, a different origin.
        // No `*`: with a bearer token that would hand the token to every page.
        .allow_origin("http://127.0.0.1:8080")
        .allow_origin("http://localhost:8080")
        .token(token("DEMO_TOKEN_DE", "demo-token-de"), [("country", "DE")])
        .token(token("DEMO_TOKEN_FR", "demo-token-fr"), [("country", "FR")])
        .shared_source(
            "orders",
            orders.connector,
            SourcePolicy {
                allowed_fields: orders.allowed.into_iter().map(String::from).collect(),
                row_filter: Some(RowFilter::new("country", "eq", ":country")),
            },
        )
}

/// The conformance data set through the local engine: demonstrable before a
/// database is involved.
fn csv_orders() -> Result<Orders, Error> {
    let csv = repo().join("crates/opengrid-conformance/data/orders.csv");
    let connector = LocalConnector::from_csv(&csv, &fixture_schema_path())?;
    let allowed = vec![
        "id",
        "customer",
        "country",
        "amount",
        "qty",
        "ordered_on",
        "ordered_year",
    ];
    Ok(Orders {
        connector: Arc::new(connector),
        allowed,
    })
}

/// The same rows in PostgreSQL (`psql -d postgres -f examples/remote-demo/setup.sql`).
/// The browser cannot tell the difference.
fn postgres_orders() -> Result<Orders, Error> {
    let url = std::env::var("DEMO_DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost dbname=postgres".to_owned());
    let schema: Schema = opengrid_json::from_str(&std::fs::read_to_string(fixture_schema_path())?)?;
    let connector = PostgresDataSource::connect(&url, "opengrid_demo_orders", schema)?;
    let allowed = vec!["id", "customer", "country", "amount", "qty", "ordered_on"];
    Ok(Orders {
        connector: Arc::new(connector),
        allowed,
    })
}

/// The same rows in a SQLite file, `target/demo-orders.sqlite` — written from
/// the fixture on the first start. No installation: SQLite is compiled in.
fn sqlite_orders() -> Result<Orders, Error> {
    use opengrid_connector::QueryResult;
    use opengrid_connector_sqlite::{Connection, SqliteConnector, create_table, insert};
    use opengrid_engine::ingest::{CsvOptions, load_csv};

    let schema: Schema = opengrid_json::from_str(&std::fs::read_to_string(fixture_schema_path())?)?;
    let path = repo().join("target/demo-orders.sqlite");
    if !path.exists() {
        std::fs::create_dir_all(repo().join("target"))?;
        let stored = schema.stored();
        let csv = std::fs::read(repo().join("crates/opengrid-conformance/data/orders.csv"))?;
        let rows = load_csv(&csv, &stored, CsvOptions::default())?;
        let mut connection = Connection::open(&path)?;
        create_table(&connection, "orders", &schema)?;
        insert(
            &mut connection,
            "orders",
            &schema,
            &QueryResult::new(stored, rows.to_values(), 0),
        )?;
    }
    let connector = SqliteConnector::open(&path, "orders", schema)?;
    let allowed = vec![
        "id",
        "customer",
        "country",
        "amount",
        "qty",
        "ordered_on",
        "ordered_year",
    ];
    Ok(Orders {
        connector: Arc::new(connector),
        allowed,
    })
}

/// The server the Playwright suite runs against (tests/e2e/playwright.config.js).
///
/// No row filter: the specs compare the grid's rows with the conformance data
/// set, so the answer has to be the whole table. The token and the narrowed
/// fields stay — they are not optional, and the page has to work with them.
/// `export` is the export spec's 100 000 rows, written by
/// tests/e2e/fixtures/write-export-data.mjs before the server starts.
fn e2e() -> Result<ServerBuilder, Error> {
    let orders = LocalConnector::from_csv(
        &repo().join("crates/opengrid-conformance/data/orders.csv"),
        &fixture_schema_path(),
    )?;
    let export = LocalConnector::from_csv(
        &repo().join("target/e2e/export.csv"),
        &repo().join("tests/e2e/fixtures/export.schema.json"),
    )?;
    let allowed = [
        "id",
        "customer",
        "country",
        "amount",
        "qty",
        "ordered_on",
        "ordered_year",
    ];
    Ok(Server::builder()
        .allow_origin("http://127.0.0.1:8080")
        .allow_origin("http://localhost:8080")
        .token("e2e-token", std::iter::empty::<(&str, &str)>())
        .source(
            "orders",
            orders,
            SourcePolicy {
                allowed_fields: allowed.map(String::from).to_vec(),
                row_filter: None,
            },
        )
        .source("export", export, SourcePolicy::default()))
}
