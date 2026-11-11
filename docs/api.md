---
title: The public API
sidebarLabel: Public API
description: Elements, attributes, events, parts, custom properties and translatable texts — everything a page can rely on.
order: 1
---

Everything a page can rely on. A test freezes this list
(`crates/opengrid-web-components/src/api.rs`): when a name here changes, that
test fails and the change has to be argued for in the diff.

Anything **not** on this page is internal, whatever its Rust visibility looks
like.

## Contents

- [Loading](#loading) · [Providers](#providers) · [Connecting](#connecting)
- [`<opengrid-grid>`](#opengrid-grid) · [`<opengrid-table>`](#opengrid-table) · [`<opengrid-pivot>`](#opengrid-pivot)
- [The view](#the-view) · [Errors](#errors) · [Events](#events) · [Styling](#styling) · [Right to left](#right-to-left) · [Texts](#texts)

## Loading

```js
import { loadOpengrid } from "@casoon/opengrid/loader.js";

const loader = await loadOpengrid();          // loads the WASM, registers the elements
loader.module.set_provider(host, provider);   // where the data comes from
```

`loadOpengrid()` answers `{ fallback, module }`. `fallback` is `true` when the
WebAssembly module could not be loaded and a plain-DOM stand-in was installed
instead.

| Function | What it does |
|---|---|
| `set_provider(host, provider)` | Attaches the data source. `provider.execute(queryJson, mode, { signal })` answers the result as JSON or in the [binary form](#the-binary-result-form) (`Uint8Array`, `ArrayBuffer`), or a Promise of either; the third argument is optional, and so is everything else. |
| `set_texts(host, texts)` | Overrides any subset of the [texts](#texts); a second call adds to the first, a key left out keeps what it was, and `null` takes them all back to the English defaults. Call it **before** `set_provider` and the component renders the right words from its first paint. |
| `set_formats(host, formats)` | Per-column display formatting — see [`<opengrid-grid>`](#opengrid-grid). |
| `set_choices(host, choices)` | Per-column editor choices: `{ customer: ["Alpha", "Beta"] }` turns that column's editor into a type-ahead combobox; an entry may be `{ value, label }`. A list column is edited only with choices — see [Lists](#lists). |
| `get_view(host)` / `set_view(host, view, options?)` | Reads and applies the whole [view](#the-view) in one step; `options.notice` is said with the result. |
| `set_columns(host, columns)` | Per-column presentation — see [`<opengrid-grid>`](#opengrid-grid). |
| `get_query(host)` | The query of the current view, without a window — what an [export](#exporting-the-view) sends. In a [tree](#tree), every node flat (`tree.flat`), with its level and path. |
| `get_pivot(host, options)` | The pivot as it is shown, as CSV — see [exporting a pivot](#exporting-a-pivot). |
| `prepare_print(host)` | Builds the grid's print table — every match of the view — and answers a Promise of its row count; see [printing the view](#printing-the-view). |
| `set_cell_state(host, key, column, state, message?)` | What became of an edit the page saves: `saving`, `saved` or `error` — see [Saving edits](#saving-edits). |
| `set_values(host, values)` | Values the page computes, `[{ key, column, value }]` — see [Saving edits](#saving-edits). |
| `set_validators(host, validators)` | A function per column that runs while the reader types and says `null` or `{ level, message }` — see [Validation](#validation). |
| `set_row_state(host, provisionalKey, state, detail?)` | What became of a row the reader added: `saving`, `saved` (with the record's key) or `error` (with a message) — see [Adding rows](#adding-rows). |
| `set_actions(host, [{ id, label }])` | A trailing column per action, a button per row; fires `opengrid-row-action` — see [Row actions](#row-actions). |
| `register()` | Defines the three elements. `loadOpengrid()` calls it; a page that loads the module itself calls it once. |

**Types.** The package ships `loader.d.ts`: every name on this page is typed —
the view, the column configuration, the texts, the formats, the providers, and
the attributes of each element as `Opengrid…Attributes` for adapters and JSX
typings to build on — and the events are in the global event map,
so `grid.addEventListener("opengrid-view-change", e => e.detail.view)` knows what
`detail` holds, on the element and on the document alike.

**A provider is a seam, not a class.** Anything with an `execute` method fits,
which is how the engine can sit in the tab, in a worker, behind HTTP, or be
split across two of them without the elements knowing.

### Data without JavaScript

The rows exist only once an element has run. A reader or crawler that runs no
script, and a page whose module failed to load, would see none of them. So a page
can write its data as a plain table **inside** the element:

```html
<opengrid-table label="Orders" datasource="orders" columns="id,customer,amount">
  <table>
    <caption>Orders</caption>
    <thead><tr><th scope="col">id</th><th scope="col">customer</th><th scope="col">amount</th></tr></thead>
    <tbody><tr><td>1</td><td>Alpha</td><td>120.00</td></tr></tbody>
  </table>
</opengrid-table>
```

| When | What is read |
|---|---|
| No JavaScript | The page's table, as HTML: headers, rows, caption. |
| The element has rendered | Its own shadow root. The page's table is not rendered and not in the accessibility tree, so nothing is read twice. |
| The module could not load | The page's table again: the plain-DOM stand-in shows it in its place, and its empty skeleton only when there is none. |

The element does not read that table: its data comes from the provider. What the
page puts there is the page's choice, such as the first page of rows or a summary,
and it should say what it is, with a caption and `scope` on its headers.

## Providers

All from `loader.js`, all the same shape:

| | |
|---|---|
| `createLocalProvider(engine)` | The engine on the main thread. `load(name, bytes, schema)`, `loadRows(name, rows, schema)` ([Data you already have](../guides/data-you-have/)), `execute`, `pivot` (see below), `stats()`, `terminate()` (a no-op here). `watch(host)` is what `set_provider` calls; a page does not. |
| `createWorkerProvider({ moduleUrl?, wasmUrl?, workerUrl? })` | The engine in a module worker; started lazily, once. Every option is optional: without `moduleUrl` it is the engine the package ships under `engine/`, without `workerUrl` the package's `worker.js`. The same methods as the local provider; `terminate()` stops the worker. `moduleUrl` and `wasmUrl` are strings, since they travel to the worker by `postMessage`. A bundler follows the defaults; `moduleUrl` and `workerUrl` are for serving the files from elsewhere — see [Frameworks → Bundlers](../guides/frameworks/#what-every-adapter-does-the-same-way). |
| `createRestProvider({ url, source, token })` | `POST /query/{source}` of an `opengrid-server`. Also offers `describe()` → `{ name, schema, capabilities, pivot_limits }`, and `export(query, options)` → a `Blob` from `POST /export/{source}` ([below](#over-a-server-one-request)), and `exportPivot(pivotQuery, options)` as the pivot provider does. |
| `createHybridProvider({ remote, planner, mode, onPlan })` | Splits each query between a remote source and the engine in the tab. `onPlan` receives the plan before anything is sent. |
| `createPivotProvider({ url, source, token })` | `POST /pivot/{source}` — a whole pivot in one request. Also offers `exportPivot(pivotQuery, options)` → the pivot as an XLSX `Blob` from `POST /pivot/{source}?format=xlsx` ([exporting a pivot](#exportpivotprovider-host-options)). |

**What the engine holds** — `stats()` of the local and the worker provider (issue #70) answers
`{ kind, memory, sources: [{ name, rows, columns, bytes }] }`: the engine module's WASM
memory, which only grows, and what each loaded source's columns take inside it. Together with
the grid's `opengrid-query` event it is a page's resource report.

**A pivot without a server.** `<opengrid-pivot>` calls `provider.pivot(pivotJson, mode, { signal })`
when the provider has it, `execute` otherwise. `createLocalProvider` and `createWorkerProvider`
have it — the engine answers pivots over its sources (`Engine.pivot`, `Engine.pivot_columns`),
under the server's default limits — and so does `createPivotProvider`.

Each of them carries a **`kind`** — `"local"`, `"worker"`, `"remote"` (REST and pivot) or
`"hybrid"` — and a provider of a page's own may too. The grid names it in its footer,
beside the number of queries asked for what is shown (`worker · wasm · 2 queries`); a
provider without a `kind` is shown with the count alone. The words are
[texts](#texts).

**The engine** — `Engine` for the tab, `Planner` for the hybrid provider —
ships in the package under `engine/`, a module of its own next to the
elements'. A worker needs no URL; a page that queries on the main thread
imports it:

```js
import init, { Engine, Planner } from "@casoon/opengrid/engine/opengrid_wasm.js";
import { createLocalProvider, createWorkerProvider } from "@casoon/opengrid";

await init();
const inTheTab = createLocalProvider(new Engine());
const inAWorker = createWorkerProvider();   // the same engine, off the main thread
```

Its types are generated with it and declare `[Symbol.dispose]()`: a
TypeScript project that checks library types needs `ESNext` or
`ESNext.Disposable` in `lib` (TypeScript 5.2+), or `skipLibCheck` — only a
project that imports the engine.

### The binary result form

A result can travel as JSON or as bytes in `application/vnd.opengrid.columns` — the
engine's own columns, written out: no text per cell, decimals exact, and a worker hands
the bytes over without copying them. The elements and `exportRows` read both forms, so a
provider of your own may answer either.

The built-in providers use the bytes: `createLocalProvider` and `createWorkerProvider`
answer them (`Engine.execute_columns`), `createRestProvider` and `createPivotProvider` ask
the server for them with `Accept` and take JSON when the server answers JSON, and
`createHybridProvider` finishes in the form its remote answered in
(`Planner.finish_columns`). `Engine.execute` and `Planner.finish` still answer JSON. A page
that calls a built-in provider's `execute` itself gets bytes; the layout is described in
`crates/opengrid-columns/src/wire.rs`.

**Cancelling.** `execute` takes an optional third argument, `{ signal }`. The
REST, pivot and hybrid providers hand the `AbortSignal` to `fetch`, so an
aborted request stops; the tab and the worker cannot stop a query that has
started and ignore it — the caller drops their answer. A provider written for
two arguments still fits. [`exportRows`](#exporting-the-view) is what passes one.

## Connecting

The module functions have rules — wait for the module, texts before the
provider, the view before the provider, a controlled view written back without
a loop. `connect` knows them, so a page or a framework adapter supplies an
element from one object and keeps it supplied:

```js
import { connect } from "@casoon/opengrid";

const grid = document.querySelector("opengrid-grid");
const connection = connect(grid, {
  provider,
  texts: { lang: "de", loading: "Wird geladen …" },
  presentation: { amount: { align: "end", aggregate: "sum" } },
  view: saved,
  onViewChange: (view) => save(view),
});
connection.update({ view: other });   // applies only what changed
connection.disconnect();              // the listeners go; the element keeps its state
```

| Option | Module function |
|---|---|
| `provider`, `texts`, `formats`, `choices`, `view` | `set_provider`, `set_texts`, `set_formats`, `set_choices`, `set_view` |
| `defaultView` | `set_view`, once — the uncontrolled form: after that the grid leads |
| `presentation` | `set_columns` — named apart from the `columns` attribute, which is the projection |
| `onViewChange`, `onSelectionChange`, `onCellChange` | the [events](#events) of the same names; each callback receives the `detail` (for the view: the view itself) |
| `actions`, `validators` | `set_actions`, `set_validators` — the grid only |
| `onPaste`, `onBulkChange`, `onRowAction`, `onSourceReplaced` | `opengrid-paste`, `opengrid-bulk-change`, `opengrid-row-action`, `opengrid-source-replaced`; each callback receives the `detail` |
| `onDrillDown` | `opengrid-drill-down` — the pivot only, with its `drill-down` attribute; the callback receives the `detail` |
| `onRowAdd` | `opengrid-row-add`, and its answer is `set_row_state`: return `{ state: "saved", key }` or `{ state: "error", message }`, or a promise of one — the row shows `saving` until it settles; resolved with nothing, it is saved, and a rejection is an `error` with its message. No answer at all leaves the row unsaved for the page; an undone add is not answered |

The connection also carries `set_cell_state` and `set_values` as `setCellState(key, column, state, message?)` and `setValues(values)` — the grid only; before the module has loaded, they wait for it.

```js
const connection = connect(grid, {
  provider,
  actions: [{ id: "remove", label: "Remove" }],
  onRowAdd: async ({ values }) => ({ state: "saved", key: (await save(values)).id }),
  onCellChange: async ({ key, column, value }) => {
    await update(key, column, value);
    connection.setCellState(key, column, "saved");
  },
});
```

| | |
|---|---|
| Order | Texts, formats, presentation, choices, actions, validators, view, provider — so the first query is the only one, and the first paint is in the right words. An `update` that changes the texts or the presentation rebuilds the grid, and one that changes the view as well rebuilds it twice: the source is asked twice, and only the second answer is shown. |
| Changes | `update` writes an option only when it differs from what the element has. A key left out keeps its value; a key given as `undefined` resets it — texts to English, no formats, no presentation, no choices, no actions, no validators. `provider` has no "none"; for `view`, `undefined` means the grid leads. |
| Controlled | `view` is written whenever it differs from what the grid shows: after the reader sorted, passing the same saved view again restores it — and a page that keeps passing a view without taking the reader's changes back holds the grid there, as a controlled input does. Writing back what the grid just reported costs nothing: `onViewChange: (view) => connection.update({ view })`. `onViewChange` hears the reader, not the views `connect` wrote. A view the grid refuses — a column it does not have yet — is tried again on the next `update`. Formats and validators compare functions by identity — keep them stable, or each update redraws. |
| Timing | `connect` returns at once; `connection.ready` settles once the module is loaded and the options are applied. Updates before that are folded in. On the [fallback](#loading), nothing is applied and `ready` says so. |
| The view | needs the element in the document. On one that is not, the view **and the provider** wait for the next `update`, together, so the first query still asks for the view; the console says so. Everything else may come first. |
| Loading | `connect` calls `loadOpengrid()`, whose first call decides the URLs: a page that needs its own calls `loadOpengrid(options)` first. |
| Attributes | are not options. `label`, `datasource`, `columns`, `group-by` and the rest are set on the element, by the page or the framework, as always. |

Importing the package touches no DOM, so it is safe in server-side rendering;
`connect` belongs where the element exists.

## `<opengrid-grid>`

An interactive `<table role="grid">`: virtualized, keyboard-driven, filterable.

```html
<opengrid-grid label="Orders" datasource="orders" columns="id,customer,amount"
               window-size="40" mode="auto"></opengrid-grid>
```

| Attribute | Meaning |
|---|---|
| `label` | The table's accessible name (`aria-label` and `<caption>`). |
| `datasource` | The source name; becomes the query's `source`. |
| `columns` | Comma-separated output fields, in order. |
| `window-size` | How many rows are rendered and fetched at once while scrolling. Default 40. |
| `page-size` | Switches from scrolling to **paging**. Mutually exclusive with virtualization. |
| `mode` | `local`, `remote`, `hybrid` or `auto`, handed to the provider unchanged. Only a provider with more than one place to run a query reads it. |
| `group-by` | Groups the rows by up to **three** columns, outermost first: `group-by="country,customer"`. See [Grouping](#grouping). |
| `tree` | Shows the rows as a **tree**: the field that holds a row's parent key, `tree="parent_id"`. See [Tree](#tree). |
| `tree-key` | The field the parent key refers to. `id` when absent. |
| `row-key` | The field that names a record: `opengrid-cell-change` carries its value as `key`, and `set_cell_state` and `set_values` name rows by it. See [Saving edits](#saving-edits). |
| `filter` | The **page's filter**, as the wire's JSON: `{"field":"country","op":"eq","value":"DE"}`, `{"and":[…]}`. And-ed onto every query the grid sends; not the reader's and not in the view. See [The page's filter](#the-pages-filter). |
| `search` | Puts a **search field** above the grid: free text, or a filter written out. See [Search](#search). |
| `facets` | Shows the **facet sidebar** — the facets a page configured with `set_columns`. See [Facets](#facets). |
| `toolbar` | Puts a **toolbar** above the grid: the active filters and the grouping as chips, a switch for the filter row, the column list, the density. Opt-in. See [Toolbar](#toolbar). |
| `add-row` | An empty **new row** after the last one: type a record into it. See [Adding rows](#adding-rows). Off while grouped, in a tree and with `page-size`. |
| `filter-builder` | Puts **Advanced filter** into the toolbar: the button that opens the builder of the [nested filter](#nested-filter). Opt-in, and only with `toolbar`. Without it a nested filter still works — the search expression writes it, the view carries it — and its chip still opens the builder. |
| `column-menu` | Gives every header a **column menu**: sort, filter, aggregate, group, hide. Opt-in. See [Column menu](#column-menu). |
| `selection` | Shows the **selection column**: a checkbox per row and one in the header. Opt-in — selecting rows works from the keyboard either way; what this adds is the column that shows it and the pointer path to it. |
| `density` | `compact`, `normal` or `comfortable` — row height, cell padding and font size in one step. `normal` is the default, and a grid without the attribute *is* a normal one; an unknown value is normal too. |
| `theme` | `base`, `paper`, `violet`, `orange` or `dark` — one of the built-in looks ([Styling](#styling)). `base` is the default; absent or unknown is `base`. Not part of the [view](#the-view): the look is the page's choice, not the reader's. |

**Keyboard.** The WAI-ARIA grid pattern — arrows, `Home`/`End`, `Ctrl`+`Home`/`End`,
`PageUp`/`PageDown`, and:

| Keys | On | Does |
|---|---|---|
| `Enter` / `Space` | header cell | sort (`Shift` adds a second key) |
| `Enter` / `F2` | data cell | open the editor (`Enter` commits, `Escape` discards) |
| a character | data cell | open the editor with it: it replaces the value in a text or number field, and in a select picks the one choice that is it or starts with it (the cell's value when none or several do). `Enter` still commits. A date or a checkbox opens on the cell's value |
| `Space` | data cell | select the row (`Shift` extends from the last one) |
| `Space` | selection cell | select the row (`Shift` extends) |
| `Enter` / `Space` | selection header | select **every matching row**, or clear it |
| `Ctrl`/`Cmd`+`A` | anywhere | select every matching row, not only the loaded page |
| `Ctrl`/`Cmd`+`←`/`→` | header cell | move the column |
| `Ctrl`/`Cmd`+`←`/`→` | band cell | move the band with all its columns ([Column bands](#column-bands)) |
| `Ctrl`/`Cmd`+`Shift`+`←`/`→` | header cell | resize the column |
| `Alt`+`Shift`+`←`/`→` | header cell | pin the column one step towards that side: let go of a pin at the other end, or pin it at this one ([Pinned columns](#pinned-columns)) |
| `Alt`+`Shift`+`←`/`→` | band cell, or the header cell of a column in a band | pin the **band** one step towards that side |
| `↑` / `↓` | header cell of a column in a band / band cell | go to the band / back to the column ([Column bands](#column-bands)) |
| `Alt`+`↓`, `Shift`+`F10`, context-menu key | header cell | open the column menu (with `column-menu`) |

On a right-to-left page `←` and `→` change places, and nothing else does:
[Right to left](#right-to-left).

With `selection` set, the column is the **start of the row**: `Home` and
`Ctrl`+`Home` go there, and `←` from the first data column reaches it. Its
header is a real `checkbox` inside the header cell — a `columnheader` may not
carry `aria-checked` — and it is tri-state: empty, `mixed`, checked. The
per-row mark is decoration; the row itself says `aria-selected`, and a second
voice per row would double every announcement.
The table carries `aria-multiselectable="true"` (`Shift`+`Space` and `Ctrl`+`A` select
several). While grouped the data rows say it and the group headers and the total never do — and
without a `row-key`, where a grouped grid selects nothing, neither the table nor a row says
anything of a selection.

**What a selection names.** With a `row-key` the selection names **records, by their key**; in a
[tree](#tree) it names nodes by `tree-key`; without either it names **positions** in the result.
The keys are the same in all three; what differs — what survives a sort, what the event and the
view carry — is in [Selecting by key](#selecting-by-key).

**Very long results** (issue #284). A browser caps the height of an element, so the
scroll area is never more than 10 000 000 px (312 500 rows at 32 px, 238 095 at 42 px;
measured: Chromium and WebKit stop at 33.5 million, Firefox at about 17.9 million). A
result that fits is drawn exactly as it always was. A taller one is scrolled through two
maps, and `aria-rowindex` and `aria-rowcount` stay the logical rows throughout:

- **Dragging the scroll bar**, `scrollTop = x` and `Ctrl`+`Home`/`End` map proportionally:
  the thumb at the middle is the middle of the result, its two ends are the first row and
  the last row (the last one at the bottom edge). One pixel of the thumb is many rows.
- **Everything smaller** — the wheel, touch, the track, `PageUp`/`PageDown`, the arrow keys —
  moves the browser's own scroll position, so one notch of the wheel is a few rows, not
  thousands. Near either end of the scroll bar the grid puts the thumb back to the
  proportional place of where the rows actually are (the rows do not move on screen), which
  is what lets the wheel reach the first and the last row from anywhere. In between the thumb
  is therefore not exactly proportional while the wheel is used.

The query is the same window of `limit` and `offset` as for a short result. The focused row
stays pinned while the reader drags away from it; it is parked outside the scroll area and
is back in place when the reader returns.

Column visibility is a panel under its button (*Columns*, in the filter row, or in
the toolbar when there is one): a checkbox per column, including the hidden ones —
otherwise there would be no way back. It stays open while columns are ticked, each
tick takes effect at once and the focus stays on the checkbox; `Escape` closes it and
returns the focus to the button, and so does a click outside (without the focus).

**The filter row** is one field per column, as wide as the column (issue #96).
Typing filters with a comparison that fits the type: *contains* for text,
*is* for numbers and dates. A small button inside the field (`filter-operator`)
shows the comparison as a sign and names it in words (`customer operator:
contains`); it opens a menu (`operator-menu`, `menuitemradio` items) of the
comparisons the type allows, with the column menu's keys — `↓`/`↑` also open it
from the button, `Escape` closes it and returns the focus. A comparison picked
while the field has a value, or one that takes none (*has no value*), filters at
once. `Enter` in the field applies as before.

**The filter row's fields** follow the column's type: text for text, a number
field for numbers, a date field for dates (a timestamp is typed as text), a
choice of *any / yes / no* for booleans, which has no comparison button. The date field is the browser's own `<input type="date">`, so it
shows dates the way the **reader's** system writes them (`dd.mm.yyyy` on a
German one), whatever the page's language, and it brings the browser's picker
and its keyboard and screen-reader support. The value it gives the query is
always `YYYY-MM-DD`. This is deliberate: the reader's own format is the one
they read and type fastest.

**Formatting** is display only and never reaches a query:

```js
loader.module.set_formats(host, {
  amount: { kind: "number", locale: "de-DE", style: "currency", currency: "EUR" },
  ordered_on: { kind: "date", locale: "de-DE" },
  qty: (text, value) => `${value} pcs`,
});
```

Which locale an `Intl` format reads is decided in this order: the format's own
`locale`, then the nearest `lang` attribute above the element (through shadow
roots), then the browser's. A page with `lang="de"` therefore formats `1.234,5`
in an English browser too, in agreement with the voice a screen reader uses. When
a `lang` changes, the formats are resolved again and the element redraws; a
format with an explicit `locale` does not move. A function format is the page's
own and is never touched.

A column without a format is rendered in Rust and never crosses the WASM/JS
boundary. `Intl` takes a JavaScript number, so a decimal beyond 2^53 loses
digits through the options form — such a column takes a function, which receives
the exact text.

**Presentation** is what the schema cannot say:

```js
loader.module.set_columns(host, {
  id:       { width: 96, mono: true, muted: true },
  customer: { title: "Customer", emphasis: true },
  amount:   { width: 150, align: "end", aggregate: "sum", facet: "range" },
});
```

| Key | Meaning |
|---|---|
| `title` | What the column is **called** where a reader reads it: the header, the column list, the filter row's labels, the column menu, the "+ Filter" dialog and "+ Group" menu, group rows, chips, facet headings, search suggestions and every announcement. The page's words, in the page's language. The field name stays the identifier — the query, the view, the search expressions (`country = DE`), exports and events. An empty title is reported. |
| `width` | The column's **starting** width in pixels. A reader's resize leads after that. |
| `pin` | `start` or `end`: where the column **starts** pinned while the others scroll sideways. A reader's own pin leads after that. See [Pinned columns](#pinned-columns). The grid only; a table refuses it. |
| `band` | The **band** the column stands under: a header above it that it shares with the other columns naming the same band, `band: "Address"`. The page's words, like `title`. See [Column bands](#column-bands). The grid only; a table refuses it. |
| `align` | `start`, `end` or `center`. Numbers default to `end`, everything else to `start`. |
| `mono` | Draw the values monospaced, so they line up character by character. |
| `emphasis` / `muted` | Bold, or the muted ink. |
| `aggregate` | `sum`, `avg`, `count`, `min`, `max` or `range` — the column's aggregate in groups. |
| `facet` | `list`, `pills`, `range` or `period` — how the column is offered as a facet. |
| `lookup` | A map from the stored key to the label a reader reads, `{ 1: "Alpha GmbH", 2: "Beta AG" }`: the cell, the editor, the filter row and the facets show and choose by label; the query, the sort, the groups and every export keep the key. See [Lookup columns](#lookup-columns). The grid only; a table refuses it. |
| `readonly` | The page fills this column with `set_values`: it never opens an editor, and its cells say `aria-readonly`. The grid only; a table refuses it. |

**The configuration narrows; it never widens.** `sum` over a text column, a
`range` facet over text, a name the grid does not have — each is reported in the
status line and **none** of the call is applied. A grid that looks configured
and is not hides the typo that caused it. Every problem in one call is reported
together.

What the type already answers is not configurable: which filter operators a
column offers is meaning, not taste. The alignment is taste, so it may be
overridden. A column that is declared in `columns` but currently hidden keeps
its configuration — its type-dependent checks run again when it is shown.

### Toolbar

With `toolbar`, a labelled group of ordinary buttons sits above the filter row —
outside `role="grid"`, like the filter row, so the grid's keys never reach it. It is
one row: the search field first (with `search`), then the switches, the column list
and the density as a segmented control.

On a narrow grid — under 560 px of its own width, whatever the window — the search
field takes a line of its own and the buttons stay in one row that scrolls sideways,
instead of wrapping into several; a button that takes the focus is brought into view.
An opened column list gets a line of its own below the row. At a 320 px viewport, the
reflow case of WCAG 1.4.10, the toolbar wraps instead.

| | |
|---|---|
| Filter row | A switch (`aria-pressed`). Hiding the row gives its height to the viewport, and `PageUp`/`PageDown` step by what is really there. Whether the row shows is part of the [view](#the-view) as `filterRow`, **on** by default — there is no attribute for it, because a boolean attribute is off by default and the row has always been there. |
| Columns | The column list moves here from the filter row, so it stays reachable when the row is hidden. |
| Density | Three buttons, the pressed one is the grid's `density`. |
| + Filter | Opens *Add filter*, a `role="dialog"` with `aria-modal="true"` (`Tab` does not leave it): *Column*, *Condition* (the operators the column's type allows), *Value*, *Cancel*, *Apply*. The focus moves to *Column*; `Tab` cycles inside; *Apply* or `Enter` in *Value* writes the column's entry in the filter row — the one place a filter lives — and closes; `Escape`, *Cancel* or a click outside close without a change. A value the column cannot take is named in the dialog, which stays open, and said in the status line — the one live region. The focus returns to the button. |
| Advanced filter | With `filter-builder`. Opens the builder of the [nested filter](#nested-filter): conditions joined by *all* or *any*, in groups. A native modal `<dialog>`. |
| + Group | Opens a `role="menu"` of the groupable columns not grouped yet, with the column menu's keys (`↓`/`↑`, `Home`/`End`, `Enter`/`Space`, `Escape`, `Tab`). A pick adds the next grouping level, as `group-by` does. With three levels the button is `aria-disabled` and says why. |
| Chips | One per active filter, in words (`country is DE`), one for the [nested filter](#nested-filter) as a whole, and one for the grouping. Each has a remove button **named for its filter** — `Remove country is DE` — and "Remove all" clears filters and grouping. A removal is said once, with the result that follows; the focus moves to the next chip, never to the document. |

The chips are a display of the view, not a second truth about the filters: they
are drawn from it, and redrawn only when what they say changed.

### The page's filter

```html
<opengrid-grid datasource="orders" columns="id,customer,qty"
               filter='{"field":"country","op":"eq","value":"DE"}'></opengrid-grid>
```

`filter` restricts the grid to a part of its source — the orders of one country, the
rows behind a pivot's cell ([Drilling down](#drilling-down)). It is a filter object as
the [protocol](../protocol/) writes it, the same notation as the pivot's `filter`: a
comparison, `is_null`, `and`, `or`, `not`, nested as needed; the empty string is a
value (`"op":"eq","value":""`) and not NULL.

| | |
|---|---|
| Where it holds | In **every** query the grid sends: the rows, the counts, group rows and their aggregates, a tree's levels, the facets' values and counts, the keys behind "select all", and `get_query` — so an export is the same rows. It is and-ed with what the reader filters; the reader works inside it. |
| Whose it is | The page's. It is not shown in the filter row or as a chip, "clear filters" leaves it, the empty state offers no reset for it, it is not in `get_view`/`set_view`, and changing it fires no `opengrid-view-change`. |
| Changing it | Asks again, as a filter of the reader's does: the first rows, the selection dropped (and said), the facets' values listed anew. Removing the attribute removes the filter. |
| What does not hold | Text that is not a filter object is said in the status line and nothing is asked — never a grid that silently shows every row. A filter the source refuses (an unknown field, a value of the wrong type) is the provider's error, in the status line like any refused query. |
| Not access control | The filter is written by the page and readable and changeable in the browser. What a reader **may** see is decided by the server's row filter; this only chooses what the page shows. |

A framework component takes it as the `filter` prop, a string, as the pivot's.

### Empty state

When a result has no rows, the viewport says why, in one of two sentences:
`No row matches these filters.` with a **Reset filters** button — the same as
the toolbar's "Remove all" — or, when the source itself is empty, `There are no
rows.` and no button, because a reset would promise what it cannot do. The
panel is silent: the status line already says `No matches`, and a second voice
would say it twice.

### Search

With `search`, a field above the grid takes two kinds of input, told apart by
how they start:

| Typed | Enter does |
|---|---|
| `country = DE and amount ≥ 10` | Writes **the filter row's own entries** — the same fields, the same view, the same chips — and empties the field. One place a filter lives. |
| `country = DE or amount ≥ 1000`, `amount > 1 and amount < 5` | Sets the grid's [nested filter](#nested-filter) — the one place a filter lives that the row cannot hold — and empties the field. |
| `Alpha` | A free-text search: `contains` on every shown text column, or-ed. |

Input that starts with a word and an operator is read as a filter even when the
word is not a column: `colour = red` says "colour is not a column of this grid"
rather than searching for the words and finding nothing.

The expression, written down:

```
expression = term   { OR  term }
term       = factor { AND factor }
factor     = "(" expression ")" | condition
condition  = column operator value
column     = a field name: letters, digits and "_"
operator   = "="  "≠" "!="  ">"  "≥" ">="  "<"  "≤" "<="  "~" (contains)  "^" (starts with)
value      = text, or "text" in double quotes
AND        = "and", or the queryAnd word
OR         = "or",  or the queryOr word
```

| | |
|---|---|
| Precedence | `and` binds tighter than `or`, as everywhere: `a = 1 or b = 2 and c = 3` is `a = 1 or (b = 2 and c = 3)`. Parentheses say otherwise: `(a = 1 or b = 2) and c = 3`. They nest. |
| The joining words | Stand between whitespace and are read in any case: `AND`, `Or`. The English words always work beside the page's (`queryAnd`, `queryOr`), so an English habit holds on a German page. |
| A value | Runs to the next joining word, to the parenthesis that closes its group, or to the end. Parentheses that pair up inside it are its own: `customer = Acme (EU)`. In double quotes it is taken whole — `customer = "Rock and Roll"`, `note ~ ":)"` — up to the next `"`; there is no escape for a quote inside. |
| Only `and`, no column twice | The conditions become the filter row's entries, one per column, as before — parentheses that group nothing are dropped. |
| With `or`, or a column twice | What the filter row cannot hold becomes the [nested filter](#nested-filter), whole, and replaces the one there was: an expression with an `or`, and one with two conditions on one column — `amount > 1 and amount < 5` keeps both. |
| What is wrong | Is said in the status line and stays in the field: an unknown column, a missing value, an operator the type does not take, a value that is not the column's — and `A parenthesis is not closed`, `A closing parenthesis has no opening one`, `A condition is missing` (`()`), `“and” or “or” is missing before “…”` (`(a = 1) b = 2`). |

- **Enter searches**, not every keystroke: a query and an announcement per
  letter would be a barrage for a screen-reader user and a round trip per
  letter for a remote source.
- An expression that does not parse — an unknown column, a missing value, an
  operator the type does not take, a value that is not the column's — is **said**
  in the status line and stays in the field for correcting. It never falls back
  to a free-text search: that would look like it worked and show the wrong rows.
- Free text searches **values**, not what a format prints: `30.00` does not find
  an amount, and `31.12.` does not find a date. It is case-sensitive, like every
  string operator in V1 (S5).
- The field is an ARIA **combobox**: while the last word is a bare prefix of a
  column, a listbox offers the columns. `↓`/`↑` move through it (the focus stays
  in the field), `Enter` takes one, `Escape` closes it — and, closed, empties the
  field and the search.
- A free-text search shows as a chip. It is not part of the view, and restoring
  a view empties it.

### Nested filter

A filter with `or` across columns — `country is DE or amount ≥ 1000` — cannot
live in the filter row, which holds one condition per column and joins them
with `and`; neither can two conditions on one column. The grid holds **one** such filter beside the row: a tree of
conditions joined by `and` and `or`. It has two editors — the
[search expression](#search) and the builder dialog — and both write the same
tree.

| | |
|---|---|
| The chip | One chip says it in words: `country is DE or amount greater or equal 1000`; a group inside a group stands in parentheses, `country is DE and (amount greater or equal 1000 or qty less than 5)`. Its text is a button named `Edit …` that opens the builder; its remove button takes the whole filter, and so does "Remove all". |
| With the rest | Everything is **and-ed**: [the page's `filter`](#the-pages-filter), the filter row, the facets, the free-text search and the nested filter. A row shows when it passes all of them. The facets count under it, like under the filter row; select-all, `get_query`, the exports and the [print table](#printing-the-view) run under it, and the print table's caption says it. |
| The view | [`where`](#the-view) holds the tree, in the wire form of a query's `filter`. |
| Setting it | Is said once, with the result: `66 matches · Filter set: country is DE or amount greater or equal 1000`. The selection is treated as under any filter: one by position goes, one by key stays. `opengrid-view-change` fires. |

**The builder** opens from the chip, and — on a grid with `filter-builder` — from
**Advanced filter** in the toolbar. The button is opt-in; the chip's is not: a
filter a reader can see, a reader can edit.
It is a native modal `<dialog>`: the page behind it is inert, `Tab` stays inside,
`Escape` and *Cancel* close it without a change.

| | |
|---|---|
| A group | *Rows match* **all conditions** or **any condition** — a `<select>` — then its members, then **+ Condition** and **+ Group**. The dialog is the outermost group; a group inside is a `<fieldset>` named `Group 1`, with its own choice, members and buttons, and a `Remove Group 1` button. Groups nest as deep as the expression wrote them. |
| A condition | A group named `Condition 1` with three labelled fields — *Column*, *Condition* (the operators the column's type allows), *Value* (typed by the column: a number, a date, a checkbox) — and a `Remove Condition 1` button. Numbers follow the order on the screen and are given anew when a member goes. |
| Opening | With no nested filter: one empty condition. From the chip: the tree as it stands. The focus is on the first field. |
| Adding and removing | **+ Condition** puts the focus on the new condition's *Column*. A removal puts it on the group's **+ Condition** — never on the document. |
| *Apply* | Also `Enter` in a value. Reads the tree: a group with one member is that member, a group inside a group of the same kind joins it, an empty group is left out. No condition left removes the nested filter. Then the dialog closes and the result says the filter. |
| What is wrong | A missing value, or one the column cannot take, keeps the dialog open: the sentence stands above the buttons, the field is `aria-invalid`, described by the sentence, and takes the focus — so it is read once, with the field. |
| A hidden column | A condition on a column the grid does not show is kept as it is: its fields are disabled, it can be removed. Show the column to edit it. |
| The focus afterwards | On the chip's button when the builder was opened from it and it is still there, else on **Advanced filter** — and without that button on the toolbar's *Filter row* switch, where the focus goes when the last chip is removed. |

### Facets

```js
loader.module.set_columns(host, {
  customer:   { facet: "list" },    // checkboxes with counts
  country:    { facet: "pills" },   // toggle buttons with counts
  amount:     { facet: "range" },   // a From and a To
  ordered_on: { facet: "period" },  // two dates
});
```

With `facets`, a sidebar beside the rows offers them. Only configured columns
get a facet — a range over the ids would be a control nobody asked for.

| | |
|---|---|
| Counting | A facet counts its values **without its own restriction**: with "Alpha" ticked, Beta still shows its own count. One `group` query per counted facet, not one per value — and always (F4). The sidebar's head says what the counts cost: `Counted with 2 queries`. |
| Filtering | Facets are and-ed onto the filter row. The values of one facet are an `or`; NULL is `(no value)` and filters with `is_null` (S1), the empty string is `(empty)` — two different values (S14). |
| Bounds | A bound that is not a value of its column (`abc` as an amount) is named in the status line, not dropped. Every bound has a visible label — `From`, `To` — inside a `fieldset` named for its column. |
| Chips | With the toolbar, each active facet is a chip: `customer is one of Alpha, Beta`. |
| View | `facets: { customer: { values: ["Alpha", null] }, amount: { min: "5", max: "" } }`. |
| Keys | The sidebar holds ordinary checkboxes, buttons and fields; the grid's keys never reach them. |

### Column menu

With `column-menu`, each header has a menu. It is a **second door** to things
that all have a first: sorting from the header, filtering in the filter row,
hiding in the column list, pinning with the header's keys, grouping and
aggregates through `group-by`, `set_columns` or the view.

| Keys | In the menu |
|---|---|
| `↓` / `↑` | next / previous entry, wrapping |
| `Home` / `End` | first / last entry |
| `Enter` / `Space` | do it, close, focus back on the header |
| `Escape` | close, focus back on the header |
| `Tab` | close, and move on from the header — no trap |

The `⋯` in the header is the pointer path, 24px square. It is not a second tab
stop inside the grid: the header cell holds the roving tabindex and says its
shortcut through `aria-keyshortcuts`.

The menu is `role="menu"` and holds **no form**: "Filter …" moves the focus to
the column's field in the filter row, because a field inside a menu breaks the
role. Entries appear only where they do something — aggregates for numbers and
dates, grouping for columns that can be grouped by, "hide" unless it is the last
column. The *Pin* group — *Not pinned*, *At the start*, *At the end* — is in
every menu ([Pinned columns](#pinned-columns)). The menu opens below its header
and never covers it.

### Pinned columns

A pinned column stays in place while the others scroll sideways: at the **start** of the row or
at its **end** — by reading direction, so on a [right-to-left](#right-to-left) page the start is
the right edge.

```js
loader.module.set_columns(grid, {
  id:     { pin: "start", width: 96 },
  amount: { pin: "end" },
});
```

| | |
|---|---|
| The page | `pin` in `set_columns` is where a column **starts**, like `width`. It asks nothing of the column's type; a value that is neither `start` nor `end` is reported like every other mistake in the call. |
| The reader | On the header cell, `Alt`+`Shift`+`←`/`→` takes the column one step towards that side: from a pin at the other end to not pinned, from not pinned to a pin at this one. With `column-menu`, the menu's *Pin* group does the same in one go. The reader's choice leads over the page's. Every change is said in the status line (`customer pinned at the start`); a key that changes nothing says where the column is. |
| The view | `columns.pinned` holds what the reader changed, `{ customer: "start", id: "none" }` — `none` is a pin of the page's the reader took away. A change fires `opengrid-view-change`, like a width or a move; `set_columns` does not, it is the page's. |
| The order | The columns pinned at the start are the **first** of the row and the ones pinned at the end the **last**, each run in the order its columns had. It is one `<table>`, and the DOM, the arrow keys and a screen reader go by the same order as the screen. Pinning a column therefore moves it: one query, around the view the reader has — sort, filters, grouping and the search stay; a selection by position is dropped and said, as for every new order. `Ctrl`/`Cmd`+`←`/`→` moves a column among the columns pinned like it and stops at the edge of that run. |
| Selection column | Pinned at the start whenever a column is. |
| [Row actions](#row-actions) | Pinned at the end whenever a column is. |
| The focus | A cell that takes the focus is scrolled clear of the pinned columns, never under them (WCAG 2.4.11): they are the viewport's `scroll-padding`. A pinned cell, or a pinned column's filter field, is in sight already: the focus on it scrolls nothing sideways. |
| Too narrow | When the pinned columns — the selection column and the action columns that are pinned along counted — would take more than **half** of the grid's width, the pins let go and every column scrolls; the status line says so once, when they let go — not again while the grid stays that narrow: `Too narrow for the pinned columns: they scroll with the others`. The setting stays, in the view too, and holds again as soon as there is room. Half, so that the columns that scroll keep the other half: at 320 px (WCAG 1.4.10) pinned columns alone would be the whole grid. |
| The filter row | A pinned column's field stays above it, and a field that takes the focus is scrolled clear of the pinned ones, like a cell. |
| Groups and totals | Their cells are pinned with their columns. The label of a group or a total runs across the empty cells beside it, as without a pin; while the grid is scrolled sideways and the first column is pinned, the label keeps to that column — it is whole again when its cell has the focus. |
| Looks | A line marks where the pinned columns end. The cells take their row's background, so `::part(cell)` styles them as before. |
| On paper | Nothing is pinned ([Printing the view](#printing-the-view)); the columns print in the order shown. |
| `get_query`, exports | The columns in the order shown, the pinned ones at their end of the row. |

Not here: pinned rows, and a total row that stays in place.

### Column bands

A band is a header above several columns — *Address* above street, city and postcode. A column
names its band in `set_columns`, and the columns that name the same one share its header:

```js
loader.module.set_columns(host, {
  street:   { band: "Address" },
  city:     { band: "Address" },
  postcode: { band: "Address", width: 96 },
});
```

There is one level of bands. A band is **always in one piece**: its columns stand side by side.

| | |
|---|---|
| The page | `band` is the band's name as the reader reads it, in the page's language. It asks nothing of the column's type. An empty name is reported like every other mistake in the call, and so are two columns of one band that `pin` at different ends. |
| The header | Two rows as soon as one shown column has a band. The band's cell stands in the first row, across its columns; the header cells of its columns are in the second. The header cell of a column without a band spans both rows. Without a band the header is the one row it always is. |
| The order | The columns of a band stand together where the **first** of them stands, in the order they have — also when `columns` or a view's `columns.order` names them apart. |
| Moving | `Ctrl`/`Cmd`+`←`/`→` on the header cell of a column in a band moves the column **inside** its band and stops at the band's edge (`city is already at the end`). On the band's cell the same keys move the **band** with all its columns, past the column or the band beside it. A column without a band steps over a band as over one column; it never lands inside. A moved band is said like a moved column, with the place of its first column: `Address moved to position 2 of 6`. |
| Hiding | A hidden column leaves its band; the band's cell spans the columns that are left. Hiding the last shown column of a band hides the band, and showing one again brings it back. |
| Pinning | A band is [pinned](#pinned-columns) as a whole. `pin` on one of its columns in `set_columns`, `Alt`+`Shift`+`←`/`→` on the band's cell or on the header cell of one of its columns, and the column menu's *Pin* group all pin the band: its cell stays in place above its columns, in one piece. The status line names the band (`Address pinned at the start`), and the view holds the choice for each of its columns, `columns.pinned: { street: "start", city: "start", postcode: "start" }`. |
| Widths, sorting, the menu | These belong to the columns: a band has no width of its own and sorts nothing. `Enter`, `Space` and the resize keys do nothing on a band's cell, and the column menu opens from the columns' header cells. |
| The view | Nothing new: a band is the page's configuration, not the reader's choice. A view saved without bands is arranged under them when it is restored. |
| The filter row, groups, totals | One field and one cell per column, as without bands. |
| On paper, exports, `get_query` | The columns in the order shown, under one header row: the bands are not printed or exported. |
| Looks | The band's cell is the part `band`; a line at either side shows which columns it spans. |

**Keys in the header.** With bands the header is two rows, and its cells span: a band's cell
several columns, the header cell of a column without a band both rows. The arrow keys move
through it by what is on the screen.

| Keys | On | Goes to |
|---|---|---|
| `↑` | header cell of a column in a band | the band's cell |
| `↑` | band cell, or the header cell of a column without a band | nowhere: it is the first row |
| `↓` | band cell | the header cell of the column the reader came up from — the band's first column when the cell was reached sideways or with the pointer |
| `↓` | header cell | the first data row, same column |
| `←` / `→` | band cell | the cell beside it in the first row: the next band's cell, or the header cell of a column without a band |
| `←` / `→` | header cell | the header cell of the column beside it, in a band or not |
| `Home` / `End` | band cell | the first / last cell of the first row |
| `Home` / `End` | header cell | the first / last header cell of the columns |
| `Ctrl`+`Home` | anywhere | the first cell of the first row |
| `↑` | a cell of the first data row | the header cell of its column |

`←` and `→` change places on a right-to-left page, as everywhere ([Right to left](#right-to-left)).
Exactly one cell of the grid is in the tab sequence, a band's cell included.

**What a screen reader is given.** The band's cell is a column header for its group of columns
(`<th scope="colgroup">`, spanning them), and each data cell under a band names both its headers,
the band first (`headers`): a cell is announced with its band and its column — *Address, City*.
The header rows are counted: with bands `aria-rowcount` is the number of rows plus two and the
first data row is `aria-rowindex="3"`. Every header cell says its `aria-colindex`, since the cells
of the two header rows are not the columns in DOM order.

Not here: more than one level of bands, and bands in `<opengrid-pivot>`, which has its own
two-level header.

### Grouping

```html
<opengrid-grid datasource="orders" columns="id,customer,country,amount"
               group-by="country,customer"></opengrid-grid>
```

While `group-by` is set, the grid is a **`treegrid`**: group headers carry
`aria-level`, `aria-expanded` and `aria-posinset` / `aria-setsize` (the group's place among
the groups of its level, so a pooled window of rows still says "group 3 of 7"), data rows sit one
level below the innermost group with their own place among that group's rows. The total row is
no group and carries neither. It stays **virtualized** — the group query answers each group's row
count, and from the counts and the open groups the display list is arithmetic,
so `aria-rowcount` is still one number and scrolling still adds no DOM rows.
The rows of a visible window are fetched per group they belong to: one query per
group the window touches, not one per row. With two or three levels, a group above
the innermost one asks once more, when it is first opened: for the groups under it.

| | |
|---|---|
| Keys | `Enter` / `Space` open and close a group; on its first cell `→` opens and `←` closes. |
| Pointer | A click on a group row opens or closes it. |
| Label | One sentence per header, `groupRow`: `country: DE (52 rows)`. NULL is `(no value)` and the empty string `(empty)` — two different groups (S10, S14). |
| Status line | Counts **rows**, not display positions — five headers are not five matches. Opening and closing is said once, with the result that follows. |
| Groupable | Text, booleans, whole numbers and dates. A decimal, a float or a timestamp repeats too rarely to group by and is refused. |
| Refused | More than three levels, a column the grid does not show, or `page-size` alongside: the grid says why in the status line and stays ungrouped. |
| View | `group` and `expanded` (paths of keys; a NULL key is `null`) travel in the [view](#the-view). |

**Aggregates** go into the group rows and into a **grand total**, the last row
of the list. A column shows one only when a page chose it with `set_columns`
(`aggregate`) or a reader chose it in the view (`aggregates: { amount: "sum" }`,
which leads) — there is no default, because "sum every number" would sum the ids.

| | |
|---|---|
| Allowed | `count` on every column; `sum` and `avg` on numbers; `min`, `max` and `range` on numbers and dates. Anything else is named in the status line. |
| Range | The smallest and the largest value, `1.1.2026 – 31.12.2026` — the dates a group spans. Asked as `min` and `max`; a group whose rows all hold one value shows it once. |
| Exact | A `sum` over a decimal is a decimal — exact past 2^53 (S8). An `avg` is a float, by the query model's result types (S12). |
| NULL | Skipped by `sum`, `avg`, `min`, `max`; not counted by `count` (S11). An aggregate over nothing is empty on screen and says `(no value)`. |
| Spoken | The cell shows a glyph (`Σ ⌀ # min max`) and says a word: `Sum: 1,234.00`. The glyph has an empty alternative. |
| Total | A row at the **end** of the list, not a sticky footer: the arrow keys and a screen reader reach it like any row. It opens nothing and cannot be selected. |

**While grouped, a row number names nothing**: it would be a display position —
a header as often as a row — and would move on every toggle. So a grouped grid
edits and selects **by `row-key`**, and without one it does neither.

**Editing** (issue #190): the data rows of a group edit, and the change
names its record by `key`; the `row` in the event is a display position and not
for the page to use. Group headers and the total stay read-only. A fresh result
(a replaced source) brings fresh aggregates for every group.

### Selecting by key

A grid whose records have a key selects **by key** (issue #322):

| The grid | The selection names | `opengrid-selection-change` | In the [view](#the-view) |
|---|---|---|---|
| `row-key`, a field among the `columns` — grouped or not, with or without `page-size` | records, by their `row-key` value | `{ count, keys }` | `selected` |
| a [tree](#tree) | nodes, by their `tree-key` value | `{ count, keys }` | `selected` |
| neither | positions in the result | `{ rows, ranges, count }` | — |
| `group-by` without `row-key` | nothing: a display position is a header as often as a row | — | — |

What selecting by key does, with `row-key`:

| | |
|---|---|
| `Space` on a data row, or its selection cell | Selects or deselects the record. |
| `Shift`+`Space` | Selects every data row from the last toggled row to this one, keeping what is selected. While grouped these are the rows **in the display list**: group headers in between are passed over, and a closed group has no rows in the list, so none of its rows are taken. When the last toggled row is not in the list — nothing toggled yet, its group closed, or another record in its place after a sort or a filter — it is a plain toggle. |
| `Ctrl`/`Cmd`+`A`, the selection header | **Every row of the filter** — [the page's](#the-pages-filter) and the reader's together, and while grouped, whether its group is open or not. The header clears the selection when all of them are selected already, and says the count (`selectedAll`). |
| A group header | Is not selected and selects nothing: `Enter`, `Space` and a click open and close it. The total is not selected either. `aria-selected` is on data rows only. |
| Opening and closing a group | Never changes what is selected. The rows of a closed group stay selected, and show it when it opens again. |
| Sort, filter, a scroll, another page, grouping, ungrouping, another `group-by` | The selection stays: a key names a record, not a position. After a filter it can hold records that match no longer; the header then shows `mixed`. |
| Another `row-key`, a `row-key` added or taken away, a replaced source, another page [`filter`](#the-pages-filter) | The selection is **dropped**, said in the status line (`selectionCleared`) and reported with an `opengrid-selection-change` that holds nothing — keys of another field name nothing, positions and keys do not turn into each other, and whether a record is in a new source, or under another page filter, nobody has asked. |
| The event | `{ count, keys }`: `keys` are the `row-key` values of the selected records, in no particular order, and `count` counts them. There is **no `rows` and no `ranges`** — a position names no record, and most selected records are not loaded. |
| The source | A selected record is known by its key alone, so the keys behind a `Shift` range that reaches past the loaded rows and behind "every row" are asked of the source, in the grid's order: one query per group a range touches, the rows of the filter in pieces of 10 000. |

A [tree](#tree) keeps its own rules for what `Shift` and "all" take — the nodes shown — and the
same event and view.

**The view carries the selection by key** as `selected`, and `set_view` restores it:

| | |
|---|---|
| `get_view` | `selected` is the list of selected keys — a snapshot, like the rest of the view. It is left out while nothing is selected, and always for a selection by position. |
| Selecting | fires `opengrid-selection-change` and **never** `opengrid-view-change`: a page that stores the view on every view change stores the selection as it was then. |
| `set_view` | replaces the selection with the view's. A view without `selected` selects nothing, the rule of every absent field of a view — the selection there was is dropped and said. In a grid that selects by position `selected` is not used. |
| Untrusted keys | The keys are checked before anything is selected: `selected` must be a list of strings, numbers and booleans, or the view is refused like one that names an unknown column. Then the **source is asked** which of them name a record — only keys of the key column's type are asked, in pieces of 1 000 — and only those are selected. The check runs under [the page's `filter`](#the-pages-filter), which says which records this grid has: a record it hides names none here. The reader's filter does not count — a selected record may be filtered out of sight. |
| What is missing | Keys that name no record are dropped, and said once with the result: `17 matches · 3 selected rows no longer exist` (`selectionMissing`). `opengrid-selection-change` fires with what remains. |
| A failed check | is said as an error with the result, and nothing is selected — not a silently empty selection. |
| Cost | A view with `selected` costs the check beside its one query: one query per 1 000 keys, and one for the key column's type when no result has told it yet. |

[Setting a value for the selected rows](#setting-a-value-for-the-selected-rows)
stays off while grouped.

### Adding rows

```html
<opengrid-grid datasource="people" columns="id,name,room" row-key="id" add-row></opengrid-grid>
```

```js
grid.addEventListener("opengrid-row-add", async ({ detail }) => {
  try {
    const saved = await post(detail.values);
    loader.module.set_row_state(grid, detail.provisionalKey, "saved", saved.id);
  } catch (error) {
    loader.module.set_row_state(grid, detail.provisionalKey, "error", `Not saved: ${error.message}`);
  }
});
```

With `add-row` the grid shows an empty **new row** after the last one. Typing into it opens the
editor as on any cell, `Enter` goes on to the next column, and `Enter` in the last column — or
`Ctrl`/`Cmd`+`Enter` anywhere in the row — adds the record: `opengrid-row-add` fires with the
`values` by field name and a `provisionalKey`, the row appears at once marked unsaved, and a fresh
new row holds the focus in its first column. The page answers with `set_row_state`; the message of
an `error` is said in the status line, as `set_cell_state` says one. An added row is read-only; the
page loads fresh rows (`loadRows`) and the saved ones are the source's from then on. Added rows and
the undo history stay when the columns are moved or hidden; another `datasource` ends them.

A field that is not nullable in the schema is marked required in the new row. Adding without a
value for one is refused: nothing fires, the cell is marked invalid and the status line says
`{column} needs a value`. Off while grouped, in a tree and with `page-size`.

### Lists

A column of type `list<utf8>` or `list<int64>` holds an ordered list of values. The cell shows them
as **chips** (`part="list-item"` inside `part="list"`; `role="list"` and `listitem`). The editor is
the ARIA **combobox** over the page's `set_choices`: the text field filters the choices as you type
(`aria-expanded`, `aria-activedescendant`, the number of choices said in the status line), `↓`/`↑`
move over them, `Enter` picks, `Backspace` on an empty field takes the last chip back, and `Enter` on
an empty field commits. A text that is none of the choices is **refused** with the reason under the
editor, never dropped. `{ value, label }` pairs show the label in chips and options and give the
page the value.

```js
loader.module.set_choices(grid, {
  subjects: [{ value: "m", label: "Maths" }, { value: "p", label: "Physics" }],
});
```

A list column with no choices has no editor. Emptied, a list becomes the empty list — a NULL cell
stays NULL if nothing was picked. The `opengrid-cell-change` for a list column carries the JSON
array as its `value`. A plain column with choices uses the same combobox in place of the old
`<select>`: one pick, committed with it.

Filtering a list column: **contains** (`has`), **contains all** (`has_all`, values separated by
commas) and **is empty** (`is_empty`), besides the null checks. It is not sorted, grouped or
faceted; its only aggregate is `count`. In CSV a list is the JSON array in the cell; see
[Query semantics](../guides/query-semantics/) for what a list is.

### Lookup columns

A column that stores a key — `customer_id`, a country code — shows the key. With a `lookup` in
`set_columns`, the page gives the labels, and the reader reads and chooses by them:

```js
loader.module.set_columns(grid, {
  customer_id: { title: "Customer", lookup: { 1: "Alpha GmbH", 2: "Beta AG", 3: "Gamma Ltd" } },
  country:     { lookup: { DE: "Germany", FR: "France" }, facet: "list" },
});
```

The map is the only source of the labels: one plain object per column, the stored key on the left,
the label on the right. The grid asks no second source for them, and the query model has no join.
**The label is display. The value is the key** — in the query, on the wire, in the view and in every
event.

**Sorting a lookup column sorts by the key, not by the label.** Sorted ascending, `1`, `2`, `3`
stand in that order whatever their labels say — *Zeta* before *Alpha* if those are the labels of
`1` and `2` — and a reader who sees only labels sees an order that is not the alphabet's. The header
says "sorted ascending" all the same: it is, by the key. A page that needs the order of the labels
gives the source a column that holds them and sorts by that one, or uses keys that sort like their
labels.

| | By | |
|---|---|---|
| Cell | label | A key the map does not name shows **the key itself**; NULL shows nothing. The label leads over a format of `set_formats` for the column. Labels are text, so the column lines up at the start, number keys included, unless `align` says otherwise. |
| Sort | key | See above. |
| Grouping | key | The groups are the keys, in the keys' order; a group's header names it by its label, `Customer: Alpha GmbH (12 rows)`. |
| Aggregate | key | `count` only: a key is not an amount, and the smallest key is not the first label. Any other aggregate is refused for the column — in `set_columns`, and from a view — and the column menu offers none. |
| Editor | label | The ARIA **combobox** of [Lists](#lists) over the map's labels, in the map's order: typing filters them, `Enter` picks, and a text that is none of the labels is refused under the editor. What is committed is the key, and `opengrid-cell-change` carries the key as its `value`. Where the page also gave the column `set_choices`, those choices are what the editor offers, named by the map's labels where they have none of their own. |
| Filter row | label | The column's value is **a choice among the map's labels**, with *is* and *is not* (and the null checks where the column may be null). The filter it writes compares the key, and the view's `filters` hold the key. The "+ Filter" dialog and the filter builder offer the same choice, and a filter's chip — the nested filter's too — names the label. A filter on a key the map does not name — a view saved under other labels — stays, named by the key. |
| Facets | label | `list` and `pills` fit a lookup column of either type, and list the keys **the rows hold**, by label, in the keys' order. The selection in the view is the keys. `range` is refused: a span of keys is not a span of labels. |
| Search | key | An expression compares the stored value — `customer_id = 2`, with `=` and `≠` only, joined by `and` or `or` like any other — and the free-text search reads values, as everywhere ([Search](#search)): a label is not found by it. |
| `get_query`, export | key | [`get_query`](#exporting-the-view) and `exportRows` hand out the stored values, as for every column: what a file is read back from. |
| Print | label | [`prepare_print`](#printing-the-view) writes the values as the grid shows them, the caption as the chips say it. |
| Paste, `set_values`, validators | key | What the page or the clipboard writes into the column is the stored value. |

A lookup fits a column of type `utf8` or `int64`. The keys of the map are JavaScript property names,
so text: for an `int64` column each is the number's digits (`{ 7: "Seven" }` and `{ "7": "Seven" }`
are the same map), and one that is not a whole number is reported. So are an empty key, a label
that is not a non-empty string, and a `lookup` that is not a plain object — as every mistake in
`set_columns`, in the status line, with none of the call applied. The options of the editor and of
the filter row stand in the order JavaScript lists the object's keys: whole numbers ascending, then
the others as written. The labels are the page's words, in the page's language. To change them,
call `set_columns` again with the whole configuration.

### Undo and redo

`Ctrl`/`Cmd`+`Z` undoes the last thing the reader did in the grid, `Shift`+`Ctrl`/`Cmd`+`Z` (and
`Ctrl`+`Y`) does it again. The history is the grid's own — cell edits, rows added and a
[paste](#pasting) as **one** step — not the page's saves. Each step fires the events the original
fired — `opengrid-cell-change`, `opengrid-row-add`, `opengrid-paste` — with `undo: true` (`redo: true`
going forward), so the page saves it as it saves an edit; an undone row add means "delete that
`provisionalKey`", and a redone one is a new add under a new key. The cell shows the old value, marked
unsaved, and the focus goes to it; the status line says `Undone: amount in row 12`. A new edit ends
the way forward. The history ends when the source is replaced (`loadRows`), and without a `row-key`
also when the sort or the filter changes, since a position then names another record. With a
`row-key` the record decides: when it is not among the loaded rows (a filter took it out, a scroll
moved on), the undo does not write into the grid — its old position holds another record now — and the
event still reaches the page with the record's `key`. A value the page sets with `set_values` ends
the steps it overrides, so the grid does not take back what the page has just said. A
[bulk change](#setting-a-value-for-the-selected-rows) is not a step: it ends the history. An open
editor keeps the browser's own undo of its text.

### Validation

```js
loader.module.set_validators(grid, {
  // a function: it never blocks
  seats: (value, row) => (Number(value) > 60 ? { level: "warning", message: "Over 60 seats is a hall." } : null),
  // an object: an error blocks the commit
  name: {
    validate: (value) => (taken.has(value) ? { level: "error", message: "That name is taken." } : null),
    block: true,
  },
});
```

The function runs on every keystroke in the editor and once more on commit; `value` is the text as it
stands (as `opengrid-cell-change` gives it) and `row` the record as loaded, by field name. It
answers `null`, or `{ level: "warning" | "error", message }`. The message shows under the editor,
stays on the cell (`data-validation`, described by a hidden node through `aria-describedby`; an `error` is also `aria-invalid`) until
the next result, and is said once, politely, in the status line.

**A finding does not block the commit** — the reader may save a duplicate and fix it later — unless
the column is given as `{ validate, block: true }` and the answer is an `error`: then the editor stays
open with the message. What the schema refuses (a letter in a number) always blocks. For a finding the
page only knows after saving, `set_cell_state(host, key, column, "invalid", message)` marks the record's
cell the same way and keeps it across results.

### Setting a value for the selected rows

With rows selected, `F2` — or **Set for selected rows** in a column's [menu](#column-menu) — opens
that column's editor once, in a small panel above the rows. `Enter` (or the button) sets the value,
`Escape` drops the panel. One `opengrid-bulk-change` fires with `{ keys, column, value }`: the
`row-key` values of the selected records, the column and the value as text. The cells in view show
it, marked unsaved; the page saves and answers with `set_cell_state` per key. It needs `row-key`,
and a column the page fills (`readonly` in `set_columns`) refuses it, said in the status line. Off
while grouped and in a tree. The keys are the [selection's own](#selecting-by-key).

### Editing a whole record

`Shift`+`F2` on a data cell opens the focused record as a
form: a native modal `<dialog>` (`part="record-dialog"`) the grid builds from the schema, one labelled
field per column in column order, each the editor its cell would open (a checkbox, a date, a
combobox over the page's choices). A column the page fills (`readonly` in `set_columns`) is shown,
not editable. The page behind the dialog is inert and `Tab` stays inside it; `Escape` or **Cancel**
closes it and changes nothing; `Enter` in a field, or **Save**, saves. The focus goes back to the cell
it came from.

Before it opens, the grid fires a **cancelable** `opengrid-record-open` with `{ key, row }`: a page
that wants its own form calls `preventDefault()` and the grid opens nothing. It needs `row-key` (the
page names the record, as for [setting a value for the selected rows](#setting-a-value-for-the-selected-rows)),
and is off while grouped and in a tree.

**Saving is the edits it holds, fired as cell edits.** Each field is read as the cell editor reads it
(an empty field is `NULL` where the column allows it) and checked first: a value the column cannot hold,
an empty required field, or an `error` from a `block: true` [validator](#validation) stays
in the dialog with the sentence under the field (`aria-invalid`, described by it) and the focus on the
first such field; **nothing is saved until every field is fine.** Then one `opengrid-cell-change` fires
per field whose value changed, in column order, exactly as for a cell edit — the page saves as it
already does — and the cells show the values, marked unsaved. A record saved with nothing changed
fires nothing and says so in the status line. The edits are **one step** of the [history](#undo-and-redo):
`Ctrl`/`Cmd`+`Z` puts all of them back, firing one `opengrid-cell-change` per field with `undo: true`.

### Copying a range of cells

`Shift`+arrow keys on a data cell mark the block between where it began and where the focus is;
`Shift`+click and dragging with the pointer do the same. The live region says its size — „3 by 4
cells“ — and each cell in it carries `data-range` (a tint with an outline, which holds in forced
colours). **The row selection is another thing**: `Shift`+`Space` and the checkboxes select rows, and
no cell says `aria-selected` for a range. Any key that moves or edits, a click elsewhere, and another
result for the whole query end the range. Off while grouped and in a tree.

`Ctrl`/`Cmd`+`C` puts the block on the clipboard as a spreadsheet reads it: tab-separated, a line
break after each row, a cell holding a tab, a line break or a double quote in double quotes. A cell is
its **value**, not what the column's format shows: `1234.5`, `2026-10-26`, `true`, a list as its
elements joined by commas, `NULL` as nothing. `Ctrl`/`Cmd`+`V` reads it back — here, into a block of
the same shape, and into a spreadsheet — with `NULL` as an empty cell (an empty text stays empty).

**The formula guard of the [export](../guides/export/#the-formula-guard) applies:** a text cell that
starts with `=`, `+`, `-`, `@`, a tab or a carriage return gets a leading `'`, so a spreadsheet runs
nothing a user typed. Pasting into a text column of the grid takes that `'` off again. A copy covers
the rows that are **loaded**: a range that reaches past them says so (`rangeUnloaded`) and copies
nothing — scroll, or narrow the range. From a field (the filter row, the search, an open editor) the
browser copies as it always did.

### Pasting

`Ctrl`/`Cmd`+`V` on a data cell reads the clipboard's text — tab-separated, as every spreadsheet
copies, or CSV — and anchors it at the focused cell. **Nothing changes yet:** a preview
(`part="paste-preview"`) shows each incoming value read against its column's type, in the number
notation of the grid's language (`1.234,5` in German; the wire notation `1234.5` is understood
too). A value that cannot be read is marked with the reason, and the live region says it:
„12 rows, 2 values not readable“. `Enter` applies, `Escape` discards. A block with unreadable
values is not applied. A column the page fills (`readonly` in `set_columns`) is not pasted into:
its cells stay in the preview marked `aria-readonly` with the reason („price is read-only“), the
sentence names the column, and the rest of the block is applied. The preview belongs to the rows it
was read from: a new result for the whole query — another sort or filter, a source replaced, a
rebuilt grid — discards it.

Applying fires **one** `opengrid-paste` for the whole block: `changes` has one entry per cell of
an existing row (`key` with `row-key`, `row` always, `column`, `value` as text, `previous`), and
the cells are marked unsaved as an edit's are; `added` holds the rows past the end, which become
[added rows](#adding-rows) when `add-row` is set and are reported as cut off otherwise. The page
saves as for single edits (`set_cell_state`, `set_row_state`). A cell that looks like a formula
(`=SUM(…)`) is text in a text column and unreadable in a number column — never evaluated. The
existing rows a block covers are asked of the source, so a block longer than the window is the
same as one inside it; of those, the cells in the loaded window show the new values at once.
Off while grouped and in a tree.

### Row actions

```js
loader.module.set_actions(grid, [
  { id: "open", label: "Open" },
  { id: "remove", label: "Remove" },
]);
grid.addEventListener("opengrid-row-action", ({ detail }) => run(detail.id, detail.key));
```

Each action is a trailing column named by its label, with a button on every row — a cell the
arrow keys reach like any other. `Enter` or `Space` on the cell (or a click on the button)
fires `opengrid-row-action` with `{ id, row, key }`; `key` is the record's `row-key` value
(`row` is a position, and in a grouped grid a display position). The label is the accessible
name. Group headers and the total have no button. Icons are the page's: the label is text.

### Tree

```html
<opengrid-grid datasource="staff" columns="name,title,region"
               tree="parent_id" tree-key="id"></opengrid-grid>
```

With `tree` set, the rows are a hierarchy — each row names its parent in the
`tree` field, by the key in `tree-key` — and the grid is a **`treegrid`** that
loads a level at a time: the roots first, a node's children when it is opened.
Each level comes with every node's child count, so a leaf shows no chevron and
offers nothing to open. Rows are indented by depth in the first column. A walk-through is
the guide [A tree in the grid](../guides/tree/).

| | |
|---|---|
| Keys | On a node's first cell `→` opens it, and on an open node goes to its first child; `←` closes an open node, and on a closed node or a leaf goes to its parent. `Enter` and `Space` keep their grid meaning — `Space` selects the node. |
| Pointer | A click on a node's first cell, where the chevron is, opens or closes it. |
| Spoken | `aria-level` (depth), `aria-posinset` / `aria-setsize` (its place among its siblings), `aria-expanded` only where there is something to open, `aria-busy` while the children load. Opening and closing is said once, with the result, as `groupExpanded` / `groupCollapsed`. |
| Filter | A match keeps the path to it: its ancestors are shown as **context**, muted and described with `treeContext`, through `aria-describedby`. The status line counts matches, not the context. |
| Orphans | A row whose parent does not exist is a root, and the status line says how many there are (`treeOrphans`). |
| Order | Siblings are sorted by the grid's sort; the hierarchy is never broken by it. |
| Summaries | A column's aggregate — from `set_columns`, the column menu or the view's `aggregates`, as in groups — is shown on every node **with children** as its subtree's summary beside its own value: `120 · Σ 1,450`, said `subtreeCell` ("120 · Σ 1,450, Sum of the subtree"). Over the node and all its descendants, from the raw rows, and with a filter over the matches only — context does not count (rule T7). A leaf shows its own value. |
| Size | A level holds at most 10 000 nodes — a larger one is refused with `treeLevelTooLarge`; narrow it with a filter. |
| Refused | `page-size` alongside: a tree scrolls (`treeRefused`). `group-by` alongside is not used (`treeGroupIgnored`), and the toolbar's + Group is `aria-disabled` and says so. |
| View | `expanded` holds the open nodes, each as `[key]`, and travels in the [view](#the-view). |
| Export | `get_query` asks for the whole tree, flat: every node depth-first, with `level`, `path` (its keys from the root) and, with a filter, `match` — see [Exporting](../guides/export/). |

**In a tree, the selection names nodes by their key** (`tree-key`), so it
stays when nodes close over it and when the grid is sorted or filtered — a key
names a record, not a position. A node is selected by itself, never with its
children. `Shift` extends over the nodes shown; `Ctrl`/`Cmd`+`A` and the
selection header select **every node shown**, since a closed level was never
loaded. `opengrid-selection-change` is `{ count, keys }` — the selected nodes, the
shown ones first, and their number; it has no `rows` and no `ranges`. The
[view](#the-view) carries the nodes as `selected`, and `set_view` restores
them — see [Selecting by key](#selecting-by-key).

**Editing is off in a tree**: an edit would be reported against a display
position, which moves on every toggle.

## `<opengrid-table>`

A plain native `<table>` for **display**: maximum semantics, ordinary copy and
paste, browser find. No virtualization, no roving tabindex.

```html
<opengrid-table label="Orders" datasource="orders" columns="id,customer,amount">
</opengrid-table>
```

| Attribute | Meaning |
|---|---|
| `label`, `datasource`, `columns`, `theme` | As in the grid. |

Header buttons sort a single column, `none → ascending → descending → none`.

**Which of the two?** Use `<opengrid-table>` when the answer is read and
`<opengrid-grid>` when it is worked with. A `role="grid"` announces itself as an
interactive widget and takes over the arrow keys; for a report table that is the
wrong promise, and the W3C says to use native HTML where it suffices.

## `<opengrid-pivot>`

A native `<table>` with a two-level column header. Deliberately **not**
virtualized: an accessible virtual pivot is the highest risk in this project,
and the row and column limits are what make rendering the whole thing safe.

```html
<opengrid-pivot label="By country and year" datasource="orders"
                rows="country" columns="ordered_year"
                values='[{"field":"qty","fn":"sum","as":"total"}]'></opengrid-pivot>
```

| Attribute | Meaning |
|---|---|
| `label`, `datasource`, `theme` | As in the grid. |
| `rows` | Comma-separated row dimensions, outermost first. |
| `columns` | Comma-separated column dimensions, outermost first. V1 allows **two**. |
| `values` | The measures, as the contract's own JSON — not an invented shorthand. A measure with `calc` is [calculated](#calculated-measures). |
| `sort` | How each level is ordered, as the wire's JSON: `[{"field":"country","by":"total","direction":"desc"}]`. The reader's sort buttons write it. |
| `collapsed` | The folded groups, a JSON list of group paths: `[["DE"],["FR","Beta"]]`, NULL as `null`. The reader's fold buttons write it. |
| `toolbar` | Shows the field toolbar — see below. Presence is what counts. |
| `fields` | The fields a reader may pivot by, comma-separated. |
| `measures` | The measures a reader may add, as the contract's JSON: `[{"field":"amount","fn":"sum","as":"revenue"}]`. |
| `filter` | The filter on the raw rows, as the wire's JSON: `{"field":"country","op":"eq","value":"DE"}`, `{"and":[…]}`. The reader's filter field and chips write it. |
| `drill-down` | Makes every value a button that fires `opengrid-drill-down` with the rows behind it — see [Drilling down](#drilling-down). Presence is what counts. Opt-in, and not part of the view. |

`rows`/`columns` are the two **axes** here, as in every pivot; in the grid
`columns` is the projection. The words are standard in their own context, so
they were kept rather than made unique and worse.

Subtotal rows carry `data-level` and `data-total`, and their row header says so
in words. A group whose dimension value is NULL is named `(no value)`, and one
whose value is the empty string `(empty)` — they are different groups, and an
empty header cell is silence to a screen reader.

**Titles and formats** are the page's, as at the grid and the table.
`set_columns` gives a row dimension or a measure (by its alias) its `title`; a
title for a name the pivot does not show now is fine, since its dimensions change
with its attributes. A pivot takes nothing but a `title` — `width`, `align` and
the rest are reported in the status line. `set_formats` formats a dimension's
values in the row, subtotal and column-group headers and a measure's cells, keyed
by the field name or the measure alias. NULL and the empty string keep their
words: a format never blanks a header.

```js
loader.module.set_columns(pivot, { country: { title: "Country" }, total: { title: "Quantity" } });
loader.module.set_formats(pivot, { total: { kind: "number", locale: "de-DE" } });
```

**The view of a pivot** is what it pivots by and measures — its
attributes as one value, `get_view`/`set_view` as at the grid:

```js
const view = loader.module.get_view(pivot);
// { rows: ["country"], columns: ["ordered_year"],
//   values: [{ field: "qty", fn: "sum", as: "total" }], sort: [], collapsed: [],
//   filter: null }
loader.module.set_view(pivot, { ...view, rows: ["customer"] });
```

The attributes **are** the view, so there is no second copy to fall out of step:
`set_view` writes all of them at once and costs one query, and every change of
them — by `set_view`, a sort button or the page setting an attribute — fires
`opengrid-view-change`. Setting what it has says nothing. A view whose `rows` or
`columns` is not a list of names, whose `values` is not a list of measures or
whose `sort` is not a list of orders or `collapsed` not a list of paths or `filter` not a filter object is reported in the status line and applied
not at all. A part left out is empty; a key the pivot does not know is ignored. In TypeScript, `get_view<PivotView>(pivot)`; the adapters' `view`
on `OpengridPivot` is a `PivotView`. `opengrid-query` fires after every answer,
as at the grid.

**Sorting** orders each level's rows among their siblings; a subtotal stays after
its group and the grand total last. Every row-dimension header is a sort button:
it toggles its level between ascending — the default — and descending by its
values, with `aria-sort` on the `<th>`. Without a column dimension a measure's
header is a sort button too and orders every level by that measure. With one it
is not: a measure orders by the whole row, across every column value, and a
header over one column value would promise an order its cells do not have. NULL
sorts last either way. The pivot keeps the focus on the pressed button once the
new answer is drawn. See the [protocol](../protocol/) for the `sort` a server
takes.

**Folding.** With two row dimensions or more, a group's subtotal row header is a
button with `aria-expanded`, named by the header's own words ("Total DE"): it
folds the group's rows away, so only its subtotal stays, and opens them again.
Folding asks nothing — the pivot draws what it holds again — and the status line
says it with the grid's texts `groupCollapsed` / `groupExpanded`. The focus stays
on the button. The folded groups are the `collapsed` attribute and part of the
view; a path that matches no group folds nothing. The export stays the whole
pivot, as the grid's export ignores its open groups.

#### Calculated measures

A measure is an aggregate; with `calc` it is shown as a share or as a running total instead:

```html
<opengrid-pivot datasource="orders" rows="country" columns="ordered_year"
  values='[{"field":"qty","fn":"sum","as":"total"},
           {"field":"qty","fn":"sum","as":"share","calc":"share_of_row"}]'></opengrid-pivot>
```

| `calc` | The cell shows |
|---|---|
| `share_of_row` | its part of the row's total across every column |
| `share_of_column` | its part of the column's total — the column's cell in the grand total row |
| `share_of_total` | its part of the grand total |
| `running_total` | the sum of the row's cells up to and including its column; it starts again in every row |

The source computes it — the engine in the tab as a server does, by the same rules
([P10–P12](../guides/query-semantics/#calculated-measures-in-a-pivot)) — so a calculated value
is a value like any other: in the table, in `get_pivot`'s CSV and in the XLSX. A share goes with
`fn` `sum` and `count`; a running total also with `avg`, `min` and `max` over a number, where it
adds up what the cells show. Anything else the source refuses, and the pivot's status line says
so. Subtotals and the total row follow the same rule as the rows above them, over their own
cells.

- **A share is a fraction**, `0.25`, and NULL — an empty cell — where its total is NULL or zero.
  Percent and digits are the page's: `set_formats(pivot, { share: { kind: "number", style:
  "percent", maximumFractionDigits: 1 } })`, by the measure's alias. A running total keeps its
  measure's type and takes the format the measure would.
- **The header says it.** A calculated measure's column header is its title followed by what it
  shows, in the element's [texts](#texts): `Quantity · share of row`. The CSV and the XLSX head
  the column the same way after its path and alias, `2025 · share · share of row`.
- **Sorting** by a calculated measure orders the rows by its aggregate, as for any measure.
- **Drilling down** from a calculated cell names the cell's own rows, as from any cell — for a
  running total the rows of its column, the last part added, not the columns before it — and
  `value` is the calculated value.

With `toolbar`, a measure's chip has a button that names what the measure is shown as — `value`
at first; its name is "Show Quantity as: value". It opens a menu of the choices its function
has, the one in use checked (`menuitemradio`), with the keys of the add menu: the value and the
four calculations for `sum` and `count`, the value and the running total for `avg`, `min` and
`max`. The pivot does not know the source's schema, so it offers the running total of a `min`
or `max` over text as well; the source refuses it, in the status line.
Choosing writes `calc` into `values`, so it is one query and one `opengrid-view-change`, and the
focus comes back to the button, which now names the new choice. A page that formats shares as
percentages listens to that event and sets the format of a measure whose `calc` changed.

**Choosing the fields.** With `toolbar`, the reader chooses the rows, the
column and the measures — without dragging, from the keyboard. Per axis a
labelled group holds the chosen fields as chips — each with "Move … earlier" and
"Move … later" where it has a neighbour, and "Remove …" — and an add button
("+ Row", "+ Column", "+ Measure") whose menu offers what the page offers and
the pivot does not use yet: `fields` for the dimensions, `measures` for the
measures, whole. The pivot does not know the source's schema, and a measure
needs a function the page chooses. "+ Column" is `aria-disabled` while there are
two column fields (V1's limit), and so is an add button with nothing left; its name
then says why. The menu is the grid's: arrows, Home/End, Enter or Space to pick,
Escape back to the button. Every change is one write of the view — one query,
one `opengrid-view-change` — and a sort or folded groups that name what went
go with it. The focus comes back to the control pressed.

```html
<opengrid-pivot toolbar datasource="orders" rows="country" values='[{"fn":"count","as":"orders"}]'
  fields="country,status,order_year"
  measures='[{"fn":"count","as":"orders"},{"field":"amount","fn":"sum","as":"revenue"}]'></opengrid-pivot>
```

**Filtering.** The filter acts on the raw rows, before the pivot, so every
subtotal is still computed from what passes it. A page sets it with `filter`;
with `toolbar` the reader has a "Filters" group: a field for an expression in
the grid's language — `country = DE and qty ≥ 3`, `and` or the page's word for
it — and a chip per clause of the filter ("country is DE"), each removable, and
"Remove all". Enter adds the clauses — one view, one query — and empties the
field. The pivot has no schema of its own, so it asks for one (a query with
`limit` 0) over the fields it knows: `fields`, its rows and columns, and the
measures' fields. What does not parse, and free text without an operator, is
said in the status line with the grid's sentences, and nothing changes. The
field needs the grid's parser, so it is in the default module; a pivot-only
build keeps the attribute.

**How large a pivot gets.** It is drawn whole — no paging, no virtualization, since
an accessible virtual pivot is the highest risk in this project — so it is bounded by what
a browser draws as a table in about a second: **131 072 cells** (rows × generated columns),
at most 10 000 rows and 256 columns. The time to draw grows with the cells alone, not with
their shape (`just measure-pivot` measures it: about 6 µs a cell in Chromium, 10 µs in
WebKit). Above a limit the pivot says so in its status line and what to narrow; it never
shows part of an answer.

**Two column dimensions** (`columns="ordered_year,status"`) give a header row per
dimension, outermost first, then the measures: a year spans all its statuses,
a status its measures. That makes the pivot a complex table, so every header
cell carries an `id` and every value cell `headers` naming its row and column
headers — a screen reader reads "DE, 2025, open, revenue" without inferring
which spanning header is meant. The combinations are the ones that occur (P3);
there are no column subtotals.

`get_pivot(host)` exports the table as it is shown, as CSV — see
[exporting a pivot](#exporting-a-pivot). Its headers and values stay the raw
names and values: titles and formats are display.

### Drilling down

A pivot cell is a number; `drill-down` lets a reader ask which rows it is made of.
The pivot shows no rows itself: it fires **`opengrid-drill-down`** with the cell's
filter, and the page shows the rows — where and how is the page's.

```html
<opengrid-pivot drill-down datasource="orders" rows="country" columns="ordered_year"
                values='[{"fn":"count","as":"orders"}]'></opengrid-pivot>
```

With the attribute every value is a native button (`part="drill-down"`): a click,
`Enter` or `Space` fires the event, and the buttons are in the tab order like the sort
and fold buttons — a native table has no arrow keys of its own. A button is named by the
value and what pressing it does, `12, show rows` (the text `pivotDrillDown`). A cell
without a value has no button. Without the attribute a pivot has none at all — a
button nobody listens to would be a control that does nothing — so a page sets it
together with its listener.

| `detail` | |
|---|---|
| `query` | The rows behind the cell, as the wire writes a query: `{ source, filter }`. `source` is the pivot's `datasource`. |
| `query.filter` | The pivot's own `filter`, clause by clause, and-ed with one clause per dimension the cell lies in: the row dimensions down to the row's level, then every column dimension — `{"and":[{"field":"country","op":"eq","value":"DE"},{"field":"ordered_year","op":"eq","value":2025}]}`. One clause alone is that clause; none — the grand total of a pivot without a column dimension and without a filter — and `query` has no `filter`. |
| `row`, `column` | The cell's row and column group, as paths of keys like `collapsed` has them, NULL as `null`. A subtotal's `row` is shorter than `rows`, the grand total's is `[]`. |
| `measure`, `value` | The measure's alias and the cell's value as the answer carried it — of a [calculated measure](#calculated-measures), the calculated value. |

- **NULL and the empty string stay two groups.** A NULL key is
  `{"field":"country","op":"is_null"}` — `eq null` would match nothing (S1) — and the
  empty string is `{"field":"country","op":"eq","value":""}` (S14).
- **A total filters by what it lies in.** A subtotal names its group's dimensions and
  not the ones below it; the grand total names no row dimension. A subtotal row never
  becomes an `is_null` on the dimensions it sums over.
- **The count.** The rows `query` answers are the rows the cell was computed from, so
  a `count` without a field is their number. A measure over a field skips that field's
  NULLs, as in every aggregate; the rows include them.

**Showing the rows in a grid.** `query.filter` is what the grid's
[`filter`](#the-pages-filter) attribute takes: the grid then shows the cell's rows, and
what the reader sorts and filters stays inside them.

```js
let cell;
pivot.addEventListener("opengrid-drill-down", ({ detail }) => {
  cell = pivot.shadowRoot.activeElement;           // the pressed button
  panel.hidden = false;                            // first: a grid measures its viewport
  const { filter } = detail.query;
  if (filter) grid.setAttribute("filter", JSON.stringify(filter));
  else grid.removeAttribute("filter");             // the grand total, nothing filtered
  heading.focus();                                 // tabindex="-1", names the cell
});
back.addEventListener("click", () => {
  panel.hidden = true;
  (cell?.isConnected ? cell : pivot.shadowRoot.querySelector('[part="drill-down"]'))?.focus();
});
```

A grid that has its provider from the start asks for every row before the first cell
is pressed; the demo gives it its provider with the first event. A pivot on
`createPivotProvider` answers pivots only: its grid takes a `createRestProvider` of the
same source.

**The focus** is on the pressed button when the event fires — the pivot gives it the
focus where a browser does not on a click — and the pivot draws nothing, so a page
that shows the rows without moving the focus leaves the reader where they were. A
page that moves it — to a heading over the rows, as above, since the rows may be far
from the cell in the tab order — remembers `pivot.shadowRoot.activeElement` in its
listener and focuses it again when the reader leaves the rows. A new answer makes
the buttons anew: the remembered one is then no longer connected, and the pivot's
first `[part="drill-down"]` is where to go back to. `examples/pivot-demo/browser.html`
does all of it.

## The view

Sort, filters, column layout and density are one value:

```js
const view = loader.module.get_view(grid);
// { sort: [{field, direction}], filters: [{column, op, value}],
//   columns: { order, hidden, widths, pinned }, density,
//   group: [], expanded: [], facets: {}, selected: [4, 9],
//   where: { or: [{ field: "country", op: "eq", value: "DE" },
//                 { field: "amount", op: "gte", value: 1000 }] } }
loader.module.set_view(grid, view);
```

**A saved view is this value with a name on it.** That is why the element has no
view management of its own: naming, storing, deleting, putting one in a URL are
the page's, the same line drawn for editing below. `opengrid-view-change` fires
whenever the reader changes any of it.

| | |
|---|---|
| Attributes vs. the value | `density`, `columns` and the rest set the **first** view; `set_view` leads after that — the relationship `value` has to `<input>`. |
| One query | Applying a view costs **one** query, not one per field. Field by field, a restore would flash through intermediate results and announce each of them. |
| Nothing partial | A view naming a column this grid does not have is reported in the status line and applied **not at all**. A grid that looks restored and is not is the worse failure. |
| The focus | Stays where it is. A page applies a view from its own control — a tab, a menu — and keeps the focus there; only a grid that had the focus gets it back, on its active cell. |
| Setting what it has | Costs nothing and says nothing — a page that writes the view back on every event must not make the grid talk to itself. |
| Why it changed | `set_view(host, view, { notice })`: the page's sentence — "Saved view Q3 applied", "Filtered by the assistant" — is said **once, with the result**, in the one live region: `17 matches · Filtered by the assistant`. A second live region of the page would be a second utterance. A view the element already has says nothing, notice or not. The same on `<opengrid-pivot>`. |
| `columns.pinned` | The reader's pins, `{ customer: "start", id: "none" }`: `start`, `end`, or `none` for a pin of `set_columns` taken away ([Pinned columns](#pinned-columns)). Leads over `set_columns`. A view saved without it leaves the pins to the page. |
| `group`, `expanded` | The grouping and its open groups. A path of keys that matches no group opens nothing. |
| `aggregates` | The reader's aggregate per column, `{ amount: "sum" }`. Leads over `set_columns`. |
| `filterRow` | Whether the filter row shows. `true` unless turned off. |
| `facets` | The facet selections, by column. |
| `selected` | The keys of the selected records, where the grid [selects by key](#selecting-by-key); left out while there are none. `set_view` restores them after asking the source which of them exist. |
| `where` | The [nested filter](#nested-filter); left out while there is none, and a view without it has none. |

**`where` is a query's `filter`, as [the wire](../protocol/) writes it** — the form
of the page's [`filter`](#the-pages-filter), so values are typed: a number is a
JSON number, a decimal and a date are strings.

```
where     = { "and": [where, …] } | { "or": [where, …] } | condition
condition = { "field", "op", "value" }      op: eq ne gt gte lt lte contains starts_with has
          | { "field", "op": "has_all", "value": [ … ] }
          | { "field", "op" }               op: is_null is_not_null is_empty
```

| | |
|---|---|
| What it takes | What its two editors can write: `and` and `or` with at least one member, and conditions. `not` and the operator `in` are refused — a page that needs them has the `filter` attribute. |
| Columns | Every `field` is one of the grid's `columns`, or the view is refused like any view naming an unknown column, and nothing of it is applied. |
| Values and operators | Are checked by the source, as the `filter` attribute's are: a value of the wrong type or an operator the column does not take is the provider's error, in the status line. |
| As written | `set_view` keeps the tree as the page wrote it — it does not flatten or reorder — so `get_view` gives it back. The builder tidies what it applies. |
| Next to `filters` | Both hold: `filters` is the filter row, `where` the nested filter, and-ed. |

**A selection by key is in the view; a selection by position is not.** Without a
`row-key` the grid names a row by its number in the result, and sorting or
filtering drops such a selection precisely because after a different sort those
positions hold different records: a restored view carrying positions would not
be incomplete — it would be wrong. Applying a view drops it, and says so. A key
names a record under any sort, so `selected` travels with the view — and
because a saved key can outlive its record, `set_view` asks the source before
it selects ([Selecting by key](#selecting-by-key)). Selecting is still not a
view change: it has its own event.

## Exporting the view

What a page exports is what the reader sees — and the grid knows that better than the page:
the filter row, the facets and the free-text search and-ed together, the sort, the shown
columns in their order. `get_query(host)` hands it out, exactly as the grid asks its provider,
but **without a window**: no `offset`, no `limit`, every match.

```js
const query = loader.module.get_query(grid);
// { source, select: ["id", "customer", …], filter: {…}, sort: [{ field, direction }, …] }
```

| | |
|---|---|
| Hidden columns | are not in `select`; moved ones are in their new place. |
| Grouped | The rows, not the group or total rows, ordered by their groups first (ascending, NULL last) and then by the sort — the order the reader sees. |
| The selection | is not in it. It names positions under exactly this query's sort, so an export of the selection is this query plus the positions from `opengrid-selection-change` — or, where the grid [selects by key](#selecting-by-key), plus its `keys`. |

**Without an element** (issue #144). The engine answers the query of a view it is
handed: `engine.view_query(source, columns, view)` — `columns` is the grid's
`columns` attribute as a JSON array, `view` a view as JSON — returns what
`get_query()` returns on that grid once the view is applied; the free-text
search, not being part of a view, is not in it. A server can count or export a
saved view this way, without a browser. A view naming a column the grid does not
have is refused as a whole, every problem named; so is a filter value that is not
a value of its column.

```js
const query = JSON.parse(engine.view_query("orders", JSON.stringify(["id", "country", "amount"]), JSON.stringify(view)));
const { total_count } = JSON.parse(engine.execute(JSON.stringify({ ...query, limit: 0 })));
```
| `null` | A grid without a query yet (not connected, no `datasource`, no columns), one whose filter does not hold — its status line says why — and `<opengrid-table>` and `<opengrid-pivot>`. |

The grid has no export button: what to export, in which format, under which name, is the
page's (the same line as for saving an edit). How a page uses what follows — a button, the
formula guard, reading a file back into opengrid, the errors to handle — is the
[Exporting](../guides/export/) guide.

### `exportRows(provider, query, options)`

Fetches every match of a query through any provider, in pieces, and answers a `Blob`:

```js
import { exportRows } from "@casoon/opengrid";

const controller = new AbortController();
const blob = await exportRows(provider, loader.module.get_query(grid), {
  format: "csv",                       // or "json", or "xlsx" — by the server, or in the tab
  signal: controller.signal,
  onProgress: ({ rows, total }) => { /* rows written so far, of total */ },
});
// The file name is the page's:
const link = Object.assign(document.createElement("a"), {
  href: URL.createObjectURL(blob),
  download: "orders.csv",
});
link.click();
// Revoked once the download has taken the URL, not in the same task.
setTimeout(() => URL.revokeObjectURL(link.href), 0);
```

| Option | |
|---|---|
| `format` | `"csv"` (default), `"json"` or `"xlsx"`. A workbook is written by the server where the provider exports there, and in the tab otherwise, by a module of its own that is loaded the first time ([XLSX](../guides/export/#xlsx)); it needs the rows to fit `maxRows` and an Excel sheet (1 048 575), and holds them in memory. |
| `chunkSize` | Rows per request; 10 000 by default — the server's `max_limit`. Not used when the provider exports by itself. |
| `maxRows` | 1 000 000 by default — 250 000 for an XLSX written in the tab, which holds the workbook in memory. More matches than that is an error with a sentence and the code `too_many_rows` ([Errors](#errors)), before anything else is fetched — never a truncated file. |
| `onProgress` | Called after each piece with `{ rows, total }`. |
| `signal` | An `AbortSignal`. An abort rejects with an `AbortError` at once, hands the signal to the provider so an HTTP request stops, and gives no `Blob`. |
| `delimiter`, `bom`, `protectFormulas`, `null` | **The CSV options**, each optional, for a CSV only — `get_pivot` takes the same: `delimiter`, one character, `,` by default (`;` for a German Excel); `bom`, a UTF-8 byte order mark, on; `protectFormulas`, the guard against formula injection, on; `null`, how NULL is written, empty (`\N` reads back into opengrid) — under the guard not starting with `=`, `+`, `-`, `@` or a tab, since it is written into every empty cell unguarded. |

Any other key is an error, and so is a CSV option on a JSON export.

| | |
|---|---|
| Values | Raw, in the wire notation — not the display formats: a decimal exact, a date `YYYY-MM-DD`, a timestamp ISO in UTC with microseconds, `NaN`/`Infinity`/`-Infinity` spelled out. The CSV header and the JSON keys are the field names, in column order. |
| CSV | RFC 4180, UTF-8 with a byte order mark, CRLF, `,` or `;`. NULL is an empty unquoted field, the empty string `""`. The formula guard applies to text columns only. `text/csv;charset=utf-8`. |
| JSON | One array of row objects. `application/json`. |
| Pieces | `offset`/`limit` windows of `chunkSize` under the query's sort. The total is the first piece's `total_count`; the pieces stop at it. |
| Order | A window is only stable under a total order, and the grid's sort may tie — against PostgreSQL a row could repeat or go missing between two pieces. So every selected column not yet in the sort is appended, ascending. The export is then deterministic, and rows equal in every selected column look the same whichever comes first. **Within a tie the export follows the columns, not the grid:** two rows the grid showed in one order may come in the other. One tie is left: `-0.0` and `0.0` compare equal, so a source that does not order them may swap them between two pieces. |
| A changing source | The tie-breaker fixes ties, not rows that come or go while the export runs — they shift the windows. So it is **detected and refused**: a piece whose `total_count` differs from the first one's, or that ends before the total, rejects the export with an error (code `source_changed`), and there is no `Blob`. A change that keeps the count and only moves a row is not visible to the export; for such a source, export from a snapshot. |
| `mode` | The provider is asked with the mode `""` — its own default. |
| `query` | Without `offset` and `limit`, with a `select`, without `group` and `aggregate` — the rows of a view. `get_query` gives exactly that; `null` from it is an error here. |

#### Over a server: one request

A provider with an `export(query, options)` method exports by itself, and `exportRows` lets
it: `createRestProvider` has one, `POST /export/{source}` of an `opengrid-server`. Then the
whole export is **one streamed request** instead of pieces — against PostgreSQL one statement
read through a cursor, instead of `OFFSET` windows that get dearer with every piece.

```js
const rest = createRestProvider({ url, source: "orders", token });
const blob = await exportRows(rest, loader.module.get_query(grid), { format: "csv" });
// or directly:
const same = await rest.export(query, { format: "csv", delimiter: ";" });
```

| | |
|---|---|
| The file | The same bytes the pieces would have made: `exportRows` checks the options the same way and sends the same query, tie-breaker included; the server writes it with the same `opengrid-export`. |
| Options | `format`, `signal`, `maxRows`, `onProgress` and the CSV options — `exportRows`' own, without `chunkSize`. Any other key is an error. |
| `maxRows` | The server sends the row count before the rows (`X-Total-Count`); more than `maxRows` is an error (`too_many_rows`) before the body is read, and so is an answer without a readable count. The server has its own bound, `max_export_rows` — more is its `413` (`limit_exceeded`), with a sentence, before the first byte; too many exports at once are its `503` (`busy`). |
| `onProgress` | Called once, at the end, with `{ rows, total }`. |
| `signal` | Aborts the request, the download included: an `AbortError`, no `Blob`. The server notices at its next piece and ends the database query. |
| Rules | The server's for `/query`, unchanged: the token, `allowed_fields`, the tenant's `row_filter`. |
| A break | A failure after the first byte cannot be a status any more; the server breaks the connection off, and the export rejects — never a shorter file. A client that takes no piece for the server's timeout is broken off too. |
| Redirects | Not followed, by any request of `createRestProvider` or `createPivotProvider`: the token goes to the configured URL and nowhere else. |

### Exporting a pivot

A pivot is exported by the element, as it is shown: `get_pivot(host, options)` answers the
table as CSV text, or `null` while nothing is shown.

```js
const csv = loader.module.get_pivot(pivot, { delimiter: ";" });
// "\uFEFFcountry;2025 · total;2026 · total;(no value) · total\r\n(empty);;114;\r\n…"
const blob = csv && new Blob([csv], { type: "text/csv;charset=utf-8" }); // the file name is the page's
```

| | |
|---|---|
| Columns | The row dimensions, then one column per generated column, in the table's order. |
| Header | **One line.** A generated column is named by its value and its measure, `2025 · total`; without a column dimension, by its measure. A [calculated measure](#calculated-measures) is followed by what it shows, in the element's texts: `2025 · share · share of row`. |
| Rows | Every row the table shows, in its order: data rows, subtotals, the grand total. |
| Subtotals | Their label — `Total DE`, `Total` — in the first dimension column; the dimension columns it spans are empty fields. There is no level column, as the table has none: a subtotal is known by its label alone, so a group literally named `Total` looks like the grand total. |
| Labels | The element's own [texts](#texts): NULL is `(no value)`, the empty string `(empty)`, as in the table, and a page's `set_texts` changes both. With non-empty texts a data row's dimension cell is never empty, so an empty one means "spanned by the total"; a text set to `""` gives that up. |
| Values | As every export writes them: the wire notation, NULL as the `null` option, the formula guard — which covers every header and label, since a dimension value is data. The notation is the canonical one, whatever the provider sent: a custom provider's float `2` reads `2.0`, a decimal `12.5` at scale 2 reads `12.50`, where the table shows the text as it came. |
| `options` | The CSV options — `delimiter`, `bom`, `protectFormulas`, `null` — as [`exportRows`](#exporting-the-view) takes them. Any other key is an error. |
| Errors | A wrong option, and an answer the export cannot read — one whose cells do not match its row dimensions and columns, as a page's own provider could send. Both throw with a sentence. |
| `null` | Before the first answer, while one loads, after an error, and for the grid and the table. |

**Why the element, not a query.** A pivot is bounded — 256 columns, 10 000 rows, 131 072 cells — and the
element holds all of it, so there is no window and nothing a second request could add. It could
only answer differently, if the data moved since the table was drawn, and it would need the
element's texts handed to it. So `get_query` stays `null` for a pivot, and `get_pivot` is
synchronous and needs no provider.

**Why one header line.** Every CSV reader — a spreadsheet's filter, pandas, a database's
`COPY` — takes the first line as the names and the second as data; a second header line would
arrive as a row of text in number columns. A CSV has no merged cells either, so a two-line header
would repeat each value over its measures anyway, or leave header cells empty — the silence the
element refuses. `2025 · total` is also what a screen reader announces for such a cell: the
group's header, then the column's.

#### `exportPivot(provider, host, options)`

The pivot as an XLSX file. A workbook is written by `opengrid-server`, so this is the one export
of a pivot that asks: the element's view goes to the server as a pivot query, and the answer is
the file.

```js
import { exportPivot } from "@casoon/opengrid";

const blob = await exportPivot(provider, pivot, { format: "xlsx" }); // a Blob; the file name is the page's
```

| | |
|---|---|
| `provider` | One that exports on the server: `createPivotProvider` or `createRestProvider`, both with `exportPivot(pivotQuery, options)` → `POST /pivot/{source}?format=xlsx`. Any other — the engine in the tab or a worker, the hybrid provider — is a `TypeError` before anything is sent. The method has a query and no element, so it alone also takes `texts` (`total`, `subtotal`, `noValue`, `emptyValue`, and for calculated measures `shareOfRow`, `shareOfColumn`, `shareOfTotal`, `runningTotal`); a page that calls it with a query of its own hands them over, or gets the English defaults. |
| `host` | An `<opengrid-pivot>` in the document, with a `datasource`. Its [view](#opengrid-pivot) is the query: `rows`, `columns`, `values`, `sort`, `filter`. Folded groups are exported, as in the CSV. |
| `format` | `"xlsx"`, and required; anything else is a `TypeError` — a pivot's CSV is `get_pivot`. |
| Labels | The element's own [texts](#texts), as in the table and in the CSV: `Total`, `Total DE`, `(no value)`, `(empty)`, or what `set_texts` made of them. They are read from the element and sent with the request; the page passes nothing, and `texts` is not an option here. |
| `signal` | Aborts the request: an `AbortError`, no `Blob`. |
| The sheet | The CSV's table: one header row with its names (`2025 · total`), the same columns and rows in the same order, a total's label in the first dimension column and the columns it spans empty. No merged cells, no outline levels. Named after the source, the header row frozen. |
| Cells | Typed as in [every workbook](../guides/export/#xlsx): a number, a decimal with its scale, a date, in a measure and in a dimension column alike. A NULL measure is an empty cell; a NULL or empty dimension value is its label, as text. A text is a string, never a formula. |
| Rules | The server's for `/pivot`, unchanged — the token, `allowed_fields`, the tenant's `row_filter`, the pivot's limits — and an export's: `max_xlsx_rows`, and `busy` when too many exports run. |
| The answer | The server's, now: if the data moved since the table was drawn, the file is the newer pivot. A server that does not write a pivot as XLSX is an `Error` saying so. |

### Printing the view

A grid holds a window of rows inside a scroller, so printing the page prints that window. For
paper — and for a PDF, which the browser's print dialog writes — the grid builds a plain table of
**every match**, and the page prints:

```js
const rows = await loader.module.prepare_print(grid);   // 200 — or null: nothing to prepare
window.print();
```

`prepare_print(host)` asks the provider for every match of the view — the query of
[`get_query`](#exporting-the-view), in one request — and builds a native `<table>` in the
grid's shadow root. Only `@media print` shows it, in place of the grid; on the screen nothing
changes, so the rows in view, the focus and the scroll position are what they were. The
`afterprint` event removes the table. It has to be asked for, because a browser does not wait
in `beforeprint` for a query to answer.

| | |
|---|---|
| The table | A `<caption>`, a `<thead>` with a `<th scope="col">` per column — a browser repeats it on every page — and one `<tr>` per match, none split over two pages. No ARIA grid roles: on paper it is a table. |
| Caption | The `label`, the number of matches and the view in the grid's own [texts](#texts), as its chips say them: `Orders · 42 matches · country is DE`. [The page's filter](#the-pages-filter) holds for the table as for every query; it has no chip, so the caption does not name it. |
| Columns | The shown columns in their order, under their `title`. The selection column, the action columns and every control are not printed. Nothing is [pinned](#pinned-columns) on paper. |
| Values | As the grid shows them, through `set_formats` — not the raw values of an export. They are the source's: an edit the page has not saved is not in them. |
| Colours | Black on white, with lines, whatever the look — a dark `theme` and the page's own tokens included. |
| Grouped | Every group open: a header row per group, `country: DE (12 rows)`, then its rows. Group and total summaries are not printed. |
| A [tree](#tree) | Whole and flat, every node open, the first column indented by depth. With a filter, a node that is only the way to a match says so after its name: `Sales (context)`. Subtree summaries are not printed. |
| `page-size` | Is not a page of paper: every match is printed, not the page the reader is on. |
| Resolves | With the number of rows in the table. With `null` — and nothing is built — where `get_query` answers `null`, and for `<opengrid-table>` and `<opengrid-pivot>`, which hold all their rows and print as they are. |
| Rejects | With the provider's error; and with an `Error` whose `code` is `too_many_rows` when the view has more than 10 000 matches, before a table is built — narrow the view, or [export](#exporting-the-view) it. A rejection leaves no table. |
| Lifetime | Until `afterprint`, or the next `prepare_print`. It is the view of the moment it was asked for: call it right before `window.print()`. |
| Without it | Printing the page prints the grid as the screen shows it — the rows in view, with its controls — in black on white. |
| Paper | Size, orientation and margins are the print dialog's and the page's `@page` rule; the grid has no option for them. |

`<opengrid-table>` and `<opengrid-pivot>` need no preparation: they are tables already. On paper
they are black on white as well, and the pivot's field toolbar is left out.

## Errors

A rejection of `createRestProvider` (`describe`, `execute`, `export`), of `createPivotProvider`
or of `exportRows` is a plain `Error`. Its message is a sentence for the developer — the
server's own, or the loader's — and a page does not parse it: to tell failures apart, the
`Error` carries fields.

| Field | |
|---|---|
| `status` | The HTTP status, when a server refused the request. |
| `code` | What went wrong, from the closed list below. Absent when nobody named it: a body that is not the server's error form (a proxy's `502` page), a server that did not say how many rows follow. |
| `path` | Where in the query, when the server knows: `select[1]`, `filter.and[1].value`. The message ends with it in parentheses, as it always has. |

```ts
import { exportRows, type CodedError } from "@casoon/opengrid";

try {
  blob = await exportRows(provider, query);
} catch (error) {
  if ((error as Error).name === "AbortError") return; // the reader cancelled
  switch ((error as CodedError).code) {
    case "busy": status.textContent = "The server is busy. Try again in a moment."; break;
    case "limit_exceeded":
    case "too_many_rows": status.textContent = "Too many rows. Narrow the view."; break;
    case "unauthorized": signIn(); break;
    // Also a code this loader does not know yet, from a newer server.
    default: status.textContent = "The export failed.";
  }
}
```

There is no error class: test `error.code`, not `instanceof`. In TypeScript the shape is
`CodedError` and the list `ErrorCode`. A wrong option is a `TypeError`, an abort a
`DOMException` named `"AbortError"` — test its `name`: its `code` is the DOM's legacy number
`20`, never one of the strings below. The hybrid provider hands its remote's
rejection on as it came. The tab and the worker reject with the engine's sentence and no fields —
the engine has no error form.

**The server's codes** come from its error form, `{ "error": { "code", "message", "path" } }`,
the same for `/query`, `/pivot`, `/source` and `/export`:

| `code` | Status | What happened | What a page does |
|---|---|---|---|
| `validation` | `422` | The query does not hold against the source's schema — a field outside `allowed_fields` is the same `unknown field` as a typo — or breaks a limit of its own, such as a `limit` above `max_limit` or a pivot too wide. | Nothing to tell the reader; a bug in the page. |
| `malformed` | `400` | The body or a parameter is not readable: not JSON, an unknown export parameter, a `delimiter` of two characters. | The same. |
| `unauthorized` | `401` | No token, or one the server does not accept. | Sign in again. |
| `unknown_source` | `404` | No source of that name. | A bug in the configuration. |
| `limit_exceeded` | `413` | Too big for the server: a body over `max_payload_bytes`, a query over the server's timeout, more rows than `max_export_rows`, an export without its first byte within the timeout. The same request usually fails again — a timeout may pass under less load. | Narrow the view. |
| `busy` | `503` | `max_concurrent_exports` exports are already running. The request itself is fine. | Try again in a moment. |
| `backend` | `502` | The source behind the server failed. | Try again later; the operator's log says why. |
| `data` | `422` | The data breaks a rule the query relies on — a tree with a key twice or a cycle. The message names the rows. Retrying does not help. | Show the message to whoever owns the data. A client that does not know the code treats it like any failure. |

**`exportRows`' own codes** never come from a server (`prepare_print` rejects with `too_many_rows` too, at its own bound):

| `code` | What happened | What a page does |
|---|---|---|
| `too_many_rows` | More matches than `maxRows` — the first piece's `total_count`, or the server's `X-Total-Count` before the body. Nothing more is fetched. | Narrow the view. |
| `source_changed` | A piece reported a different `total_count` than the first, or ended before the total. | Export again, or from a server, which reads one snapshot. |
| `module_not_loaded` | The WebAssembly module did not load, and the export notation is in it. | Say the export is not available. |

## Events

The events fire on the host, `bubbles` and `composed` (without `composed` they would
not leave a shadow root the page wrapped the element in), and none is
`cancelable` — they report what has already happened. `opengrid-source-replaced` is the
exception to the first part: `loader.js` dispatches it on the element, without `bubbles`.

| Event | `detail` |
|---|---|
| `opengrid-selection-change` | One of two shapes, by what the selection names ([Selecting by key](#selecting-by-key)). **By position** (no `row-key`, no tree): `{ rows: number[], ranges: { from: number, to: number }[], count: number }` — logical row numbers, ascending, and the same rows as `ranges`: runs of adjacent rows, ascending, both ends inclusive (select-all over 100 000 rows is one range, `{ from: 0, to: 99999 }`). **By key** (`row-key`, or a [tree](#tree)): `{ count: number, keys: unknown[] }` — the keys of the selected records, and neither `rows` nor `ranges`. `"keys" in detail` tells them apart. |
| `opengrid-bulk-change` | `{ keys, column, value }` — one value was set for the selected rows; see [Setting a value for the selected rows](#setting-a-value-for-the-selected-rows). |
| `opengrid-record-open` | `{ key, row }` — **cancelable**: the reader asked for the record as a form; `preventDefault()` keeps the grid from opening its own. See [Editing a whole record](#editing-a-whole-record). |
| `opengrid-paste` | `{ changes: [{ key?, row, column, value, previous }], added: [{ values, provisionalKey }] }` — a pasted block was applied; see [Pasting](#pasting). |
| `opengrid-row-add` | `{ values, provisionalKey, undo?, redo? }` — a record was entered in the new row; see [Adding rows](#adding-rows). |
| `opengrid-row-action` | `{ id, row, key? }` — an action was activated; `key` is the `row-key` value. |
| `opengrid-cell-change` | `{ row, column, value, previous }` — everything needed to persist it. With `row-key` also `key`: the record's value of that field, `null` when the result does not carry it. |
| `opengrid-drill-down` | `{ query: { source, filter? }, row, column, measure, value }` — a value of an `<opengrid-pivot>` with `drill-down` was pressed; `query` is the rows behind the cell, as the wire writes a query. See [Drilling down](#drilling-down). |
| `opengrid-view-change` | `{ view }` — the whole [view](#the-view) after the change. Scrolling and selecting are not view changes. On `<opengrid-pivot>` the view is its `rows`, `columns`, `values`, `sort`, `collapsed` and `filter`. |
| `opengrid-source-replaced` | `{ name }` — an engine provider replaced the source this element shows; the grid, table and pivot re-run their query, the grid clears its selection and history and asks a tree and the facets again. Dispatched by `loader.js` on every element on that `datasource`, not bubbling; `connect` hands it to `onSourceReplaced`. |
| `opengrid-query` | `{ kind, ms, rows, total, bytes, form, memory }` after **every** answer of the provider, on the grid and the pivot (issue #70): where it ran (the provider's `kind`), the round trip in milliseconds measured in the tab, rows answered and matches before paging, the answer's size as it arrived and its form (`binary` or `json`), and the element module's WASM memory. Measured always, sent nowhere — what the page does with it is the page's. |

**The component edits; the page saves.** There is no write path: the engine's
contract is a query. An edited value is shown at once and marked unsaved; a
fresh result from the source clears the marks.

### Saving edits

A page that saves each edit on its own says back what became of it (issue #153).
`row-key` names the field that identifies a record; `opengrid-cell-change` then
carries its value as `key`, which stays right while the reader sorts, filters or
the grid reloads — `row` does not.

```js
grid.setAttribute("row-key", "id");
grid.addEventListener("opengrid-cell-change", async ({ detail }) => {
  const { key, column, value } = detail;
  loader.module.set_cell_state(grid, key, column, "saving");
  try {
    await save(key, column, value);
    loader.module.set_cell_state(grid, key, column, "saved");
  } catch (error) {
    loader.module.set_cell_state(grid, key, column, "error", `Grade for ${key} not saved: ${error.message}`);
  }
});
```

| `state` | What the grid does |
|---|---|
| `saving` | The cell is `part="cell cell-saving"` (the muted ink by default). |
| `saved` | The unsaved mark goes; the cell is `part="cell cell-saved"`. |
| `error` | The value and the unsaved mark stay; the cell is `part="cell cell-error"` with `aria-invalid="true"`, and `message` is said in the grid's one live region (polite). |

A page styles the states through those parts — `opengrid-grid::part(cell-error) { … }`.
Inside the shadow root the cell also carries `data-state`.

`set_values(grid, [{ key, column, value }])` shows values the page computes — a
running average, a proposed grade. They are not edits: no unsaved mark, no
`opengrid-cell-change`. Such a column is `readonly` in `set_columns`, so it never
opens an editor and is announced as read-only.

Both name **records**: a state or a value stays with its record through sorting,
filtering and every result that follows, until the page says something else.
Another `row-key` or another `datasource` forgets them. A fresh result still
replaces an edited value with the source's, as above — the state stays. Neither
call moves the focus. Without `row-key`, or for a key that is not the field's
type, nothing happens. A walk-through is in the guide
[Saving edits from the page](../guides/saving-edits/).

Sorting and filtering **drop the selection**, and say so. That is not a
preference: a selection names positions, the grid has no key column, and after a
different sort those positions hold different records. Where the selection names
keys instead — with a `row-key`, and in a [tree](#tree) — it stays
([Selecting by key](#selecting-by-key)).

## Styling

Shadow DOM, so the page reaches in through parts and custom properties.

**Custom properties.** A page sets these; the grid computes the accented ones
from them, so picking one accent is enough. Set them **on the element** (`opengrid-grid { … }`),
not only on an ancestor: the grid declares every default on `:host`, and a
declaration on the element wins over an inherited value.

| Set | Default | What it paints |
|---|---|---|
| `--og-font` / `--og-font-size` | `"Geist", system-ui, sans-serif` / `0.875rem` | everything the grid writes |
| `--og-font-mono` | `"Geist Mono", ui-monospace, …` | the values of a column marked `mono` |
| `--og-surface` | `#ffffff` | rows, the body of the grid |
| `--og-surface-2` | `#fafbfc` | header, filter row, pager, the facet sidebar, the search field |
| `--og-ink` | `#14161a` | the text |
| `--og-ink-muted` | `#646b78` | text that is there but not the point |
| `--og-line` | `#eceef2` | the rules between rows |
| `--og-line-strong` | `#e0e3e9` | the rules between regions |
| `--og-accent` | `#3d5fd6` | the one colour a page picks — drawn as text too, so a colour meant for text |
| `--og-on-accent` | `#ffffff` | text drawn on the accent (the tick of a checked box) |
| `--og-radius` | `12px` | the corners of the grid's boxes |
| `--og-pad` | `8px` | horizontal padding inside a cell |
| `--og-focus-width` | `2px` | the focus ring |
| `--og-row-height` | `42px` | a data row; **goes into the window math**, a change at runtime is followed, a fractional length is rounded up |
| `--og-header-height` | `--og-row-height` | the header row |
| `--og-filter-height` | `40px` | the filter row |
| `--og-status-height` | `24px` | the footer with the status line, as a minimum |

| Computed | From |
|---|---|
| `--og-accent-soft` | 13 % accent on surface — a pressed switch, a chip |
| `--og-accent-ink` | 80 % accent on ink — the accent as readable text |
| `--og-selected` | 9 % accent on surface — a selected row |
| `--og-hover` | 4 % ink on surface — a hovered row |

**Density** sets three of these at once:

| `density` | `--og-row-height` | `--og-pad` | `--og-font-size` |
|---|---|---|---|
| `compact` | `34px` | `10px` | `0.8125rem` |
| `normal` (default) | `42px` | `12px` | `0.875rem` |
| `comfortable` | `50px` | `16px` | `0.875rem` |

The row height is **pixels** and the font size is **`rem`**, and that is not an
oversight. The row height is the virtualization contract: the element parses
`<number>px` out of the property, because an unregistered custom property is not
resolved for `getComputedStyle`. The font size has no such reader, so it can be
relative — and it should be, or a reader who raised their browser's font size
would be overruled. The consequence a page has to know: **raising the font size
means raising `--og-row-height` with it.**

**The defaults are the look called Base** — the one of the design prototype — so a
grid, table or pivot with no page CSS already looks finished. **`theme`** picks
another built-in look; the defaults in the table above are Base's:

| `theme` | Font | Surface | Ink | Accent | Radius | |
|---|---|---|---|---|---|---|
| `base` (default) | Geist | `#ffffff` | `#14161a` | `#3d5fd6` | `12px` | light |
| `paper` | IBM Plex Sans | `#fffdf8` | `#1f1d19` | `#2a7a59` | `8px` | light |
| `violet` | Geist | `#ffffff` | `#17161f` | `#6b4bc8` | `14px` | light |
| `orange` | Geist | `#ffffff` | `#111111` | `#b44c1c` | `10px` | light |
| `dark` | Geist | `#15181c` | `#e6e8eb` | `#4fd1d1` | `12px` | dark |

A look sets every colour token, the fonts, the radius and `color-scheme`. What a
page sets on the element wins over it, so a look is where a page's own starts:
`<opengrid-grid theme="dark" style="--og-accent: #f0b429">`. A look does not follow
the reader's light or dark preference by itself — a page that does sets `theme`.

The fonts are **named, not loaded**: the element makes no request to a font
service. A page that wants Geist or IBM Plex Sans loads it; without it the stack
falls back to the system UI font.

`<opengrid-table>` and `<opengrid-pivot>` take the same `theme` and tokens, apply the
ink and the font to their text, and are styled beyond that through their parts.

Under `forced-colors` every colour here resolves to a system colour, whatever the
look: a `color-mix` of two system colours resolves unpredictably, and the user's
palette is the one that has to win.

Three things a theme cannot switch off, because they are accessibility rather
than decoration: the **focus ring** never uses `--og-accent` (a pale accent
would make it invisible), a **selected row** carries an inset accent bar as well
as the tint (colour alone would be 1.4.1), and `prefers-reduced-motion` beats a
theme that animates a part.

**Parts:** `action`, `action-cell`, `action-header`, `add-field`, `add-filter`, `add-grouping`, `band`, `body`, `bulk`, `bulk-apply`, `bulk-cancel`, `caption`, `cell`, `cell-error`, `cell-saved`, `cell-saving`, `chip`, `chip-calc`, `chip-edit`, `chip-move`, `chip-remove`, `chips`,
`chips-clear`, `column-menu`, `column-menu-button`, `column-toggle`, `columns`, `columns-toggle`, `density`, `drill-down`,
`editor`, `empty`, `empty-reset`, `empty-text`, `facet`, `facet-bounds`, `facet-cost`, `facet-count`,
`facet-pill`, `facet-pills`, `facet-value`, `facets`, `facets-head`, `facets-toggle`, `filter`,
`filter-builder`, `filter-builder-dialog`, `filter-clear`, `filter-dialog`, `filter-operator`, `filter-row-toggle`, `filter-value`, `footer`,
`field-group`, `field-menu`, `group-mark`, `group-toggle`, `grouping-menu`, `header`, `layout`, `list`, `list-item`, `list-item-remove`, `listbox`, `menu-label`, `operator-menu`, `page-first`, `page-label`, `page-last`, `page-next`,
`page-previous`, `pager`, `paste-apply`, `paste-discard`, `paste-preview`, `paste-summary`, `record-dialog`, `row`, `row-header`, `search`, `search-hint`, `search-input`, `search-list`,
`select`, `select-all`, `select-mark`, `sort-button`, `sort-direction`, `sort-index`, `source`, `status`,
`table`, `toolbar`, `total-row`, `validation`, `viewport`.

`<opengrid-table>` and `<opengrid-pivot>` ship no stylesheet beyond the look's
ink and font and the alignment of a column (above) — they are plain tables, and the
rest of their look is the page's, through their parts (issue #29):

| Part | `<opengrid-table>` | `<opengrid-pivot>` |
|---|---|---|
| `table`, `caption` | the table and its caption | the same |
| `header` | a column header cell | a column header cell, both header rows |
| `sort-button`, `sort-direction` | the button in a header, its mark | the same, in a row-dimension header, and in a measure header without a column dimension |
| `row-header` | — | a row's header cell, the total rows' included |
| `group-toggle`, `group-mark` | — | a group's fold button in its subtotal's row header, and its mark |
| `toolbar`, `field-group`, `menu-label` | — | the field toolbar, one axis's group, its visible name |
| `chip`, `chip-move`, `chip-remove` | — | a chosen field, its move buttons, its remove button |
| `chip-calc` | — | a measure chip's button for what the measure is shown as |
| `add-field`, `field-menu` | — | an axis's add button, and its menu |
| `row`, `total-row` | a body row | a body row; `total-row` a subtotal or the grand total |
| `cell` | a value | a value |
| `drill-down` | — | a value's button, with `drill-down` |

```css
opengrid-table::part(cell) { padding: 0.25rem 0.5rem; border-bottom: 1px solid #ddd; }
opengrid-pivot::part(total-row) { font-weight: 600; }
```

`<opengrid-table>` shows its values **as the grid does**: `set_formats` applies to its
cells, and `set_columns` gives a column its `title` and its `align`, `mono`, `emphasis`
and `muted`. The grid's own options — `width`, `aggregate`, `facet` — are refused for a
table, in the same alert as any other mistake, not ignored.

## Right to left

The three elements take the direction from the platform: `dir="rtl"` on the element or on any
ancestor, usually on `<html>`. There is no attribute of their own for it.

```html
<html lang="ar" dir="rtl">
  …
  <opengrid-grid label="…" datasource="orders" columns="id,customer,amount"></opengrid-grid>
```

Right to left, the first column stands on the right and the row reads towards the left; the
selection column, a group's or a node's indent and mark, the bar of a selected row, the sort mark
and the column menu's button, the toolbar, the facet sidebar and the popups all follow. A column
with `align: "start"` is right-aligned and one with `align: "end"` — every number — left-aligned.
A grid wider than its columns' room scrolls from the right edge, and a column
[pinned](#pinned-columns) at the start stays at the right edge. In `<opengrid-table>` and
`<opengrid-pivot>` the columns, the alignment, a group's fold mark and the field toolbar follow
the same way; neither has keys of its own that name a side.

**Keys in `<opengrid-grid>`.** `←` and `→` follow what is on the screen, as the WAI-ARIA grid and
treegrid patterns ask; every other key names a place in reading order and is the same in both
directions.

| Keys | On | Left to right | Right to left |
|---|---|---|---|
| `←` | cell | the cell before | the next cell |
| `→` | cell | the next cell | the cell before |
| `Ctrl`/`Cmd`+`←` | header cell | move the column one place towards the start | one place towards the end |
| `Ctrl`/`Cmd`+`→` | header cell | move the column one place towards the end | one place towards the start |
| `Ctrl`/`Cmd`+`Shift`+`←` | header cell | narrow the column | widen the column |
| `Ctrl`/`Cmd`+`Shift`+`→` | header cell | widen the column | narrow the column |
| `Alt`+`Shift`+`←` | header cell | pin one step towards the start | one step towards the end |
| `Alt`+`Shift`+`→` | header cell | pin one step towards the end | one step towards the start |
| `←` / `→` | band cell | the cell before / the next cell of the first header row ([Column bands](#column-bands)) | the next cell / the cell before |
| `Ctrl`/`Cmd`+`←` / `→` | band cell | move the band one place towards the start / the end | towards the end / the start |
| `←` | a group's or node's first cell | close; on a closed node or a leaf, go to the parent | open; on an open node, go to its first child |
| `→` | a group's or node's first cell | open; on an open node, go to its first child | close; on a closed node or a leaf, go to the parent |
| `Home` / `End` | cell | the first / last cell of the row | the same: the first cell is the one on the right |
| `Ctrl`+`Home` / `Ctrl`+`End` | cell | the first cell / the last cell of the last row | the same |
| `↑` `↓` `PageUp` `PageDown` `Tab`, and every other key | | | the same |

**What is supported.**

- The direction is read from the computed `direction` whenever it is needed, so a `dir` that
  changes while the element is in the page is followed: the layout at once, the keys with the
  next key, a popup the next time it opens, the filter row's place under the columns the next
  time the grid lays it out (a resize, a column shown or resized).
- Set the direction with `dir`, not with the CSS property `direction` alone: the marks that
  point sideways and the alignment are mirrored with `:dir(rtl)`, which reads the attribute.
- The texts are not translated by the direction; they are the page's ([Texts](#texts)). The
  default English ones in a right-to-left line are ordered by the browser, like any Latin text.
- The order of letters inside a cell is the browser's (the Unicode bidirectional algorithm);
  the elements add nothing to it.

## Texts

Everything the components write themselves is English and overridable. A `lang`
given along with the texts is written onto the elements that carry **those
texts** — never onto the data, which is the page's, in the page's language.
A group that holds the page's words — the toolbar, the column list, the filter
row, the facets, the search field and its suggestions — is named by
`aria-labelledby` pointing at a hidden element that carries the `lang`, so the
name is ours and the column names inside stay the page's.

```js
loader.module.set_texts(host, { lang: "de", loading: "Wird geladen …" });
```

**Translations.** The package ships German as a module of its own,
`@casoon/opengrid/texts/de` — every key below with `lang: "de"`, for all three
elements. A page that does not import it does not load it.

```js
import de from "@casoon/opengrid/texts/de";

loader.module.set_texts(host, de);
// With the page's own words on top — here an example that names its columns:
loader.module.set_texts(host, { ...de, searchPlaceholder: "Suchen oder filtern: kunde = Krause" });
```

The same object is the `texts` of [`connect`](#connecting) and of the framework
components. What a translation does not reach is what is not the component's:
column names and cell values are the page's, and so is how a number, a date or a
boolean reads — that is `set_formats`.

| Key | Default | Placeholders |
|---|---|---|
| `lang` | — | |
| `loading` | `Loading …` | |
| `matchesOne` / `matchesOther` | `{count} match` / `{count} matches` | `{count}` |
| `empty` | `No matches` | |
| `error` / `errorUnknown` | `The data could not be loaded: {cause}` / `The data could not be loaded.` | `{cause}` |
| `filterGroup` | `Filter` | |
| `operatorLabel` / `valueLabel` | `{column} operator` / `{column} value` | `{column}` |
| `clear` | `Clear` | |
| `selectAll` | `Select all matching rows` | |
| `selectedAll` | `{count} rows selected` | `{count}` |
| `groupRow` | `{column}: {value} ({rows})` | `{column}`, `{value}`, `{rows}` |
| `rowsOne` / `rowsOther` | `1 row` / `{count} rows` | `{count}` |
| `groupExpanded` / `groupCollapsed` | `{group} expanded, {rows}` / `{group} collapsed` | `{group}`, `{rows}` |
| `groupInvalid` | `Cannot group by {column}` | `{column}` |
| `totalRow` | `Total ({rows})` | `{rows}` |
| `aggregateCell` | `{aggregate}: {value}` | `{aggregate}`, `{value}` |
| `aggregateSum` / `aggregateAvg` / `aggregateCount` / `aggregateMin` / `aggregateMax` / `aggregateRange` | `Sum` / `Average` / `Count` / `Minimum` / `Maximum` / `Range` | |
| `columnMenu` | `{column} column menu` | `{column}` |
| `sortAscending` / `sortDescending` | `Sort ascending` / `Sort descending` | |
| `sortKey` | `Sort key {position}` — a further sort key's place (1-based), said by its column header through `aria-describedby`; the primary key carries `aria-sort` | `{position}` |
| `filterColumn` | `Filter …` | |
| `aggregateGroup` / `aggregateNone` | `Aggregate in groups` / `No aggregate` | |
| `groupByColumn` / `groupSecondLevel` / `ungroupColumn` | `Group by this column` / `Group as second level` / `Remove this grouping` | |
| `hideColumn` | `Hide column` | |
| `pinGroup` / `pinNone` / `pinStart` / `pinEnd` | `Pin` / `Not pinned` / `At the start` / `At the end` — the column menu's group and its three choices | |
| `toolbarGroup` / `filterRowToggle` | `Grid tools` / `Filter row` | |
| `densityGroup` / `densityCompact` / `densityNormal` / `densityComfortable` | `Density` / `Compact` / `Normal` / `Comfortable` | |
| `chipsGroup` / `chipsClear` | `Active filters` / `Remove all` | |
| `chipRemove` / `filterRemoved` | `Remove {filter}` / `{filter} removed` | `{filter}` |
| `filtersCleared` | `All filters removed` | |
| `groupChip` | `Grouped by {columns}` | `{columns}` |
| `facetsGroup` / `facetsToggle` / `facetsReset` | `Facets` / `Facets` / `Reset facets` | |
| `facetFrom` / `facetTo` | `From` / `To` | |
| `facetQueries` | `Counted with {count} queries` | `{count}` |
| `facetChipValues` | `{column} is one of {values}` | `{column}`, `{values}` |
| `addFilter` / `addFilterTitle` | `+ Filter` / `Add filter` — the toolbar's button and the dialog's title | |
| `filterColumnLabel` / `filterConditionLabel` / `filterValueLabel` | `Column` / `Condition` / `Value` | |
| `cancel` / `apply` | `Cancel` / `Apply` | |
| `addGrouping` / `groupingFull` | `+ Group` / `Grouped by three columns already` — the button, and the reason that follows its text in its name when no level can be added (`+ Group. Grouped by three columns already`) | |
| `pivotToolbar` / `pivotRows` / `pivotColumns` / `pivotMeasures` | `Pivot fields` / `Rows` / `Columns` / `Measures` — the pivot's field toolbar and its three groups | |
| `addRow` / `addColumn` / `addMeasure` | `+ Row` / `+ Column` / `+ Measure` | |
| `columnFull` / `nothingToAdd` | `Two column fields already` / `Nothing left to add` — the reason that follows an add button's text in its name when it cannot add (`+ Row. Nothing left to add`) | |
| `fieldRemove` / `fieldEarlier` / `fieldLater` | `Remove {field}` / `Move {field} earlier` / `Move {field} later` | `{field}` |
| `pivotFilters` / `pivotFilterLabel` / `pivotFilterHint` | `Filters` / `Add a filter` / `field = value · Enter` — the pivot's filter group, its field's name and hint | |
| `pivotDrillDown` | `{value}, show rows` — the name of a value's button in a pivot with `drill-down`: "12, show rows". `{value}` is the cell's visible text, so the name contains it (WCAG 2.5.3) | `{value}` |
| `pivotShowAs` | `Show {field} as: {calc}` — the name of a measure chip's button in the pivot's toolbar, and of its menu: "Show Quantity as: share of row". `{calc}` is the button's visible text, so the name contains it (WCAG 2.5.3) | `{field}`, `{calc}` |
| `calcValue` | `value` — a measure shown as it is, in that button and its menu | |
| `shareOfRow` / `shareOfColumn` / `shareOfTotal` / `runningTotal` | `share of row` / `share of column` / `share of total` / `running total` — what a [calculated measure](#calculated-measures) is shown as: in the button and its menu, after the measure's title in its column header, and in the headers of the CSV and the XLSX | |
| `treeContext` | `context` — a tree's row shown only because a match is below it | |
| `subtreeCell` | `{shown}, {aggregate} of the subtree` — what a node's cell says when it also shows its subtree's summary: "120 · Σ 1,450, Sum of the subtree". `{shown}` is the cell's visible text, so the name contains it (WCAG 2.5.3); German: `{shown}, {aggregate} im Teilbaum`. A text you set **without** `{shown}` is used exactly as written, and its name then does not contain the visible text — your decision. | `{shown}`, `{value}`, `{aggregate}`, `{total}` |
| `subtreeOnly` | `{shown}, {aggregate} of the subtree` — the same for a node without a value of its own: "Σ 1,450, Sum of the subtree" | `{shown}`, `{aggregate}`, `{total}` |
| `treeOrphans` | `Without a parent, shown at the top: {count}` | `{count}` |
| `treeLevelTooLarge` | `{count} rows at one level; a level holds at most {max} — choose a filter` | `{count}`, `{max}` |
| `treeRefused` / `treeGroupIgnored` | `no tree with page-size: a tree scrolls` / `group-by is not used in a tree` | |
| `filterAny` / `booleanTrue` / `booleanFalse` | `any` / `yes` / `no` — the filter row's choice for a boolean column | |
| `sourceLocal` / `sourceWorker` / `sourceRemote` / `sourceHybrid` | `local · wasm` / `worker · wasm` / `server` / `hybrid` — the footer's source, by the provider's `kind` | |
| `queriesOne` / `queriesOther` | `{count} query` / `{count} queries` — the footer's count | `{count}` |
| `searchLabel` / `searchPlaceholder` | `Search or filter` / `Search, or filter: country = DE and amount ≥ 10` | |
| `queryAnd` / `queryOr` | `and` / `or` — the joining words of the search expression and of the nested filter's chip | |
| `searchHint` / `searchSuggestions` | `Query · Enter` / `Columns` | |
| `typeText` / `typeBool` / `typeInteger` / `typeNumber` / `typeDate` / `typeTime` | `text` / `yes/no` / `integer` / `number` / `date` / `time` — the type beside a suggested column | |
| `searchChip` | `Text contains “{text}”` | `{text}` |
| `queryUnknownColumn` / `queryMissingValue` | `{column} is not a column of this grid` / `{column}: the value is missing` | `{column}` |
| `queryWrongOperator` | `{column} does not take {operator}` | `{column}`, `{operator}` |
| `queryUnclosed` / `queryUnopened` | `A parenthesis is not closed` / `A closing parenthesis has no opening one` | |
| `queryMissingCondition` | `A condition is missing` | |
| `queryMissingJoin` | `“{and}” or “{or}” is missing before “{text}”` | `{and}`, `{or}`, `{text}` |
| `filterBuilder` | `Advanced filter` — the toolbar's button and the builder's title | |
| `filterBuilderJoin` / `filterBuilderAll` / `filterBuilderAny` | `Rows match` / `all conditions` / `any condition` — a group's choice and its two options | |
| `filterBuilderCondition` / `filterBuilderGroup` | `Condition {number}` / `Group {number}` — the name of a condition and of a group inside a group; with `chipRemove`, the name of its remove button | `{number}` |
| `filterBuilderAddCondition` / `filterBuilderAddGroup` | `+ Condition` / `+ Group` | |
| `filterBuilderEdit` | `Edit {filter}` — the name of the nested filter's chip button | `{filter}` |
| `filterApplied` | `Filter set: {filter}` — said with the result when the nested filter was set | `{filter}` |
| `emptyFiltered` / `emptySource` | `No row matches these filters.` / `There are no rows.` | |
| `emptyReset` | `Reset filters` | |
| `operators` | the thirteen filter operators, keyed by wire token (`has`, `has_all` and `is_empty` are the list's) | |
| `choicesOne` / `choicesOther` | `1 choice` / `{count} choices` — said while typing in a combobox | `{count}` |
| `choiceRefused` | `“{value}” is not one of the choices` | `{value}` |
| `listItemAdded` / `listItemRemoved` / `listItemRemove` | `{value} added` / `{value} removed` / `Remove {value}` — the chip's button | `{value}` |
| `typeList` | `list` — the type beside a suggested column | |
| `filterInvalid` | `{column}: {value} is not a value for this column` | `{column}`, `{value}` |
| `cellRequired` | `{column} needs a value` | `{column}` |
| `validationWarning` / `validationError` | `Warning: {message}` / `Error: {message}` | `{message}` |
| `undoCell` / `redoCell` | `Undone: {column} in row {row}` / `Redone: …` | `{column}`, `{row}` |
| `undoAdded` / `redoAdded` | `Undone: row added` / `Redone: row added` | |
| `undoPaste` / `redoPaste` | `Undone: paste, {count} values` / `Redone: …` | `{count}` |
| `undoRecord` / `redoRecord` | `Undone: record {row}, {count} fields` / `Redone: …` | `{row}`, `{count}` |
| `recordTitle` / `recordSave` / `recordDone` / `recordNothing` | `Edit record {row}` / `Save` / `Record {row} saved, {count} fields changed` / `Record {row}: no field changed` | `{row}`, `{count}` |
| `rangeSize` / `rangeCopied` / `rangeUnloaded` | `{rows} by {cols} cells` / `Copied {rows} by {cols} cells` / `Not copied: the range reaches rows that are not loaded` | `{rows}`, `{cols}` |
| `undoNothing` / `redoNothing` | `Nothing to undo` / `Nothing to redo` | |
| `bulkCaption` / `bulkApply` / `bulkMenu` | `Set {column} for {rows} selected` / `Set` / `Set for selected rows` | `{column}`, `{rows}` |
| `bulkReadonly` / `bulkDone` | `{column} is read-only` / `{column} set for {rows}` | `{column}`, `{rows}` |
| `pastePreview` / `pasteApply` / `pasteDiscard` | `Paste preview` / `Paste` / `Discard` | |
| `pasteUnreadable` | `“{value}” cannot be read as {type}` | `{value}`, `{type}` |
| `pasteBadOne` / `pasteBadOther` | `1 value not readable` / `{count} values not readable` | `{count}` |
| `pasteCutOff` / `pasteApplied` | `{count} values beyond the end are not pasted` / `{count} values pasted` | `{count}` |
| `pasteDiscarded` / `pasteBlocked` | `Paste discarded` / `Not pasted: {problems}. Discard it and paste again.` | `{problems}` |
| `newRow` / `rowAdded` | `New row` / `Row added, {count} not saved yet` — the empty row of `add-row`, and what is said once a row is added | `{count}` |
| `selectionCleared` | `Selection cleared` | |
| `selectionMissing` | `{count} selected rows no longer exist` | `{count}` |
| `columnWidth` | `{column} is {width} pixels wide` | `{column}`, `{width}` |
| `columnMoved` | `{column} moved to position {position} of {count}` | `{column}`, `{position}`, `{count}` |
| `columnAtEdge` | `{column} is already at the end` | `{column}` |
| `columnPinnedStart` / `columnPinnedEnd` / `columnUnpinned` | `{column} pinned at the start` / `{column} pinned at the end` / `{column} no longer pinned` | `{column}` |
| `pinsReleased` | `Too narrow for the pinned columns: they scroll with the others` — said once when the pins let go | |
| `columnHidden` / `columnShown` | `{column} hidden, {visible} of {count} columns shown` | `{column}`, `{visible}`, `{count}` |
| `columnsGroup` | `Columns` | |
| `pageFirst` / `pagePrevious` / `pageNext` / `pageLast` | `First page` / `Previous page` / `Next page` / `Last page` | |
| `pageOf` | `Page {page} of {pages}` | `{page}`, `{pages}` |
| `total` / `subtotal` | `Total` / `Total {value}` | `{value}` |
| `noValue` / `emptyValue` | `(no value)` / `(empty)` | |
