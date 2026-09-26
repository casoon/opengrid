# Changelog

Notable changes to `@casoon/opengrid` and the `opengrid-*` crates. The npm
package and the crates carry the same version and are released together, so one
entry covers both.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with the `0.x` reading written down in [docs/releasing.md](docs/releasing.md): a
minor bump may break the API, a patch bump may not.

Two things belong in every release entry and are easy to leave out:

- **What breaks**, named, whenever the minor version moves. "Renamed X to Y" is
  an entry; "various improvements" is not.
- **Which screen-reader pairings were tested**, and which were not. An untested
  pairing is a documented gap until 1.0, so it has to be documented.

## [Unreleased]

## [0.1.0] — 2026-09-26

The first release: `@casoon/opengrid`, `@casoon/opengrid-react`,
`@casoon/opengrid-vue`, `@casoon/opengrid-svelte` and the `opengrid-*` crates,
all at `0.1.0`. Nothing breaks — there was nothing before it.

**Screen-reader pairings tested: none yet.** Everything below is built and
tested — keyboard, focus, announcements, axe-core in every state — but no screen
reader has been run over it. The passes follow this release
([issue #5](https://github.com/casoon/opengrid/issues/5)). **Browsers:**
Chromium, Firefox and WebKit through the end-to-end suite (Firefox and WebKit at
a desktop viewport, `just e2e-browsers`); Safari itself, iOS and Edge not
tested.

### Added

- **One query model.** A JSON AST — filter, sort, group, aggregate, paging,
  projection — that a browser sends instead of SQL. Its semantics are pinned by
  a conformance suite of 53 query cases and 5 pivot cases covering NULL
  ordering, binary collation, exact decimals, non-finite floats and microsecond
  timestamps.
- **Three ways to run that model, proven to agree.** In the tab over Apache
  Arrow compiled to WebAssembly; on a server against PostgreSQL as one compiled
  statement; or split between the two, with a planner deciding from what the
  source declares it can do. All three answer every conformance case
  identically.
- **`<opengrid-table>`** — an ordinary semantic `<table>` for display, so
  copying, find-in-page and screen readers behave as they do anywhere else.
- **`<opengrid-grid>`** — the interactive element: keyboard navigation under the
  WAI-ARIA grid pattern, virtualized row recycling that keeps a ~40-row DOM
  window over 100,000 logical rows, a type-aware filter row, multi-column sort,
  row selection, cell editing, columns that move, resize and hide from the
  keyboard alone, and paging as an alternative to virtualization.
- **`<opengrid-pivot>`** — a pivot as a set of grouping sets plus a reshaping,
  which PostgreSQL folds into a single `GROUPING SETS` statement. Limits are
  errors with a sentence, never a silently truncated result.
- **`opengrid-server`** — the gateway: bearer tokens from configuration, a
  mandatory row filter derived from the token's context that a client cannot
  opt out of, an `allowed_fields` allowlist, and explicit CORS origins.
- **Derived columns**, computed from a date part (year, month) without an
  expression language.
- **Cell formatting** through `Intl` in the browser, kept strictly out of the
  query: sorting stays binary, decimals stay exact, dates are computed in UTC.
- **Overridable texts** via `set_texts`, with the language declared on the nodes
  carrying the component's own words and never on the page's data.
- **Theming** through eighteen `--og-*` custom properties and `::part`. The
  defaults are system colours, so an unthemed grid follows light and dark on its
  own; the focus ring, `forced-colors` and `prefers-reduced-motion` cannot be
  switched off. `density` sets row height, padding and font size in one step.
- **Configurable views.** The schema from the source is the truth;
  `set_columns` narrows it — width, alignment, emphasis, the aggregate a column
  shows, how it is offered as a facet — and refuses, in words, what the type
  contradicts. The reader's view (sort, filters, columns, density, grouping,
  facets) is one value: `get_view`, `set_view` and `opengrid-view-change`. A
  saved view is that value with a name on it, so naming and storing views stays
  with the page.
- **Grouping** by up to two columns, still virtualized, as a `treegrid` while
  grouped, with sums, averages, counts, minima, maxima or the range a group
  spans, and a grand total as the last row.
- **Search** that takes free text or a filter written out
  (`country = DE and amount ≥ 10`) as an ARIA combobox, and turns the latter
  into the filter row's own entries rather than a second kind of filter.
- **Facets** — lists, toggle buttons, ranges and periods — each counted
  without its own restriction, with the cost of the counting said.
- **A toolbar** with the active filters as chips, a **column menu** as a second
  way to sort, filter, aggregate, group and hide, a **selection column**, and an
  **empty state** that says why and offers a way out only where one exists.
- **The engine ships in `@casoon/opengrid`**, under `engine/` — `Engine` and
  `Planner`, imported as `@casoon/opengrid/engine/opengrid_wasm.js` to query
  in the tab. `createWorkerProvider()` defaults to it; `moduleUrl` and
  `wasmUrl` still pick another.
- **A frozen public API**, written down in `docs/api.md` and held by a test.
- **TypeScript declarations** (`loader.d.ts`) for everything in
  [docs/api.md](docs/api.md), resolved through the package's `exports`. The
  event `detail`s are typed on the element and on the document; a view, a
  column configuration or a text key the API does not take is a compile error.
- **`opengrid-export`** — a query result as CSV (RFC 4180, UTF-8 with a byte
  order mark, a guard against formula injection in text cells) or JSON rows,
  in the wire notation and piece by piece; the same code for the browser and
  the server. With NULL spelled `\N` and the guard off, an export reads back
  into opengrid unchanged — except a text that is `\N` itself, which comes
  back as NULL.
- **`get_query(host)`** hands out the query of the grid's current view —
  the filter row, the facets and the search and-ed together, the sort, the
  shown columns in their order — without a window: what a page exports is
  then what the reader sees, every match of it. Grouped, the group keys lead,
  NULL last said explicitly, and the rows come in the order the grid draws
  them. A grouped grid whose filter does not hold says so in its status line,
  as an ungrouped one does, instead of falling back to the filter row alone.
- **`exportRows(provider, query, options)`** — every match of a query, through
  any provider, fetched in pieces of `chunkSize` (10 000) and handed back as a
  `Blob` of CSV or JSON in the `opengrid-export` notation. The sort is made
  total by appending every selected column not yet in it, so a tie cannot
  repeat or drop a row between two pieces, even over PostgreSQL; within a tie
  the export follows the columns, not the grid. A source whose count changes
  during the export is detected and refused with an error, not exported.
  Progress after each piece, an
  `AbortSignal` that stops the request in flight, and `maxRows` (1 000 000) as
  an error with a sentence, never a truncated file. Providers take the signal
  as an optional third argument, `execute(query, mode, { signal })`; the REST,
  pivot and hybrid providers hand it to `fetch`. The prototype page exports its
  current view with it.
- **`POST /export/{source}`** on `opengrid-server` streams every row of a query
  as CSV or JSON, under the rules of `POST /query` — the token, `allowed_fields`,
  the tenant's `row_filter`, both validations — with its own bound,
  `max_export_rows` (1 000 000), instead of `max_limit`. More rows are a `413`
  before the first byte, counted in the same `REPEATABLE READ` snapshot a
  PostgreSQL cursor then reads; `timeout_ms` bounds the time to the first byte,
  each fetch, and the time a client may take to accept each piece. At most
  `max_concurrent_exports` run at once (half the smallest PostgreSQL pool by
  default; one more is a `503` with the error code `busy`, before any database
  work), and PostgreSQL's own
  `idle_in_transaction_session_timeout` backs both up. The body is a bounded
  channel, so a slow client slows the reading and a client that leaves or
  stalls ends the query; a failure midway — a panic included — breaks the
  connection off rather than ending a short file. `Content-Disposition` names
  the file after the source, `X-Total-Count` carries the row count. A million
  rows from PostgreSQL keep the server under 40 MiB (measured in
  [docs/guides/where-queries-run.md](docs/guides/where-queries-run.md)).
  `createRestProvider(...).export(query, options)` fetches it as a `Blob`, and
  `exportRows` uses a provider's `export` when there is one — one request
  instead of pieces, the same file. No request of the REST or pivot provider
  follows a redirect, so the token goes nowhere but the configured URL.
- **Errors a page can tell apart without reading the sentence.** A rejection
  of `createRestProvider` (`describe`, `execute`, `export`) and
  `createPivotProvider` carries the server's HTTP `status`, and the `code` and
  `path` of its error form, as fields on the `Error`; `exportRows`' own
  refusals carry a `code` — `too_many_rows`, `source_changed`,
  `module_not_loaded`. The message is what it was, and there is no error class:
  a page switches on `error.code`. The server's codes are the closed list of
  its error form `{ "error": { "code", "message", "path" } }` — part of the
  wire format — and `busy` (`503`, try again later) is kept apart from
  `limit_exceeded` (`413`, narrow the request). Typed as `CodedError` and
  `ErrorCode`, frozen with the rest of the API, and listed with their statuses
  in [docs/api.md → Errors](docs/api.md#errors).
- Under the formula guard, the CSV option `null` may not start like a formula
  (`=`, `+`, `-`, `@`, a tab): it is written into every empty cell unguarded.
  `exportRows`, `get_pivot` and the server refuse it with a sentence.
- Against PostgreSQL, a query with aggregates and no grouping reports
  `total_count` 1 — its one row — as the local engine does, instead of the
  number of rows it aggregated.
- **An export guide** ([docs/guides/export.md](docs/guides/export.md)): what an export
  holds and why raw values, the browser and the server path, a pivot as shown, the formula
  guard and when to switch it off, NULL in a CSV, reading an export back into opengrid, and
  the errors a page handles. The framework guide shows the export for each adapter.
- **`get_pivot(host, options)`** exports an `<opengrid-pivot>` as it is shown,
  as CSV: the row dimensions as columns, one header line naming each generated
  column by its value and measure (`2025 · total`), the subtotals and the grand
  total as rows with their labels, NULL and the empty group named as in the
  table — in the element's own texts. The element exports the answer it holds
  rather than asking again, so the file is the table on screen. In the crates:
  `opengrid_export::pivot_csv`, and `opengrid_pivot::pivot_from_json`, which
  reads the pivot wire form back and refuses one of the wrong shape.
- **`connect(host, options)`** supplies an element from one object — provider,
  texts, formats, presentation, choices, a controlled view and the three event
  callbacks — in the one order that asks the source once, after the module has
  loaded, and keeps it supplied: `update` writes only what changed, and writing
  back the view the grid just reported costs nothing. The seam for framework
  adapters, usable as it is from Angular, Lit or plain pages.
- **`@casoon/opengrid-react`** — `OpengridGrid`, `OpengridTable` and
  `OpengridPivot` for React 18 and 19: the attributes as props (rendered, so
  they are in the server HTML), everything else through `connect`, the view
  controlled (`view` + `onViewChange`) or not (`defaultView`), the element
  through `ref`, `"use client"` for Server Components. Tested in React 18
  and 19 under StrictMode; rendered on the server with React 19; the packed
  tarball checked in a scratch project (server rendering and types).
- **`@casoon/opengrid-vue`** — the same three components for Vue 3.3 and
  later: `v-model:view`, `@selection-change`, `@cell-change`, the other
  options as props. Under `<KeepAlive>` a grid comes back with its view and
  selection and asks nothing; taken out for good, it is collected. Rendered
  on the server, and its packed tarball checked like the React one.
- **`@casoon/opengrid-svelte`** — the three components for Svelte 5, shipped
  as `.svelte` sources: `bind:view`, `bind:element`, `onselectionchange`,
  `oncellchange`, the other options as props. Rendered on the server through
  Vite's SSR with the Svelte plugin, and its packed tarball checked like the
  others.
- **Elements that come and go.** A grid or table removed from the page is
  garbage-collected, and with it its rows and whatever the page handed only to
  it — the provider, a format function — even when those close over the
  element. A grid that is moved, or taken out and put back later (Vue's
  `<KeepAlive>`, a detached tab panel), keeps its view, selection, active cell
  and scroll position. It asks its source again only for rows it no longer
  holds: when its active cell had been scrolled out of the loaded window, it
  comes back at that cell, so `Tab` still finds the grid.

### Accessibility

- Every feature is keyboard-operable; nothing requires dragging (WCAG 2.5.7),
  and targets meet 2.5.8.
- One polite live region carries every state: loading, N matches, no matches,
  and an error that does **not** replace the grid.
- 880 end-to-end test runs in Chromium — most specs on both a desktop and a
  narrow viewport — and the same suite in Firefox (433) and WebKit (432) before a
  release, with
  axe-core in every state the components can be in — open menus, the facet
  sidebar and all five looks of the design prototype included — and a spec that
  records the status line's *successive* states, so an announcement made twice,
  never, or too early is a failing test.
- The language of every leaf node and every accessible name is checked in the
  real DOM: the component's own words carry its language, the page's column
  names and values never do.
- New texts or a new presentation rebuild the grid without taking the focus:
  the active cell gets it back only if the grid had it, so `set_texts` while a
  page loads — or from the page's own language switch — leaves the focus where
  the reader put it.
- Scroll areas are not tab stops in Firefox, which makes every scroller
  focusable: Tab meets the controls and the grid, never an unnamed box. A key
  pressed on the grid's scroll area after a click into empty space scrolls, and
  does nothing to a cell the reader cannot see.
- A filter or facet control reached by Tab is scrolled fully into view, also in
  the narrow layout.
- In WebKit the filter row's selects are drawn by the grid, so they meet the
  24 px target and follow the theme; under forced colours they fall back to the
  native control.
- **Not yet verified by a screen reader.** The passes follow the first release
  ([issue #5](https://github.com/casoon/opengrid/issues/5)); see `docs/releasing.md`.

### Known limitations

- No calculated fields beyond date parts.
- One column dimension per pivot; at most 256 generated columns and 2,000 rows.
- Paging and virtualization are mutually exclusive, and so are paging and
  grouping.
- At most two levels of grouping; while grouped, rows can be neither selected
  nor edited, because both are reported by position and a position would include
  the group headers.
- `<opengrid-pivot>` requires a server: the browser engine has no pivot export,
  so the element cannot run client-side.
- No CDN build, no documentation site beyond the project page. Adapters exist for
  React, Vue and Svelte; Angular uses a directive over `connect`, run in an
  Angular 22 example; the rest use `connect` directly
  ([docs/guides/frameworks.md](docs/guides/frameworks.md)). Hydration is tested
  in all three adapters; Vite's development server only checked by hand; not
  tested: React 18 rendering on the server, SvelteKit, Nuxt, Next.js and Astro
  as real applications. A Vite production build needs `loadOpengrid({ moduleUrl })`.
