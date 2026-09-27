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

## What a connector never has to do

Security. Before a query reaches the connector, the server has checked the token, validated the
query against the schema narrowed to `allowed_fields`, added the row filter with the value from
the token's context, and validated the result against the full schema. A connector only ever
sees a `ValidatedQuery` that already carries every rule — it cannot forget one.

## Reference connectors

PostgreSQL (`opengrid-datasource-postgres`: the pivot as one `GROUPING SETS` statement, the
export through a cursor) and the local engine over a table in memory
(`opengrid_server::local::LocalConnector`). They are examples, not a list of supported
databases: anything that can answer the contract is one.

The contract is frozen with its first release and grows only by methods with a default.
