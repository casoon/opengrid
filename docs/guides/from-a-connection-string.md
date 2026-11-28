---
title: From a connection string to a grid
description: Read the schema of your PostgreSQL, MySQL or SQLite database, edit the proposal, start the server from the file.
order: 11
---

You have a database and an application with its own forms. opengrid stays a grid: it reads,
sorts, filters, groups; saving stays your page's. `opengrid-server` (crate `opengrid-serve`)
takes you from a connection string to a running server in three steps, and releases nothing
until you have read what it would serve.

```sh
export OPENGRID_DATABASE_URL=postgres://app@localhost/shop   # or mysql://app@localhost/shop, sqlite:/path/to/shop.db
opengrid-server propose                                      # writes opengrid.json
# read it, edit it, delete "proposal": true
export OPENGRID_TOKEN=$(openssl rand -hex 32)
opengrid-server --config opengrid.json
```

## 1. The proposal

`propose` reads the tables and columns of the database (`information_schema` for PostgreSQL and
MySQL, `pragma table_info` for SQLite; `--namespace` picks the PostgreSQL schema, `public` by
default; MySQL's tables are those of the database the URL names)
and writes `opengrid.json`. It never overwrites a file. Each table becomes a source, each column
with a type opengrid knows a field:

```json
{
  "proposal": true,
  "listen": "127.0.0.1:8080",
  "allow_origins": [],
  "tokens": [{"env":"OPENGRID_TOKEN","context":{}}],
  "sources": [{
    "name": "orders",
    "database": "postgres",
    "connection": "OPENGRID_DATABASE_URL",
    "namespace": "public",
    "table": "orders",
    "schema": { "fields": [
      {"name":"id","type":"int64","nullable":false},
      {"name":"amount","type":{"decimal":{"precision":12,"scale":2}},"nullable":true}
    ] },
    "left_out": [{"column":"payload","database_type":"jsonb","reason":"the type has no opengrid type"}]
  }]
}
```

### Types

| PostgreSQL | opengrid |
|---|---|
| `smallint`, `integer`, `bigint` | `int64` — `bigint` stays 64-bit |
| `real`, `double precision` | `float64` |
| `numeric(p,s)` | `decimal(p,s)` — exact; plain `numeric` has no precision and is left out |
| `text`, `varchar`, `char` | `utf8` |
| `boolean` | `bool` |
| `date` | `date` |
| `timestamptz` | `timestamp`, microseconds, UTC; `timestamp` without a zone is left out |
| `text[]`, `int[]`, `bigint[]` | `list<utf8>`, `list<int64>` |

| MySQL | opengrid |
|---|---|
| `TINYINT(1)`, `BOOLEAN` | `bool` |
| `TINYINT` … `BIGINT` | `int64` — `BIGINT UNSIGNED` does not fit and is left out |
| `FLOAT`, `DOUBLE` | `float64` |
| `DECIMAL(p,s)` | `decimal(p,s)` up to 38 digits |
| `CHAR`, `VARCHAR`, `TEXT`, … in `utf8mb4` | `utf8` — another character set is left out |
| `DATE` | `date` |
| `TIMESTAMP` | `timestamp`, read in UTC; `DATETIME` is a wall clock and is left out |

A MySQL `JSON` column is left out: name a list column by hand (`list<utf8>`, `list<int64>`).

| SQLite (declared type) | opengrid |
|---|---|
| `INTEGER`, `INT`, `BIGINT`, … | `int64` |
| `REAL`, `FLOAT`, `DOUBLE` | `float64` |
| `TEXT`, `VARCHAR`, `CHAR`, `CLOB` | `utf8` |
| `BOOLEAN` | `bool` |

A type without a mapping is **named under `left_out` and left out, not guessed** — as is a
column whose name is not a field name (`^[A-Za-z_][A-Za-z0-9_]{0,62}$`). SQLite has no decimal
or date type of its own: the SQLite connector stores them as scaled integers and microseconds
(see [Connectors](../connectors/)), so a file laid out that way reads as `int64`, and you edit
the type to `decimal`, `date` or `timestamp`. `DECIMAL`, `DATE` and `DATETIME` columns of a file
written by something else are left out: their values are not what the connector reads.

## 2. The edit

The file is the server builder's options as data, and a source's `schema` is the schema JSON the
library reads everywhere, so you can fix a type or add a derived column
(`"from": {"part": "year", "field": "ordered_on"}`) in the file.

- **Delete what must not be served**: a table, a column. A column the file does not name does
  not exist for a client — it cannot be selected, sorted or filtered by anyone.
- **`allowed_fields`** narrows what a client may name further (default: every field the file
  names); **`row_filter`** `{"field","op","value"}` is added to every query, with `":key"` taking
  the caller's token context, as in the builder.
- **`tokens`**: each `{"env": NAME, "context": {...}}` is one bearer token and the context its
  row filter reads. At least 32 characters; set in the environment, never in the file.
- Options: `listen`, `allow_origins`, and `limits` (`max_payload_bytes`, `timeout_ms`,
  `max_export_rows`, `max_tree_rows`, `max_xlsx_rows`, `count_cache_ttl_ms`,
  `max_concurrent_exports`). Unknown keys are refused: a typo must not silently drop a restriction.
- Delete `"proposal": true`. A file that still has it does not start.

## 3. Run

`opengrid-server --config opengrid.json` reads the connection string and every token from the
environment variables the file names, checks everything the builder checks (a name twice, an
allowed field or row filter the schema does not have, a token context that does not fit a row
filter), and serves. A page talks to it like to any opengrid server — see
[Connectors](../connectors/) for the contract and [Saving edits](../saving-edits/) for writing
back through your own API.

PostgreSQL over TLS is not in this command (`sslmode=require` is refused rather than downgraded);
a server in code can use `connect_with_tls`. Writing to the database from the server is not part
of it either.
