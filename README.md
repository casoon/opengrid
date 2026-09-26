# opengrid

A portable Rust data and query engine with intelligent client/server execution,
and on top of it an uncompromisingly accessible **DataGrid** and **PivotGrid**
delivered as Web Components.

**The product is the engine, not a JavaScript grid.** The same query model, the
same semantics and the same results run in WebAssembly in a browser, natively on
a server, and against PostgreSQL — and they are proven to agree.

## What is here

| | |
|---|---|
| **One query model** | A JSON AST, never SQL from a browser. 53 conformance cases pin its semantics (NULL ordering, binary collation, exact decimals, NaN, microseconds) and every engine answers all of them identically. |
| **Three ways to run it** | In the tab (WebAssembly over Apache Arrow), on a server (PostgreSQL, one compiled statement), or split between the two — the planner decides from what the source declares it can do. |
| **Accessibility as the design** | A native `<table>` where that suffices and a `role="grid"` where interaction needs one. Every feature is keyboard-operable, every state change goes through one polite live region, and 272 end-to-end tests run axe-core over the result. See [what that does and does not yet prove](#how-far-the-accessibility-claim-goes). |
| **A pivot that is an engine** | Not a grid feature: a pivot is a set of grouping sets plus a reshaping, so it works over any source — and PostgreSQL folds it into a single `GROUPING SETS` statement. |

## Using it

```sh
npm install @casoon/opengrid
```

Three elements ship in one module. `<opengrid-table>` is a plain semantic
`<table>` for displaying data; `<opengrid-grid>` adds selection, editing,
filtering, sorting, column control and either virtualization or paging;
`<opengrid-pivot>` renders a pivot.

```html
<opengrid-grid label="Orders" datasource="orders" columns="id,customer,amount">
</opengrid-grid>

<script type="module">
  import { loadOpengrid, createRestProvider } from "@casoon/opengrid/loader.js";

  const loader = await loadOpengrid();
  loader.module.set_provider(
    document.querySelector("opengrid-grid"),
    createRestProvider({ url: "https://example.org", source: "orders", token: "…" }),
  );
</script>
```

The full surface — elements, attributes, events, parts, custom properties and
translatable texts — is in **[docs/api.md](docs/api.md)**. Runnable examples are
under `examples/`.

In **React, Vue or Svelte**, `@casoon/opengrid-react`, `@casoon/opengrid-vue`
and `@casoon/opengrid-svelte` give the three elements as components, with the
view as state you bind; anywhere else, `connect(host, options)` does the same
from one object. See **[docs/guides/frameworks.md](docs/guides/frameworks.md)**.

## Building

Requires a recent stable Rust with the `wasm32-unknown-unknown` target,
[`just`](https://github.com/casey/just), Node 22 with pnpm, and `wasm-bindgen`
plus `wasm-opt` pinned to the versions in `justfile`.

```sh
just check         # fmt, clippy -D warnings, the whole test suite
just wasm-check    # every wasm-capable crate builds for wasm32
just e2e           # Playwright + axe-core against a real browser
```

`cargo test -p opengrid-datasource-postgres` additionally runs the conformance
suite against a real PostgreSQL. It **skips itself** when there is none, so
`just check` stays green on a machine without a database; point it at one with
`OPENGRID_TEST_PG`.

## How far the accessibility claim goes

Calling something "uncompromisingly accessible" is easy and usually wrong, so
here is the split.

**Verified by tests, on every commit.** Roles, accessible names, `aria-rowcount`
/ `aria-rowindex` / `aria-sort` / `aria-selected`, the roving tabindex and the
whole keyboard matrix, target sizes, the language of the component's own words
versus the page's data, and axe-core over eighteen scenarios. One spec records
the status line's *successive* states, so an announcement that fires twice,
never, or too early to survive the next result is a failing test.

**Not yet verified: a screen reader has not been through the finished V1.** The
protocol exists and is the last open item before a release. It matters: the
sequence-recording spec, the first time it ran, found that turning a page
announced nothing at all — the row count is identical on every page, so the one
live region repeated itself while the content changed underneath. A conformance
test suite would have called that grid accessible.

So: the structure is tested hard, the *experience* is not signed off. Until it
is, treat the accessibility of this library as well-built and unaudited.

## What V1 deliberately does not do

- **No calculated fields.** A column is stored or derived from a date part
  (year, month); there is no expression language.
- **One column dimension in a pivot**, at most 256 generated columns and 2 000
  rows. Over a limit you get an error with a sentence, never a silently
  truncated result.
- **Paging and virtualization are exclusive**, not combined.
- **No CDN build.** Adapters exist for React, Vue and Svelte; Angular, Lit and
  the rest use `connect` directly.

## Status

Pre-1.0. The API is frozen in the sense that it is
written down and a test fails when it changes
(`crates/opengrid-web-components/src/api.rs`) — not in the sense that it will
not change before 1.0. What a `0.x` bump is allowed to break, and the steps a
release actually takes, are in **[docs/releasing.md](docs/releasing.md)**.

One module ships, with all three elements in it: 193 KiB brotli; the pivot costs
8.3 KiB of that next to the grid, while splitting them would duplicate 69.1 KiB
(`just measure-modules`). The Cargo
features `grid` and `pivot` are there for anyone who wants only one.
The engine ships beside it under `engine/`, 335 KiB brotli, loaded only by a
page that queries in the tab or in a worker.

## Licence

MIT OR Apache-2.0 — see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE).
