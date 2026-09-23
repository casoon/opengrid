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

The remaining blocker is the screen-reader run over the finished V1
(`docs/releasing.md` → *For any `0.x`*). Everything below is built and tested;
none of it has been listened to.

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
- **Theming** through `::part` and custom properties, with the focus ring,
  `forced-colors` and `prefers-reduced-motion` not switchable off.
- **A frozen public API**, written down in `docs/api.md` and held by a test.

### Accessibility

- Every feature is keyboard-operable; nothing requires dragging (WCAG 2.5.7),
  and targets meet 2.5.8.
- One polite live region carries every state: loading, N matches, no matches,
  and an error that does **not** replace the grid.
- 276 end-to-end tests, including axe-core over eighteen scenarios, and a spec
  that records the status line's *successive* states so an announcement made
  twice, never, or too early is a failing test.
- **Not yet verified by a screen reader.** See `docs/releasing.md`.

### Known limitations

- No calculated fields beyond date parts.
- One column dimension per pivot; at most 256 generated columns and 2,000 rows.
- Paging and virtualization are mutually exclusive.
- `<opengrid-pivot>` requires a server: the browser engine has no pivot export,
  so the element cannot run client-side.
- No CDN build, no framework adapters, no documentation site beyond the project
  page.
