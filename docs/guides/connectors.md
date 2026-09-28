---
title: Connectors
description: The server knows no database — your application hands it sources through one contract.
order: 6
---

`opengrid-server` is a library. It checks tokens, narrows what each client may see, adds the
mandatory row filter and speaks HTTP. It does not know where your data lives. Your application
implements `Connector` (crate `opengrid-connector`) for whatever holds it — PostgreSQL, a file,
another service, something without a query language — and hands it to the server.

```rust
use opengrid_server::{RowFilter, Server, SourcePolicy};

let server = Server::builder()
    .source("orders", my_connector, SourcePolicy {
        allowed_fields: vec!["id".into(), "customer".into(), "amount".into()],
        row_filter: Some(RowFilter::new("tenant_id", "eq", ":tenant")),
    })
    .token(std::env::var("ORDERS_TOKEN")?, [("tenant", "acme")])
    .build()
    .await?;
axum::serve(listener, server.router()).await?;
```

## The contract

| Method | Required | Without it |
|---|---|---|
| `schema()` | yes | — |
| `capabilities()` | yes | — |
| `execute(ValidatedQuery)` | yes | — |
| `pivot(&ValidatedPivotQuery)` | no | one `execute` per level |
| `export(&ValidatedQuery, idle_limit)` | no | one `execute` with `limit` 0 to count, one to read |
| `concurrent_exports()` | no | the server picks its own bound |

Methods return `BoxFuture` — `Box::pin(async move { … })` — so the server can hold every source
as `Arc<dyn Connector>`. A source written against `SendDataSource` becomes a connector with
`FromSource(source)`.

`execute` answers with the requested page **and** `total_count`, the rows that matched before
`offset` and `limit`. The query's semantics — NULL ordering, binary string comparison, exact
decimals — are in [Query semantics](query-semantics.md); a connector that answers them
differently gives the browser different results from the engine in the tab.

## Sources that only hand out rows

A file, a list in memory, a service without a query language: implement `RowSource` — `schema()`
and `scan()`, which hands out the rows in pieces — and wrap it in `Rows`. The engine then
answers every query on the server, natively: filter, sort, group, aggregate, paging.

```rust
use opengrid_connector::Rows;

let server = Server::builder()
    .source("events", Rows::new(my_file).max_scan_rows(2_000_000), SourcePolicy::default())
    // …
```

- Each piece holds the **stored** columns of the schema, in order; derived columns
  (`"from": { "part": "year", … }`) are computed by the engine.
- `scan` gets the query's filter, the row filter already in it. It is a **hint**: a source may
  use it to hand out fewer rows; the engine applies the filter again either way.
- The rows **stream**: the server keeps only what the answer needs — the best
  `offset + limit` rows for a page, the partial sums and counts per group for a grouping, a
  counter for `total_count`. Memory follows the answer, not the source.
- `max_scan_rows` (default 1 000 000) bounds the rows or groups one answer has to **hold** —
  every match of a query without a page, say. More is a `413` (`limit_exceeded`) that says so.
  How long a scan may take is the server's timeout; the scan gives the runtime its turn between
  pieces, so the timeout fires even for a source whose pieces are always ready.

Measured (2026-09-28, Apple M4 Pro, release build, four columns generated on the fly — the time
includes making the rows; `cargo run --release -p opengrid-connector --example rows_bench`):

| Rows | Page (filter, sort, 50) | Group by country, sum + avg | Count of a filter |
|---|---|---|---|
| 1 000 000 | 0.15 s, 28 MiB | 0.17 s, 27 MiB | 0.14 s, 28 MiB |
| 10 000 000 | 1.3 s, 31 MiB | 1.6 s, 28 MiB | 1.4 s, 28 MiB |
| 100 000 000 | 13 s, 75 MiB | 16 s, 41 MiB | 14 s, 40 MiB |

Reading every row into one table first, as the tier did before, took 65 MiB for a million rows
and 452 MiB for ten million — and would take about 4.5 GiB for a hundred million.

## Proving a connector

`opengrid-conformance` runs the whole suite — every rule of the query semantics, one case each —
against any source. Load the fixture into yours, then in a test:

```rust
use opengrid_conformance::{check_source, fixture_csv, fixture_schema};
use opengrid_connector::AsSource;

// load fixture_csv() into your database under fixture_schema() first
let report = check_source(&AsSource(&my_connector)).await;
report.assert_ok(); // panics with one line per case that differs
```

All cases agree means the browser gets the same answers from your source as from the engine in
the tab. The PostgreSQL, local-engine and rows-tier references run exactly this in their tests.

## What a connector never has to do

Security. Before a query reaches the connector, the server has checked the token, validated the
query against the schema narrowed to `allowed_fields`, added the row filter with the value from
the token's context, and validated the result against the full schema. A connector only ever
sees a `ValidatedQuery` that already carries every rule — it cannot forget one.

## A server in another language

The Rust contract helps Rust programs. A server written in .NET, Java or Node implements the
HTTP [protocol](../../protocol/) instead, and proves it with the same suite over HTTP:
`opengrid-conformance --endpoint <url> --token <token>`.

## Reference connectors

PostgreSQL (`opengrid-datasource-postgres`: the pivot as one `GROUPING SETS` statement, the
export through a cursor) and the local engine over a table in memory
(`opengrid_connector::LocalConnector`, `from_csv` for a file). They are examples, not a list of supported
databases: anything that can answer the contract is one.

The contract is frozen with its first release and grows only by methods with a default.
