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

- [Loading](#loading) · [Providers](#providers)
- [`<opengrid-grid>`](#opengrid-grid) · [`<opengrid-table>`](#opengrid-table) · [`<opengrid-pivot>`](#opengrid-pivot)
- [Events](#events) · [Styling](#styling) · [Texts](#texts)

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
| `set_provider(host, provider)` | Attaches the data source. `provider.execute(queryJson, mode)` answers the result JSON, or a Promise of it. Everything else is optional. |
| `set_texts(host, texts)` | Overrides any subset of the [texts](#texts). Call it **before** `set_provider` and the component renders the right words from its first paint. |
| `set_formats(host, formats)` | Per-column display formatting — see [`<opengrid-grid>`](#opengrid-grid). |
| `set_choices(host, choices)` | Per-column editor choices: `{ customer: ["Alpha", "Beta"] }` turns that column's editor into a `<select>`. |

**A provider is a seam, not a class.** Anything with an `execute` method fits,
which is how the engine can sit in the tab, in a worker, behind HTTP, or be
split across two of them without the elements knowing.

## Providers

All from `loader.js`, all the same shape:

| | |
|---|---|
| `createLocalProvider(engine)` | The engine on the main thread. |
| `createWorkerProvider({ moduleUrl, wasmUrl })` | The engine in a module worker; started lazily, once. |
| `createRestProvider({ url, source, token })` | `POST /query/{source}` of an `opengrid-server`. Also offers `describe()` → `{ name, schema, capabilities, pivot_limits }`. |
| `createHybridProvider({ remote, planner, mode, onPlan })` | Splits each query between a remote source and the engine in the tab. `onPlan` receives the plan before anything is sent. |
| `createPivotProvider({ url, source, token })` | `POST /pivot/{source}` — a whole pivot in one request. |

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

**Keyboard.** The WAI-ARIA grid pattern — arrows, `Home`/`End`, `Ctrl`+`Home`/`End`,
`PageUp`/`PageDown`, and:

| Keys | On | Does |
|---|---|---|
| `Enter` / `Space` | header cell | sort (`Shift` adds a second key) |
| `Enter` | data cell | open the editor (`Enter` commits, `Escape` discards) |
| `Space` | data cell | select the row (`Shift` extends from the last one) |
| `Ctrl`/`Cmd`+`A` | anywhere | select every matching row, not only the loaded page |
| `Ctrl`/`Cmd`+`←`/`→` | header cell | move the column |
| `Ctrl`/`Cmd`+`Shift`+`←`/`→` | header cell | resize the column |

Column visibility is a disclosure in the filter row: a checkbox per column,
including the hidden ones — otherwise there would be no way back.

**Formatting** is display only and never reaches a query:

```js
loader.module.set_formats(host, {
  amount: { kind: "number", locale: "de-DE", style: "currency", currency: "EUR" },
  ordered_on: { kind: "date", locale: "de-DE" },
  qty: (text, value) => `${value} pcs`,
});
```

A column without a format is rendered in Rust and never crosses the WASM/JS
boundary. `Intl` takes a JavaScript number, so a decimal beyond 2^53 loses
digits through the options form — such a column takes a function, which receives
the exact text.

## `<opengrid-table>`

A plain native `<table>` for **display**: maximum semantics, ordinary copy and
paste, browser find. No virtualization, no roving tabindex.

```html
<opengrid-table label="Orders" datasource="orders" columns="id,customer,amount">
</opengrid-table>
```

| Attribute | Meaning |
|---|---|
| `label`, `datasource`, `columns` | As in the grid. |

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
| `label`, `datasource` | As in the grid. |
| `rows` | Comma-separated row dimensions, outermost first. |
| `columns` | Comma-separated column dimensions. V1 allows **one**. |
| `values` | The measures, as the contract's own JSON — not an invented shorthand. |

`rows`/`columns` are the two **axes** here, as in every pivot; in the grid
`columns` is the projection. The words are standard in their own context, so
they were kept rather than made unique and worse.

Subtotal rows carry `data-level` and `data-total`, and their row header says so
in words. A group whose dimension value is NULL is named `(no value)`, and one
whose value is the empty string `(empty)` — they are different groups, and an
empty header cell is silence to a screen reader.

## Events

Both fire on the host, `bubbles` and `composed` (without `composed` they would
not leave a shadow root the page wrapped the element in), and neither is
`cancelable` — they report what has already happened.

| Event | `detail` |
|---|---|
| `opengrid-selection-change` | `{ rows: number[], count: number }` — logical row numbers, ascending. |
| `opengrid-cell-change` | `{ row, column, value, previous }` — everything needed to persist it. |

**The component edits; the page saves.** There is no write path: the engine's
contract is a query. An edited value is shown at once and marked unsaved; a
fresh result from the source clears the marks.

Sorting and filtering **drop the selection**, and say so. That is not a
preference: a selection names positions, the grid has no key column, and after a
different sort those positions hold different records.

## Styling

Shadow DOM, so the page reaches in through parts and custom properties.

**Custom properties:** `--grid-row-height`, `--grid-header-height`,
`--grid-filter-height`, `--grid-status-height`, `--grid-border-color`,
`--grid-focus-width`.

**Parts:** `cell`, `column-toggle`, `columns`, `columns-toggle`, `editor`,
`filter`, `filter-clear`, `filter-operator`, `filter-value`, `header`,
`layout`, `page-first`, `page-label`, `page-last`, `page-next`,
`page-previous`, `pager`, `row`, `sort-direction`, `sort-index`, `status`,
`total-row`, `viewport`.

`<opengrid-table>` and `<opengrid-pivot>` ship **no** stylesheet — they are
plain tables and the page owns their look.

## Texts

Everything the components write themselves is English and overridable. A `lang`
given along with the texts is written onto the elements that carry **those
texts** — never onto the data, which is the page's, in the page's language.

```js
loader.module.set_texts(host, { lang: "de", loading: "Wird geladen …" });
```

| Key | Default | Placeholders |
|---|---|---|
| `lang` | — | |
| `loading` | `Loading …` | |
| `matchesOne` / `matchesOther` | `1 match` / `{count} matches` | `{count}` |
| `empty` | `No matches` | |
| `error` / `errorUnknown` | `The data could not be loaded: {cause}` | `{cause}` |
| `filterGroup` | `Filter` | |
| `operatorLabel` / `valueLabel` | `{column} operator` / `{column} value` | `{column}` |
| `clear` | `Clear` | |
| `operators` | the ten filter operators, keyed by wire token | |
| `filterInvalid` | `{column}: {value} is not a value for this column` | `{column}`, `{value}` |
| `cellRequired` | `{column} needs a value` | `{column}` |
| `selectionCleared` | `Selection cleared` | |
| `columnWidth` | `{column} is {width} pixels wide` | `{column}`, `{width}` |
| `columnMoved` | `{column} moved to position {position} of {count}` | `{column}`, `{position}`, `{count}` |
| `columnAtEdge` | `{column} is already at the end` | `{column}` |
| `columnHidden` / `columnShown` | `{column} hidden, {visible} of {count} columns shown` | `{column}`, `{visible}`, `{count}` |
| `columnsGroup` | `Columns` | |
| `pageFirst` / `pagePrevious` / `pageNext` / `pageLast` | `First page` … | |
| `pageOf` | `Page {page} of {pages}` | `{page}`, `{pages}` |
| `total` / `subtotal` | `Total` / `Total {value}` | `{value}` |
| `noValue` / `emptyValue` | `(no value)` / `(empty)` | |
