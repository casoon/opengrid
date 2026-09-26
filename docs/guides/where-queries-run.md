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
| `createRestProvider({ url, source, token })` | `POST /query/{source}` of an `opengrid-server`; `export(query, options)` streams a whole export from `POST /export/{source}`. |
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

`[server]` also takes `max_payload_bytes` (64 KiB), `timeout_ms` (10 000), `max_limit`
(10 000 rows a page), `max_depth`, the pivot bounds and `max_export_rows` (1 000 000, below).

The full example, including a PostgreSQL variant, is in `examples/remote-demo/`:

```sh
just wasm-build-components
cargo run -p opengrid-server -- examples/remote-demo/opengrid.toml   # :8081
just serve-demo                                                      # :8080
```

## Exporting from a server

`POST /export/{source}` answers every row of a query as a file, streamed. The request is the
one `POST /query/{source}` takes — the same body, and the same path through the token,
`allowed_fields`, the tenant's `row_filter` and both validations. What differs is the bound:
`max_export_rows` takes the place of `max_limit`, because an export is every match, not a page.

```sh
curl -X POST 'http://127.0.0.1:8081/export/orders?format=csv&delimiter=%3B' \
  -H 'Authorization: Bearer demo-token-de' -H 'Content-Type: application/json' \
  --data '{"source":"orders","select":["id","customer","amount"],"sort":[{"field":"id","direction":"asc"}]}' \
  -o orders.csv
```

| | |
|---|---|
| Format | `?format=csv` or `?format=json`. Without it, whichever of `text/csv` and `application/json` `Accept` prefers; without either, CSV. |
| CSV options | `delimiter`, `bom`, `protectFormulas`, `null` as parameters — the names `exportRows` takes, the rules of `opengrid-export`. A wrong one, an unknown parameter, and a CSV option on a JSON export are a `400` before anything runs. |
| Headers | `Content-Type`; `Content-Disposition: attachment` with the source's name (`orders.csv`, an ASCII `filename` and the exact name as `filename*`); `X-Total-Count`, the rows that follow. With `allowed_origins`, a page may read the last two. |
| Too many rows | More than `max_export_rows` (1 000 000 by default) is a `413` with a sentence **before the first byte** — never a file cut short. The rows are counted first, in the same `REPEATABLE READ` snapshot the cursor then reads, so the count is the number of rows that come. A limit inside the cursor would only notice after that many rows had been sent. |
| `timeout_ms` | Bounds the time to the first byte, and then each single fetch from the source. The whole export is bounded by `max_export_rows`, not by a clock: a million rows to a slow client take longer than any one query. A statement still running when a timeout hits is cancelled in the database. |
| Streaming | PostgreSQL is read through a cursor, 10 000 rows at a time, into a bounded response body that holds two pieces. A slow client slows the reading down instead of filling memory; a client that goes away is noticed at the next piece, and the transaction and its cursor end. The local engine runs the query once and hands out slices of its answer. |
| A failure midway | After the first byte there is no status left to send, so the connection is broken off without the end of the body: a client sees an error, never a shorter file that looks whole. |

**Memory, measured** (2026-09-26, Apple M4 Pro, PostgreSQL 16 on the same machine, release
build, six columns — `bigint`, `text`, `numeric(12,2)`, `date`, `timestamptz`, `text`):

| Export | Size | Time | Server's peak RSS |
|---|---|---|---|
| — (started, idle) | | | 7 MiB |
| 100 000 rows, CSV | 6.9 MB | 0.5 s | 22 MiB |
| 1 000 000 rows, CSV | 71.5 MB | 4.2 s | 29 MiB |
| 1 000 000 rows, JSON | 131.5 MB | 3.8 s | 33 MiB |
| 1 000 000 rows, CSV, client reading 5 MB/s | 71.5 MB | 13.6 s | 39 MiB |

Ten times the rows, the same memory within a few MiB — what the process keeps is its allocator's
working set, not the export. Writing the CSV takes about 0.9 s of the 4.2 s for a million rows;
the rest is PostgreSQL and reading its text, so the writer stays as it is.

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
