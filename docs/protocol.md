---
title: The server protocol
sidebarLabel: Server protocol
description: What an opengrid server speaks over HTTP — for writing one in any language, and proving it with the conformance runner.
order: 2
---

The browser talks to a server through four endpoints. `opengrid-server` is one implementation;
a server in .NET, Java, Node or anything else that follows this page is another, and the
browser cannot tell them apart. The **conformance runner** proves it (at the end).

## Requests

Every request carries `Authorization: Bearer <token>`. Bodies are JSON
(`Content-Type: application/json`).

| Endpoint | Body | Answer |
|---|---|---|
| `POST /query/{source}` | a query | a result |
| `POST /pivot/{source}` | a pivot query | a pivot result |
| `GET /source/{source}` | — | `{ name, schema, capabilities, pivot_limits }` — `pivot_limits` is `{ max_column_dimensions, max_columns, max_rows, max_cells }` |
| `POST /export/{source}` | a query | every row as a file, streamed |

The body's `source` must equal the path's; a mismatch is a `422` at `source`.

### The query object

```json
{
  "source": "orders",
  "select": ["id", "customer", "amount"],
  "filter": { "and": [
    { "field": "country", "op": "eq", "value": "DE" },
    { "field": "amount", "op": "gte", "value": "10.00" },
    { "not": { "field": "note", "op": "is_null" } }
  ] },
  "group": [],
  "aggregate": [{ "field": "amount", "fn": "sum", "as": "total" }],
  "sort": [{ "field": "id", "direction": "asc", "nulls": "last" }],
  "offset": 0,
  "limit": 50
}
```

- Operators: `eq`, `ne`, `lt`, `lte`, `gt`, `gte`, `in` (value is an array), `contains`,
  `starts_with`, `is_null`, `is_not_null` (no value). Combine with `and`, `or`, `not`.
- Aggregates: `count` (without `field`: every row), `sum`, `avg`, `min`, `max`.
- `offset` needs a `sort`. Values are JSON, read against the column's type: a decimal as a
  string (`"10.00"`), a date as `"2026-01-31"`, a timestamp as RFC 3339 in UTC.
- What each of these means exactly — NULL, ordering, comparison, rounding — is
  [Query semantics](guides/query-semantics.md). A server that answers differently gives the
  browser different results from the engine in the tab.

A query may ask for **one level of a tree** (E38): `tree: { key?, parent, under?, scope? }` — `key`
(default `"id"`) and `parent` name the hierarchy, `under` the node whose children are asked
for; without it, the roots. A root is a node whose parent is NULL or names no node (an
orphan); a key twice or a cycle is an error. With a filter the level shows the matches **and
their ancestors**, the ancestors that do not match as context. A sort orders siblings, ties by
the key; paging is among siblings. The answer is the ordinary result of that level plus
`tree: { children, match, matches, orphans }` — per row its visible children and whether it
matches, and for the whole tree the matches and the orphans. A tree query cannot also group.
`scope` is a filter that decides which rows the tree **consists of**: a row outside it is
neither a match nor context nor a child, and a node whose parent is outside is an orphan. The
server puts its mandatory row filter (E16) there, not on the query's filter — on the filter it
would only stop matches, and the ancestors shown as context could be another tenant's rows.
Every source answers a tree through the server: one that cannot by itself (the capability
`tree`) is asked for the rows of the scope, and the engine answers the level. A tree answers in
JSON even where the binary form is preferred — the binary form has no place for its part yet.

The pivot query is `{ source, rows, columns, values: [{ field, fn, as }], filter?, sort? }`.

`sort` orders the rows of a level among their siblings: `[{ field, by?, direction }]`, where
`field` names a row dimension — the level — and `direction` is `asc` or `desc`. Without `by`
the level is ordered by its own values; `by` names a measure alias, and the level is ordered by
that measure over the whole row, across every column value, computed from the raw rows. A
subtotal stays after its group and the grand total last; NULL sorts last in either direction;
ties are broken by the level's values, ascending. A level without an entry is ascending by its
values — so a pivot without `sort` answers as it always did, and a client sends the key only
when there is one.

