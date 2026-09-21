---
title: Query semantics
description: The rules every engine has to answer identically, and the conformance cases that pin them.
order: 2
---

A query is a JSON AST, not SQL. What it means is fixed by a set of rules, and each rule by
conformance cases: one JSON file per case in `crates/opengrid-conformance/cases/`, each holding
a query against the shared `orders` dataset and the exact expected answer.

```json
{
  "id": "s8-decimal-is-exact",
  "rule": "S8",
  "query": {
    "source": "orders",
    "select": ["id", "amount"],
    "filter": { "field": "amount", "op": "eq", "value": "999999999.99" }
  },
  "expected": { "columns": ["id", "amount"], "rows": [[7, "999999999.99"]] },
  "ordered": false
}
```

## Who answers them

The same case files run against

- the Arrow engine, natively (`crates/opengrid-arrow-engine/tests/conformance_engine.rs`),
- the WASM build in headless Chrome (`crates/opengrid-wasm/tests/conformance_in_browser.rs`),
- the hybrid path, with a source that refuses what it does not declare
  (`crates/opengrid-arrow-engine/tests/conformance_hybrid.rs`),
- PostgreSQL (`crates/opengrid-datasource-postgres/tests/conformance_postgres.rs`) — this one
  skips itself without a database; point it at one with `OPENGRID_TEST_PG`.

## The rules, by their cases

| Rule | What the cases pin |
|---|---|
| S1 | Comparisons with NULL: `eq` and `ne` exclude NULL rows, `NOT` of an unknown stays unknown |
| S2 | `in` lists, including the empty list, which matches nothing |
| S3 | NULL ordering, ascending and descending |
| S4 | Binary collation |
| S5 | `contains` and `starts_with` are case-sensitive |
| S6 | Multi-column sort and paging with offset |
| S7 | NaN: not NULL, sorts after numbers, `-0.0` equals `0.0` |
| S8 | Decimals are exact |
| S9 | Dates have no time zone; timestamps keep microseconds |
| S10 | NULL forms its own group, and the empty string is a different one |
| S11 | Aggregates ignore NULL; `count(*)` versus `count(field)`; empty sets |
| S12 | Aggregate result types: `sum` of int64 stays int64, of decimal widens to 38 digits |
| S13 | Unicode normalisation: NFC is not NFD |
| S14 | The empty string is a value, not NULL |
| S15 | Derived date-part columns (year, month) filter, sort and group like stored ones |

The file names say the rest — `s7-nan-sorts-after-numbers.json`,
`s9-timestamp-microsecond-precision.json` — and each file is short enough to read.
