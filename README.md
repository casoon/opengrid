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
| **Accessibility as the design** | A native `<table>` where that suffices and a `role="grid"` where interaction needs one. Every feature is keyboard-operable, every state is announced, and 250+ end-to-end tests run axe-core over the result. |
| **A pivot that is an engine** | Not a grid feature: a pivot is a set of grouping sets plus a reshaping, so it works over any source — and PostgreSQL folds it into a single `GROUPING SETS` statement. |

## Using it

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

## Status

Pre-1.0. The API is frozen in the sense that it is written down and a test
fails when it changes (`crates/opengrid-web-components/src/api.rs`) — not in the
sense that it will not change before 1.0.

## Licence

MIT OR Apache-2.0.
