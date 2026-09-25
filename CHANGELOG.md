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

Nothing is published yet — no npm package, no crates, no tag. The version reads
`0.0.0` everywhere until the first release turns this section into `[0.1.0]`.

The remaining blocker is the screen-reader run — over the finished V1 and over
the configurable views that followed it (`docs/releasing.md` → *For any `0.x`*).
Everything below is built and tested; none of it has been listened to.

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
- **A frozen public API**, written down in `docs/api.md` and held by a test.
- **TypeScript declarations** (`loader.d.ts`) for everything in
  [docs/api.md](docs/api.md), resolved through the package's `exports`. The
  event `detail`s are typed on the element and on the document; a view, a
  column configuration or a text key the API does not take is a compile error.
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
  through `ref`. Tested in both React versions under StrictMode, rendered on
  the server, and installed from its packed tarball.
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
- 319 end-to-end tests, each run on a desktop and a narrow viewport, with
  axe-core in every state the components can be in — open menus, the facet
  sidebar and all five looks of the design prototype included — and a spec that
  records the status line's *successive* states, so an announcement made twice,
  never, or too early is a failing test.
- The language of every leaf node and every accessible name is checked in the
  real DOM: the component's own words carry its language, the page's column
  names and values never do.
- **Not yet verified by a screen reader.** See `docs/releasing.md`.

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
- No CDN build, no framework adapters, no documentation site beyond the project
  page.
