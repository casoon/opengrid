---
title: Data you already have
description: Rows the page holds as objects go into the engine as they are — typed, replaceable, with the grid keeping its view.
order: 10
---

An application often has its data as objects already — from its own backend, an IPC call, a
store. It does not have to write CSV for the engine:

```js
await provider.loadRows("people", rows, schemaJson);
```

`rows` is an array of objects keyed by field name (or that array as JSON text). Values are in the
notation of a result: decimals as strings, dates as `YYYY-MM-DD`, `null` for a missing value. The
schema is required — ingest never guesses a type — and the engine reads the rows as it reads the
same data from a CSV file, so a query answers the same either way.

## Replacing

Loading under a name that is already loaded **replaces** the source, all at once. A grid showing
it runs its query again and keeps its view: the sort, the filter, and the focus, which follows the
record by its `row-key` (and stays at its place where the record is gone).

## What does not fit

A value that does not fit its field refuses the load, with the row and the field named
(`JSON row 1, field id: …`). Nothing is loaded, and the rows that were there stay.

Changing single rows is not part of this: a replace takes all rows.
