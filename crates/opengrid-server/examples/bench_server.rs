//! The server behind the browser measurements of `bench/` (issue #421): the same rows as
//! `bench/src/data.js`, one source `orders`, token `bench`, answering a page served from
//! `http://localhost:4173` (CORS for that origin and no other). It prints `listening <port>` and
//! serves until it is killed.
//!
//! ```sh
//! cargo run --release -p opengrid-server --example bench_server -- sqlite 1000000 8099
//! DATABASE_URL=postgres://$USER@localhost:5432/opengrid_dev \
//!   cargo run --release -p opengrid-server --example bench_server -- postgres 1000000 8099
//! ```
//!
//! The rows: `amount` is `((id * 7919) % 10000019) / 100 + 0.5`, unique for every id up to ten
//! million, so every grid sorts to the same first row. The tables are kept for the next run
//! (`target/harness-orders-<rows>.sqlite`, `harness_orders_<rows>` in PostgreSQL). Indexes as a
//! page tuned for these requests would have them: the key `id`, and `(country, amount)`.

use std::path::PathBuf;

use opengrid_connector::Connector;
use opengrid_connector_sqlite::{Connection, SqliteConnector, create_table};
use opengrid_datasource_postgres::PostgresDataSource;
use opengrid_server::{Server, SourcePolicy};
use opengrid_types::Schema;

type Error = Box<dyn std::error::Error + Send + Sync>;

const FIELDS: [&str; 6] = ["id", "customer", "country", "amount", "qty", "ordered_on"];
/// 2026-03-01 in days since 1970-01-01.
const MARCH_FIRST: i64 = 20_513;
const SCHEMA: &str = r#"{"fields":[
  {"name":"id","type":"int64","nullable":false},
  {"name":"customer","type":"utf8","nullable":true},
  {"name":"country","type":"utf8","nullable":true},
  {"name":"amount","type":"float64","nullable":true},
  {"name":"qty","type":"int64","nullable":true},
  {"name":"ordered_on","type":"date","nullable":true}]}"#;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let (kind, rows, port) = match (
        args.get(1).map(String::as_str),
        args.get(2).and_then(|n| n.parse::<u64>().ok()),
        args.get(3).and_then(|n| n.parse::<u16>().ok()),
    ) {
        (Some(kind @ ("postgres" | "sqlite")), Some(rows), Some(port)) => (kind, rows, port),
        _ => {
            eprintln!("usage: bench_server postgres|sqlite <rows> <port>");
            std::process::exit(2);
        }
    };
    let schema: Schema = opengrid_json::from_str(SCHEMA)?;
    let connector: Box<dyn Connector> = if kind == "postgres" {
        Box::new(postgres(rows, schema).await?)
    } else {
        Box::new(sqlite(rows, schema)?)
    };
    let server = Server::builder()
        .token("bench", std::iter::empty::<(&str, &str)>())
        .allow_origin("http://localhost:4173")
        .timeout(std::time::Duration::from_secs(120))
        .shared_source(
            "orders",
            connector.into(),
            SourcePolicy {
                allowed_fields: FIELDS.iter().map(|f| f.to_string()).collect(),
                row_filter: None,
            },
        )
        .build()
        .await?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("listening {port}");
    axum::serve(listener, server.router()).await?;
    Ok(())
}

fn repo() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn postgres(rows: u64, schema: Schema) -> Result<PostgresDataSource, Error> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost dbname=opengrid_dev".to_owned());
    let table = format!("harness_orders_{rows}");
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let present: i64 = client
        .query_one(
            "SELECT count(*) FROM pg_tables WHERE schemaname = 'public' AND tablename = $1",
            &[&table],
        )
        .await?
        .get(0);
    if present == 0 {
        eprintln!("writing {table} …");
        client
            .batch_execute(&format!(
                r#"CREATE TABLE {table} (
                     "id" bigint NOT NULL, "customer" text, "country" text,
                     "amount" double precision, "qty" bigint, "ordered_on" date);
                   INSERT INTO {table}
                     SELECT i, 'Customer ' || (i % 500),
                            (ARRAY['DE','FR','PL','NL','ES','IT','AT','CH'])[i % 8 + 1],
                            ((i::bigint * 7919) % 10000019) / 100.0 + 0.5, i % 40,
                            date '2026-03-01' + (i % 28)::int
                     FROM generate_series(1, {rows}) AS i;
                   ALTER TABLE {table} ADD PRIMARY KEY ("id");
                   CREATE INDEX ON {table} ("country", "amount");"#
            ))
            .await?;
        client
            .batch_execute(&format!("VACUUM ANALYZE {table}"))
            .await?;
    }
    Ok(PostgresDataSource::connect(&url, &table, schema)?)
}

fn sqlite(rows: u64, schema: Schema) -> Result<SqliteConnector, Error> {
    let path = repo().join(format!("target/harness-orders-{rows}.sqlite"));
    if !path.exists() {
        eprintln!("writing {} …", path.display());
        std::fs::create_dir_all(repo().join("target"))?;
        let partial = path.with_extension("partial");
        let _ = std::fs::remove_file(&partial);
        let connection = Connection::open(&partial)?;
        create_table(&connection, "orders", &schema)?;
        connection.execute_batch(&format!(
            r#"PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
               WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < {rows})
               INSERT INTO orders
                 SELECT i, 'Customer ' || (i % 500),
                        CASE i % 8 WHEN 0 THEN 'DE' WHEN 1 THEN 'FR' WHEN 2 THEN 'PL' WHEN 3 THEN 'NL'
                                   WHEN 4 THEN 'ES' WHEN 5 THEN 'IT' WHEN 6 THEN 'AT' ELSE 'CH' END,
                        ((i * 7919) % 10000019) / 100.0 + 0.5, i % 40, {MARCH_FIRST} + i % 28
                 FROM s;
               CREATE UNIQUE INDEX orders_id ON orders ("id");
               CREATE INDEX orders_country_amount ON orders ("country", "amount");
               ANALYZE;"#
        ))?;
        drop(connection);
        std::fs::rename(&partial, &path)?;
    }
    Ok(SqliteConnector::open(&path, "orders", schema)?)
}
