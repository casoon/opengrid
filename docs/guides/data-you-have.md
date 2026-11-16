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

The rows do not travel as one JSON text. The provider turns them into typed columns — numbers as
numbers, text as bytes, in packs of 65,536 rows — and the engine appends each pack to its columns,
so no text of the whole table is made, held or read a second time. Anything that is not plain goes
the old way, as JSON text, which names what is wrong with it: a bigint, a number that is not exact
or not finite, a text that is not well formed, a key the schema does not have, a list column. The
table is the same either way, and so is every refusal.

## A big file

`load` takes the whole file as bytes, and the engine's memory holds the file next to the columns it
makes of it. For a file of tens of megabytes, hand it over as a stream instead:

```js
await provider.loadStream("orders", file, schemaJson); // a File, a ReadableStream, an async iterable
```

The engine reads it a piece at a time (4 MiB by default, `{ chunkSize }` changes it), so only a
piece and the columns are in memory. The pieces may end anywhere, inside a field or a character;
the result, the errors and their line numbers are those of the whole file. The same replace rules
hold: all or nothing.

## Replacing

Loading under a name that is already loaded **replaces** the source, all at once. A grid showing
it runs its query again and keeps its view: the sort, the filter, and the focus, which follows the
record by its `row-key` (and stays at its place where the record is gone).

## What does not fit

A value that does not fit its field refuses the load, with the row and the field named
(`JSON row 1, field id: …`). Nothing is loaded, and the rows that were there stay.

Changing single rows is not part of this: a replace takes all rows.
