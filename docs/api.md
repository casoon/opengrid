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
- [The view](#the-view) · [Events](#events) · [Styling](#styling) · [Texts](#texts)

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
| `get_view(host)` / `set_view(host, view)` | Reads and applies the whole [view](#the-view) in one step. |
| `set_columns(host, columns)` | Per-column presentation — see [`<opengrid-grid>`](#opengrid-grid). |
| `get_query(host)` | The query of the current view, without a window — what an [export](#exporting-the-view) sends. |
| `get_pivot(host, options)` | The pivot as it is shown, as CSV — see [exporting a pivot](#exporting-a-pivot). |
| `register()` | Defines the three elements. `loadOpengrid()` calls it; a page that loads the module itself calls it once. |

**Types.** The package ships `loader.d.ts`: every name on this page is typed —
the view, the column configuration, the texts, the formats, the providers, and
the attributes of each element as `Opengrid…Attributes` for adapters and JSX
typings to build on — and the three events are in the global event map,
so `grid.addEventListener("opengrid-view-change", e => e.detail.view)` knows what
`detail` holds, on the element and on the document alike.

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
| `onViewChange`, `onSelectionChange`, `onCellChange` | the three [events](#events); each callback receives the `detail` (for the view: the view itself) |

| | |
|---|---|
| Order | Texts, formats, presentation, choices, view, provider — so the first query is the only one, and the first paint is in the right words. An `update` that changes the texts or the presentation rebuilds the grid, and one that changes the view as well rebuilds it twice: the source is asked twice, and only the second answer is shown. |
| Changes | `update` writes an option only when it differs from what the element has. A key left out keeps its value; a key given as `undefined` resets it — texts to English, no formats, no presentation, no choices. `provider` has no "none"; for `view`, `undefined` means the grid leads. |
| Controlled | `view` is written whenever it differs from what the grid shows: after the reader sorted, passing the same saved view again restores it — and a page that keeps passing a view without taking the reader's changes back holds the grid there, as a controlled input does. Writing back what the grid just reported costs nothing: `onViewChange: (view) => connection.update({ view })`. `onViewChange` hears the reader, not the views `connect` wrote. A view the grid refuses — a column it does not have yet — is tried again on the next `update`. Formats compare functions by identity — keep them stable, or each update redraws. |
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
| `group-by` | Groups the rows by up to **two** columns, outermost first: `group-by="country,customer"`. See [Grouping](#grouping). |
| `search` | Puts a **search field** above the grid: free text, or a filter written out. See [Search](#search). |
| `facets` | Shows the **facet sidebar** — the facets a page configured with `set_columns`. See [Facets](#facets). |
| `toolbar` | Puts a **toolbar** above the grid: the active filters and the grouping as chips, a switch for the filter row, the column list, the density. Opt-in. See [Toolbar](#toolbar). |
| `column-menu` | Gives every header a **column menu**: sort, filter, aggregate, group, hide. Opt-in. See [Column menu](#column-menu). |
| `selection` | Shows the **selection column**: a checkbox per row and one in the header. Opt-in — selecting rows works from the keyboard either way; what this adds is the column that shows it and the pointer path to it. |
| `density` | `compact`, `normal` or `comfortable` — row height, cell padding and font size in one step. `normal` is the default, and a grid without the attribute *is* a normal one; an unknown value is normal too. |

**Keyboard.** The WAI-ARIA grid pattern — arrows, `Home`/`End`, `Ctrl`+`Home`/`End`,
`PageUp`/`PageDown`, and:

| Keys | On | Does |
|---|---|---|
| `Enter` / `Space` | header cell | sort (`Shift` adds a second key) |
| `Enter` | data cell | open the editor (`Enter` commits, `Escape` discards) |
| `Space` | data cell | select the row (`Shift` extends from the last one) |
| `Space` | selection cell | select the row (`Shift` extends) |
| `Enter` / `Space` | selection header | select **every matching row**, or clear it |
| `Ctrl`/`Cmd`+`A` | anywhere | select every matching row, not only the loaded page |
| `Ctrl`/`Cmd`+`←`/`→` | header cell | move the column |
| `Ctrl`/`Cmd`+`Shift`+`←`/`→` | header cell | resize the column |
| `Alt`+`↓`, `Shift`+`F10`, context-menu key | header cell | open the column menu (with `column-menu`) |

With `selection` set, the column is the **start of the row**: `Home` and
`Ctrl`+`Home` go there, and `←` from the first data column reaches it. Its
header is a real `checkbox` inside the header cell — a `columnheader` may not
carry `aria-checked` — and it is tri-state: empty, `mixed`, checked. The
per-row mark is decoration; the row itself says `aria-selected`, and a second
voice per row would double every announcement.

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

**Presentation** is what the schema cannot say:

```js
loader.module.set_columns(host, {
  id:       { width: 96, mono: true, muted: true },
  customer: { emphasis: true },
  amount:   { width: 150, align: "end", aggregate: "sum", facet: "range" },
});
```

| Key | Meaning |
|---|---|
| `width` | The column's **starting** width in pixels. A reader's resize leads after that. |
| `align` | `start`, `end` or `center`. Numbers default to `end`, everything else to `start`. |
| `mono` | Draw the values monospaced, so they line up character by character. |
| `emphasis` / `muted` | Bold, or the muted ink. |
| `aggregate` | `sum`, `avg`, `count`, `min`, `max` or `range` — the column's aggregate in groups. |
| `facet` | `list`, `pills`, `range` or `period` — how the column is offered as a facet. |

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
outside `role="grid"`, like the filter row, so the grid's keys never reach it.

| | |
|---|---|
| Filter row | A switch (`aria-pressed`). Hiding the row gives its height to the viewport, and `PageUp`/`PageDown` step by what is really there. Whether the row shows is part of the [view](#the-view) as `filterRow`, **on** by default — there is no attribute for it, because a boolean attribute is off by default and the row has always been there. |
| Columns | The column list moves here from the filter row, so it stays reachable when the row is hidden. |
| Density | Three buttons, the pressed one is the grid's `density`. |
| Chips | One per active filter, in words (`country is DE`), and one for the grouping. Each has a remove button **named for its filter** — `Remove country is DE` — and "Remove all" clears filters and grouping. A removal is said once, with the result that follows; the focus moves to the next chip, never to the document. |

The chips are a display of the view, not a second truth about the filters: they
are drawn from it, and redrawn only when what they say changed.

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
| `Alpha` | A free-text search: `contains` on every shown text column, or-ed. |

Input that starts with a word and an operator is read as a filter even when the
word is not a column: `colour = red` says "colour is not a column of this grid"
rather than searching for the words and finding nothing.

Operators: `=` `≠` (`!=`) `>` `≥` (`>=`) `<` `≤` (`<=`) `~` (contains) `^`
(starts with). Clauses are joined by the `queryAnd` word — `and` by default —
and `and` always works as well.

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
hiding in the column list, grouping and aggregates through `group-by`,
`set_columns` or the view.

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
column. The menu opens below its header and never covers it.

### Grouping

```html
<opengrid-grid datasource="orders" columns="id,customer,country,amount"
               group-by="country,customer"></opengrid-grid>
```

While `group-by` is set, the grid is a **`treegrid`**: group headers carry
`aria-level` and `aria-expanded`, data rows sit one level below the innermost
group. It stays **virtualized** — the group query answers each group's row
count, and from the counts and the open groups the display list is arithmetic,
so `aria-rowcount` is still one number and scrolling still adds no DOM rows.
The rows of a visible window are fetched per group they belong to: one query per
group the window touches, not one per row.

| | |
|---|---|
| Keys | `Enter` / `Space` open and close a group; on its first cell `→` opens and `←` closes. |
| Pointer | A click on a group row opens or closes it. |
| Label | One sentence per header, `groupRow`: `country: DE (52 rows)`. NULL is `(no value)` and the empty string `(empty)` — two different groups (S10, S14). |
| Status line | Counts **rows**, not display positions — five headers are not five matches. Opening and closing is said once, with the result that follows. |
| Groupable | Text, booleans, whole numbers and dates. A decimal, a float or a timestamp repeats too rarely to group by and is refused. |
| Refused | More than two levels, a column the grid does not show, or `page-size` alongside: the grid says why in the status line and stays ungrouped. |
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

**While grouped, selection and editing are off.** A row number would name
display positions — headers as well as rows — and would move on every toggle;
the page could not map a reported change to anything.

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

`get_pivot(host)` exports the table as it is shown, as CSV — see
[exporting a pivot](#exporting-a-pivot).

## The view

Sort, filters, column layout and density are one value:

```js
const view = loader.module.get_view(grid);
// { sort: [{field, direction}], filters: [{column, op, value}],
//   columns: { order, hidden, widths }, density,
//   group: [], expanded: [], facets: {} }
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
| `group`, `expanded` | The grouping and its open groups. A path of keys that matches no group opens nothing. |
| `aggregates` | The reader's aggregate per column, `{ amount: "sum" }`. Leads over `set_columns`. |
| `filterRow` | Whether the filter row shows. `true` unless turned off. |
| `facets` | The facet selections, by column. |

**The selection is deliberately not in a view.** It names positions, the grid
has no key column, and sorting or filtering drops it precisely because after a
different sort those positions hold different records. A restored view carrying
a selection would not be incomplete — it would be wrong. Applying a view drops
the selection, and says so.

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
| The selection | is not in it. It names positions under exactly this query's sort, so an export of the selection is this query plus the positions from `opengrid-selection-change`. |
| `null` | A grid without a query yet (not connected, no `datasource`, no columns), one whose filter does not hold — its status line says why — and `<opengrid-table>` and `<opengrid-pivot>`. |

The grid has no export button: what to export, in which format, under which name, is the
page's (the same line as for saving an edit).

### Exporting a pivot

A pivot is exported by the element, as it is shown: `get_pivot(host, options)` answers the
table as CSV text, or `null` while nothing is shown.

```js
const csv = loader.module.get_pivot(pivot, { delimiter: ";" });
// "﻿country;2025 · total;2026 · total;(no value) · total\r\n(empty);;114;\r\n…"
const blob = csv && new Blob([csv], { type: "text/csv;charset=utf-8" }); // the file name is the page's
```

| | |
|---|---|
| Columns | The row dimensions, then one column per generated column, in the table's order. |
| Header | **One line.** A generated column is named by its value and its measure, `2025 · total`; without a column dimension, by its measure. |
| Rows | Every row the table shows, in its order: data rows, subtotals, the grand total. |
| Subtotals | Their label — `Total DE`, `Total` — in the first dimension column; the dimension columns it spans are empty fields. |
| Labels | The element's own [texts](#texts): NULL is `(no value)`, the empty string `(empty)`, as in the table, and a page's `set_texts` changes both. |
| Values | As every export writes them: the wire notation, NULL as the `null` option, the formula guard — which covers every header and label, since a dimension value is data. |
| `options` | `{ delimiter, bom, protectFormulas, null }`, each optional, as for a query's export. Any other key is an error. |
| `null` | Before the first answer, while one loads, after an error, and for the grid and the table. |

**Why the element, not a query.** A pivot is bounded — 256 columns, 2 000 rows — and the
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

## Events

All three fire on the host, `bubbles` and `composed` (without `composed` they would
not leave a shadow root the page wrapped the element in), and neither is
`cancelable` — they report what has already happened.

| Event | `detail` |
|---|---|
| `opengrid-selection-change` | `{ rows: number[], count: number }` — logical row numbers, ascending. |
| `opengrid-cell-change` | `{ row, column, value, previous }` — everything needed to persist it. |
| `opengrid-view-change` | `{ view }` — the whole [view](#the-view) after the change. Scrolling and selecting are not view changes. |

**The component edits; the page saves.** There is no write path: the engine's
contract is a query. An edited value is shown at once and marked unsaved; a
fresh result from the source clears the marks.

Sorting and filtering **drop the selection**, and say so. That is not a
preference: a selection names positions, the grid has no key column, and after a
different sort those positions hold different records.

## Styling

Shadow DOM, so the page reaches in through parts and custom properties.

**Custom properties.** A page sets these; the grid computes the accented ones
from them, so picking one accent is enough. Set them **on the element** (`opengrid-grid { … }`),
not only on an ancestor: the grid declares every default on `:host`, and a
declaration on the element wins over an inherited value.

| Set | Default | What it paints |
|---|---|---|
| `--og-font` / `--og-font-size` | `inherit` | everything the grid writes |
| `--og-font-mono` | `ui-monospace, …` | the values of a column marked `mono` |
| `--og-surface` | `Canvas` | rows, the body of the grid |
| `--og-surface-2` | `Canvas` | header, filter row, status line, pager |
| `--og-ink` | `CanvasText` | the text |
| `--og-ink-muted` | a mix of the two | text that is there but not the point |
| `--og-line` | a mix of the two | the rules between rows |
| `--og-line-strong` | a mix of the two | the rules between regions |
| `--og-accent` | `LinkText` | the one colour a page picks — drawn as text too, so a colour meant for text |
| `--og-on-accent` | `Canvas` | text drawn on the accent (the tick of a checked box) |
| `--og-radius` | `0` | the corners of the grid's boxes |
| `--og-pad` | `8px` | horizontal padding inside a cell |
| `--og-focus-width` | `2px` | the focus ring |
| `--og-row-height` | `42px` | a data row; **goes into the window math** |
| `--og-header-height` | `--og-row-height` | the header row |
| `--og-filter-height` | `40px` | the filter row |
| `--og-status-height` | `24px` | the status line, as a minimum |

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

**The defaults are the system colours**, so a grid with no page CSS stays
legible and in the right light or dark. Under `forced-colors` every colour here
resolves to a system colour: a `color-mix` of two system colours resolves
unpredictably, and the user's palette is the one that has to win.

Three things a theme cannot switch off, because they are accessibility rather
than decoration: the **focus ring** never uses `--og-accent` (a pale accent
would make it invisible), a **selected row** carries an inset accent bar as well
as the tint (colour alone would be 1.4.1), and `prefers-reduced-motion` beats a
theme that animates a part.

**Parts:** `body`, `cell`, `chip`, `chip-remove`, `chips`, `chips-clear`, `column-menu`, `column-menu-button`, `column-toggle`, `columns`, `columns-toggle`, `editor`,
`filter`, `filter-clear`, `filter-operator`, `filter-value`, `header`,
`density`, `empty`, `empty-reset`, `empty-text`, `facet`, `facet-bounds`, `facet-cost`, `facet-count`, `facet-pill`,
`facet-pills`, `facet-value`, `facets`, `facets-head`, `facets-toggle`, `filter-row-toggle`, `layout`, `menu-label`, `page-first`, `page-label`, `page-last`, `page-next`,
`page-previous`, `pager`, `row`, `search`, `search-hint`, `search-input`, `search-list`, `select`, `select-all`, `select-mark`,
`sort-direction`, `sort-index`, `status`, `toolbar`,
`total-row`, `viewport`.

`<opengrid-table>` and `<opengrid-pivot>` ship **no** stylesheet — they are
plain tables and the page owns their look.

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
| `filterColumn` | `Filter …` | |
| `aggregateGroup` / `aggregateNone` | `Aggregate in groups` / `No aggregate` | |
| `groupByColumn` / `groupSecondLevel` / `ungroupColumn` | `Group by this column` / `Group as second level` / `Remove this grouping` | |
| `hideColumn` | `Hide column` | |
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
| `searchLabel` / `searchPlaceholder` | `Search or filter` / `Search, or filter: country = DE and amount ≥ 10` | |
| `queryAnd` | `and` | |
| `searchHint` / `searchSuggestions` | `Query · Enter` / `Columns` | |
| `typeText` / `typeBool` / `typeInteger` / `typeNumber` / `typeDate` / `typeTime` | `text` / `yes/no` / `integer` / `number` / `date` / `time` — the type beside a suggested column | |
| `searchChip` | `Text contains “{text}”` | `{text}` |
| `queryUnknownColumn` / `queryMissingValue` | `{column} is not a column of this grid` / `{column}: the value is missing` | `{column}` |
| `queryWrongOperator` | `{column} does not take {operator}` | `{column}`, `{operator}` |
| `emptyFiltered` / `emptySource` | `No row matches these filters.` / `There are no rows.` | |
| `emptyReset` | `Reset filters` | |
| `operators` | the ten filter operators, keyed by wire token | |
| `filterInvalid` | `{column}: {value} is not a value for this column` | `{column}`, `{value}` |
| `cellRequired` | `{column} needs a value` | `{column}` |
| `selectionCleared` | `Selection cleared` | |
| `columnWidth` | `{column} is {width} pixels wide` | `{column}`, `{width}` |
| `columnMoved` | `{column} moved to position {position} of {count}` | `{column}`, `{position}`, `{count}` |
| `columnAtEdge` | `{column} is already at the end` | `{column}` |
| `columnHidden` / `columnShown` | `{column} hidden, {visible} of {count} columns shown` | `{column}`, `{visible}`, `{count}` |
| `columnsGroup` | `Columns` | |
| `pageFirst` / `pagePrevious` / `pageNext` / `pageLast` | `First page` / `Previous page` / `Next page` / `Last page` | |
| `pageOf` | `Page {page} of {pages}` | `{page}`, `{pages}` |
| `total` / `subtotal` | `Total` / `Total {value}` | `{value}` |
| `noValue` / `emptyValue` | `(no value)` / `(empty)` | |
