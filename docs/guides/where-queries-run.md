---
title: Where queries run
description: The provider seam — the engine in the tab, in a worker, behind HTTP, or split between them.
order: 1
---

An element never runs a query itself. It hands the query JSON to a **provider**, and anything
with an `execute` method is one. That is how the engine can sit in the tab, in a worker, behind
HTTP, or be split across two of them without the elements knowing.

| Provider (from `loader.js`) | Where the query runs |
|---|---|
| `createLocalProvider(engine)` | The engine on the main thread. |
| `createWorkerProvider({ moduleUrl, wasmUrl })` | The engine in a module worker, started lazily, once. |
| `createRestProvider({ url, source, token })` | `POST /query/{source}` of an `opengrid-server`. |
| `createHybridProvider({ remote, planner, mode, onPlan })` | Split between a remote source and the engine in the tab. |
| `createPivotProvider({ url, source, token })` | `POST /pivot/{source}` — a whole pivot in one request. |

## On a server

`opengrid-server` answers the same query AST over HTTP. The browser sends the AST, never SQL.
A configuration names the sources, the bearer tokens and what each client may see:

```toml
[server]
address = "127.0.0.1:8081"
allowed_origins = ["http://127.0.0.1:8080"]

[[tokens]]
value = "demo-token-de"
context = { country = "DE" }

[[datasources]]
name = "orders"
type = "local-csv"   # or "postgres", with `connection` and `table`
path = "orders.csv"
schema = "orders.schema.json"
allowed_fields = ["id", "customer", "country", "amount", "qty", "ordered_on"]
row_filter = { field = "country", op = "eq", value = ":country" }
```

- A field outside `allowed_fields` does not exist for a client: the answer is the same
  `422 unknown field` as a typo.
- `row_filter` is attached to every query from the token's context and cannot be switched off.
- CORS is off by default. `allowed_origins` lists origins one by one; there is no `*`.

The full example, including a PostgreSQL variant, is in `examples/remote-demo/`:

```sh
just wasm-build-components
cargo run -p opengrid-server -- examples/remote-demo/opengrid.toml   # :8081
just serve-demo                                                      # :8080
```

## Split between the two

A source declares what it can do. The planner takes a query, gives the source the part it can
answer, and finishes the rest in the tab. `mode` — on the provider, or per element through the
`mode` attribute — is `local`, `remote`, `hybrid` or `auto`. `onPlan` receives the plan before
anything is sent, so the split is readable from outside.

## A pivot

A pivot is a set of grouping sets plus a reshaping. `createPivotProvider` sends the whole pivot
as one request; PostgreSQL answers it as a single `GROUPING SETS` statement, and the local
engine as ordinary queries with the same result. The pivot conformance cases in
`crates/opengrid-conformance/pivot-cases/` pin that result.
