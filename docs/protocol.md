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
| `POST /pivot/{source}?format=xlsx` | a pivot query | the pivot as an XLSX file ([Export](#export)) |

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
  `starts_with`, `icontains`, `istarts_with` (ignoring case, rule S5a), `is_null`, `is_not_null` (no value). Combine with `and`, `or`, `not`.
  For a list column (`list<utf8>`, `list<int64>`): `has` (value is an element), `has_all` (value
  is an array of elements) and `is_empty` (no value) — and only those, plus the null checks. A
  list is a JSON array on the wire; its elements are never `null`.
- Aggregates: `count` (without `field`: every row), `sum`, `avg`, `min`, `max`.
- `offset` needs a `sort`. Values are JSON, read against the column's type: a decimal as a
  string (`"10.00"`), a date as `"2026-01-31"`, a timestamp as RFC 3339 in UTC, a list column
  (`list<utf8>`, `list<int64>`) as a JSON array whose elements are never `null`.
- What each of these means exactly — NULL, ordering, comparison, rounding — is
  [Query semantics](../guides/query-semantics/). A server that answers differently gives the
  browser different results from the engine in the tab.

A query may ask for **one level of a tree** (E38): `tree: { key?, parent, under?, scope?, aggregate?, flat? }` — `key`
(default `"id"`) and `parent` name the hierarchy, `under` the node whose children are asked
for; without it, the roots. A root is a node whose parent is NULL or names no node (an
orphan); a key twice or a cycle is an error. With a filter the level shows the matches **and
their ancestors**, the ancestors that do not match as context. A sort orders siblings, ties by
the key; paging is among siblings. The answer is the ordinary result of that level plus
`tree: { children, match, matches, orphans }` — per row its visible children and whether it
matches, and for the whole tree the matches and the orphans. A tree query cannot also group.
`aggregate` asks, per node of the level, for aggregates over its **subtree** (rule T7, issue
#165): `[{ field?, fn, as }]` like a query's `aggregate`, the same functions and result types
(S12), over the node and all its descendants, from the raw rows — and with a filter over the
subtree's **matches** only; context does not count. The answer carries them beside the rows,
not as columns of them: `tree.aggregates`, a list of columns in the result's own form (`name` is
the alias, one value per row of the level). An alias may not be a field's name.
`flat: true` asks for the **whole tree** instead of one level (rule T8, issue #166): every
visible node, depth-first, each node's children in the query's sort (ties by the key), paged
by `offset`/`limit` like any query — so an export reads it in pieces. It names no `under` and
carries no `aggregate` (both refused). Each row's place comes in `tree.flat: { level, path,
key_type, filtered }`: its depth (1 for a root), the keys from its root down to it (read as
`key_type`), and whether the query had a filter — only then does `match` say anything. A
source field named `level` or `path` — or `match`, with a filter — is refused: those are the
columns an export of the flat tree adds.
`scope` is a filter that decides which rows the tree **consists of**: a row outside it is
neither a match nor context nor a child, and a node whose parent is outside is an orphan. The
server puts its mandatory row filter (E16) there, not on the query's filter — on the filter it
would only stop matches, and the ancestors shown as context could be another tenant's rows.
Every source answers a tree through the server: one that cannot by itself (the capability
`tree`) is asked for the rows of the scope, and the engine answers the level. In the binary
form a tree's level is its own kind (`2`), the result followed by `matches`, `orphans`, the child
count per row and the matches as a bitmap; a reader of plain results refuses it rather than lose
that part. A level with parts beyond those appends a `u64` of flags (bit 0 subtree aggregates, bit 1 flat)
and then the parts: the aggregates as one more table (one row per row of the level); the flat
part as `filtered`, the levels, each path's length, and a one-column table of every path's
keys. A tree without parts is byte for byte what it was before them; a reader from before
them refuses those bytes rather than drop a part.

The pivot query is `{ source, rows, columns, values: [{ field, fn, as, calc? }], filter?, sort? }`.

`calc` shows a measure as a calculation over the pivot's cells: `share_of_row`,
`share_of_column`, `share_of_total` or `running_total`. A share's column is `float64`, a
fraction between 0 and 1, NULL where its total is NULL or zero; a running total keeps the
aggregate's result type. A share is defined for `fn` `sum` and `count`; a running total also for
`avg`, `min` and `max`, where the result is a number. Anything else is a `validation` error that
names the measure, and a name the contract does not have is `malformed`. The source computes it; the answer
has the same shape, the calculated values in the measure's columns. The rules are
[P10–P12](../guides/query-semantics/#calculated-measures-in-a-pivot). A measure without `calc`
is written without the key, so a server from before it reads a pivot as it always did.

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
| `backend` | 502 | The source behind the server failed. The message is one fixed sentence with a reference (`the data source failed (reference og-…)`); the cause stays with the operator, who finds it under that reference (`ServerBuilder::on_error`). |
| `data` | 422 | The data breaks a rule the query relies on — a tree with a key twice or a cycle (T1, T3). Not an outage and not the query's fault: the message is told in full, because it names rows of the caller's own data. Added after the others; a client that does not know it treats it as a general failure. |

## Export

`POST /export/{source}` takes the query body of `/query` and answers every row as a file:
`?format=csv` or `?format=json`, CSV options `delimiter`, `bom`, `protectFormulas`, `null`.
The headers carry `Content-Disposition: attachment` and `X-Total-Count`, the rows that follow.
Everything that can still be a status is decided before the first byte; a failure after it
breaks the connection off instead of ending a shorter file. The details are in
[Where queries run](../guides/where-queries-run/#exporting-from-a-server).

**A pivot as a workbook.** `POST /pivot/{source}?format=xlsx` takes the pivot query of `/pivot`
and answers the pivot as an XLSX file instead of a pivot result: one sheet with one header row —
the row dimensions by name, each generated column as its path and measure joined by ` · `
(`2025 · total`), a calculated measure followed by what it shows (`2025 · total · share of row`)
— and every row in the pivot's order, subtotals and the grand total included.
Nothing is merged and there are no outline levels. A value is a cell of its type; a total's label
stands in the first column and the dimension columns it spans are empty; a NULL measure is an
empty cell. The words come as parameters, each optional: `total` (`Total`), `subtotal` with
`{value}` for the closed group (`Total {value}`), `noValue` for a NULL dimension value
(`(no value)`), `emptyValue` for the empty string (`(empty)`), and what a calculated measure is
shown as: `shareOfRow` (`share of row`), `shareOfColumn`, `shareOfTotal`, `runningTotal`. Any other parameter beside
`format`, and any other `format`, is a `400`; without `format` the query string is ignored and the
answer is the pivot result. The headers are an export's, `X-Total-Count` being the pivot's rows.
The rules and bounds are the pivot's, and it counts as an export: `max_xlsx_rows` bounds it, and
too many exports at once are `busy`. A server may leave this out; a client that gets a pivot
result back treats it as "not offered".

## The duties of a server

A server that holds other people's data owes the browser four things, whatever it is written
in:

1. **A token on every request.** Compared in constant time, never logged, never echoed in an
   error, and what a database says never reaches the caller (it can name tables, hidden columns
   and tenant values). The token stands for a context — the tenant, the account. A token is a
   random secret of at least 32 characters (`openssl rand -hex 32`), never a word. The `Bearer`
   scheme is matched without regard to case (RFC 7235). The example server's demo presets fall
   back to public demo tokens when their environment variable is unset and warn about it on
   stderr instead of refusing to start (declined, #222): the demo pages carry those values, and
   the presets listen on the loopback address only; the `e2e` preset's `e2e-token` is a test
   fixture.
2. **A mandatory row filter** from that context, added to every query, every pivot level and
   every export before anything runs. No request can remove it. A context that does not fit the
   filter (a value that is not the column's type, a missing key) is a startup error, not a
   per-request one. For a tree the filter is the tree's `scope` only: putting it on the query's
   filter as well was declined (#221), because it would change the answer (T5 context, the flat
   `match` column).
3. **A field allowlist.** A column the caller may not see does not exist for them: the same
   `422 unknown field` a typo gets, and it is missing from `GET /source`.
4. **Bounds.** Every answer carries `Cache-Control: no-store` and `X-Content-Type-Options:
   nosniff`. A body is cut off at `max_payload_bytes` while it is read (also above axum's own
   2 MiB), an `offset` past `2^63 - 1` and an `in`, `has_all`, `and` or `or` of more than
   `max_list_values` (1 000) entries are `422`. `max_limit` rows a page — a `/query` that names no `limit` gets `max_limit`, and
   `total_count` still counts every match — a body size, a timeout, and for exports a row bound and
   a bound on how many run at once — each a status with a sentence, never a truncated answer.
   Work is bounded before it is done: a pivot's grouping sets are asked for with a limit one past
   `max_cells` (and the P9 order sets one past `max_rows`), so a pivot over a high-cardinality
   dimension is `limit_exceeded` without the source sending every group; a tree built by the
   engine (the source does not answer trees itself, and every flat tree) has its scope counted
   against `max_tree_rows` (default 5 000 000) before any row is read, for a level and for an
   export; an Excel export has `max_xlsx_rows` (default 1 000 000, never above the 1 048 576 of a
   sheet), because the file is assembled, and held in memory, before the first byte: it counts
   against the timeout, which a million rows of ten columns exceed at the default 10 s.

## Proving a server

`opengrid-conformance` runs the whole suite over HTTP, in both result forms:

```sh
cargo run -p opengrid-conformance -- --endpoint http://127.0.0.1:8081 --token <token>
```

The server has to hold the fixture — `crates/opengrid-conformance/data/orders.csv` under
`orders.schema.json` — as the source `orders`, every column allowed, no row filter. Exit code 0
means every case agrees in JSON and in binary; otherwise each differing case is named with its
form and the first difference. It speaks plain HTTP; for TLS, run it next to the server.
