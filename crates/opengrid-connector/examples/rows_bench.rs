//! Measures the rows tier (issue #50): a source of `N` generated rows, one
//! query, time and — read by the caller with `/usr/bin/time -l` — peak memory.
//!
//! ```sh
//! cargo run --release -p opengrid-connector --example rows_bench -- 10000000 page
//! /usr/bin/time -l target/release/examples/rows_bench 10000000 groups
//! ```
//!
//! Queries: `page` (filter, sort, first 50), `groups` (group by country, sum
//! and average), `count` (count of a filter), `materialized` (the page, the
//! old way: every row read into one table first).

use std::time::Instant;

use opengrid_conformance::block_on;
use opengrid_connector::{
    BoxFuture, Connector, DataSourceError, LocalConnector, QueryResult, RowSource, RowStream, Rows,
    Schema, Value,
};
use opengrid_engine::datasource::LocalDataSource;
use opengrid_engine::ingest::PieceBuilder;
use opengrid_query::{Limits, Query, ValidatedFilter};
use opengrid_types::Decimal;

const SCHEMA: &str = r#"{"fields":[
    {"name":"id","type":"int64","nullable":false},
    {"name":"country","type":"utf8","nullable":false},
    {"name":"amount","type":{"decimal":{"precision":12,"scale":2}},"nullable":true},
    {"name":"qty","type":"int64","nullable":true}]}"#;
const COUNTRIES: [&str; 12] = [
    "AT", "BE", "CH", "DE", "DK", "ES", "FR", "GB", "IT", "NL", "PL", "SE",
];
const PIECE: u64 = 65_536;

struct Generated {
    rows: u64,
    schema: Schema,
}

impl RowSource for Generated {
    fn schema(&self) -> BoxFuture<'_, Result<Schema, DataSourceError>> {
        let schema = self.schema.clone();
        Box::pin(async move { Ok(schema) })
    }

    fn scan<'a>(
        &'a self,
        _filter: Option<&'a ValidatedFilter>,
    ) -> BoxFuture<'a, Result<Box<dyn RowStream + 'a>, DataSourceError>> {
        Box::pin(async move {
            let stream: Box<dyn RowStream + 'a> = Box::new(Pieces {
                source: self,
                next: 0,
            });
            Ok(stream)
        })
    }
}

struct Pieces<'a> {
    source: &'a Generated,
    next: u64,
}

impl RowStream for Pieces<'_> {
    fn next_piece(&mut self) -> BoxFuture<'_, Result<Option<QueryResult>, DataSourceError>> {
        let start = self.next;
        let end = (start + PIECE).min(self.source.rows);
        self.next = end;
        let schema = self.source.schema.clone();
        Box::pin(async move {
            if start >= end {
                return Ok(None);
            }
            Ok(Some(piece(&schema, start, end)))
        })
    }
}

/// Rows `start..end`: a deterministic spread of countries, amounts and
/// quantities, every 97th amount NULL.
fn piece(schema: &Schema, start: u64, end: u64) -> QueryResult {
    let mut columns = vec![Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for id in start..end {
        let mix = id.wrapping_mul(2_654_435_761) % 1_000_003;
        columns[0].push(Value::Int64(id as i64));
        columns[1].push(Value::Utf8(COUNTRIES[(mix % 12) as usize].into()));
        columns[2].push(if id % 97 == 0 {
            Value::Null
        } else {
            Value::Decimal(Decimal::new((mix % 1_000_000) as i128, 2))
        });
        columns[3].push(Value::Int64((mix % 20) as i64));
    }
    QueryResult::new(schema.clone(), columns, 0)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rows: u64 = args
        .next()
        .and_then(|n| n.parse().ok())
        .expect("a row count");
    let what = args.next().unwrap_or_else(|| "page".to_owned());
    let schema: Schema = opengrid_json::from_str(SCHEMA).unwrap();
    let query_json = match what.as_str() {
        "page" | "materialized" => {
            r#"{"source":"s","select":["id","country","amount"],"filter":{"field":"country","op":"eq","value":"DE"},"sort":[{"field":"amount","direction":"desc"}],"limit":50}"#
        }
        "groups" => {
            r#"{"source":"s","select":["country"],"group":["country"],"aggregate":[{"field":"amount","fn":"sum","as":"total"},{"field":"qty","fn":"avg","as":"mean"}]}"#
        }
        "count" => {
            r#"{"source":"s","filter":{"field":"qty","op":"gte","value":10},"aggregate":[{"fn":"count","as":"n"}]}"#
        }
        other => panic!("unknown query {other}"),
    };
    let query: Query = opengrid_json::from_str(query_json).unwrap();
    let limits = Limits {
        max_limit: u64::MAX,
        ..Limits::default()
    };
    let query = query.validate(&schema, &limits).expect("a valid query");

    let started = Instant::now();
    let result = if what == "materialized" {
        let mut builder = PieceBuilder::new(&schema);
        let mut start = 0;
        while start < rows {
            let end = (start + PIECE).min(rows);
            builder.push(&piece(&schema, start, end)).unwrap();
            start = end;
        }
        let connector = LocalConnector::new(LocalDataSource::new(builder.finish().unwrap()));
        block_on(connector.execute(query))
    } else {
        let connector = Rows::new(Generated { rows, schema }).max_scan_rows(u64::MAX);
        block_on(connector.execute(query))
    };
    let result = result.expect("an answer");
    println!(
        "{what} over {rows} rows: {:.2} s, {} rows answered, total_count {}",
        started.elapsed().as_secs_f64(),
        result.row_count(),
        result.total_count
    );
}