## Results

`POST /query` and `POST /pivot` answer in the form the request's `Accept` asks for:

- **JSON** (default, `application/json`), column-oriented, the type on each column:

  ```json
  {
    "total_count": 312,
    "row_count": 2,
    "columns": [
      { "name": "customer", "type": "utf8", "nullable": true, "values": ["Alpha", null] },
      { "name": "amount", "type": { "decimal": { "precision": 12, "scale": 2 } },
        "nullable": true, "values": ["10.00", "20.50"] }
    ]
  }
  ```

  `total_count` is the rows that matched **before** `offset` and `limit`. A decimal is a
  string; `NaN`, `Infinity` and `-Infinity` in a `float64` column are those strings.
- **Binary** (`application/vnd.opengrid.columns`, when `Accept` names it and not with `q=0`):
  the same columns as little-endian buffers, 8-byte aligned — the layout is documented in
  `crates/opengrid-columns/src/wire.rs` (version 1). A server may leave it out and answer JSON;
  the browser reads both.

Every answer carries `Vary: Accept`. The pivot's JSON form is
`{ row_dimensions, columns: [{ path, measure }], levels, result }`, `result` being the form
above and `levels[i]` how many row dimensions row `i` sets (the grand total is `0`).

## Errors

Always JSON, whatever `Accept` said:

```json
{ "error": { "code": "validation", "message": "unknown field \"note\"", "path": "select[2]" } }
```

| `code` | Status | When |
|---|---|---|
| `malformed` | 400 | The body is not a readable query. |
| `unauthorized` | 401 | No token, or not a valid one — one answer for every way of getting it wrong. |
| `unknown_source` | 404 | No source of that name. |
| `validation` | 422 | The query does not fit the schema the caller may see; `path` says where. |
| `limit_exceeded` | 413 | Too big: the body, the page (`max_limit`), the time, an export's rows. |
| `busy` | 503 | Too many exports running; the same request may succeed later. |
| `backend` | 502 | The source behind the server failed. |

## Export

`POST /export/{source}` takes the query body of `/query` and answers every row as a file:
`?format=csv` or `?format=json`, CSV options `delimiter`, `bom`, `protectFormulas`, `null`.
The headers carry `Content-Disposition: attachment` and `X-Total-Count`, the rows that follow.
Everything that can still be a status is decided before the first byte; a failure after it
breaks the connection off instead of ending a shorter file. The details are in
[Where queries run](guides/where-queries-run.md#exporting-from-a-server).

## The duties of a server

A server that holds other people's data owes the browser four things, whatever it is written
in:

1. **A token on every request.** Compared in constant time, never logged, never echoed in an
   error. The token stands for a context — the tenant, the account.
2. **A mandatory row filter** from that context, added to every query, every pivot level and
   every export before anything runs. No request can remove it.
3. **A field allowlist.** A column the caller may not see does not exist for them: the same
   `422 unknown field` a typo gets, and it is missing from `GET /source`.
4. **Bounds.** `max_limit` rows a page, a body size, a timeout, and for exports a row bound and
   a bound on how many run at once — each a status with a sentence, never a truncated answer.

## Proving a server

`opengrid-conformance` runs the whole suite over HTTP, in both result forms:

```sh
cargo run -p opengrid-conformance -- --endpoint http://127.0.0.1:8081 --token <token>
```

The server has to hold the fixture — `crates/opengrid-conformance/data/orders.csv` under
`orders.schema.json` — as the source `orders`, every column allowed, no row filter. Exit code 0
means every case agrees in JSON and in binary; otherwise each differing case is named with its
form and the first difference. It speaks plain HTTP; for TLS, run it next to the server.
