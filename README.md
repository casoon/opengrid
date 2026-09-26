# opengrid

Accessible data grid, table and pivot as Web Components — driven by one query engine that
runs in the browser (WebAssembly), on a server (PostgreSQL), or split between the two, with
the same results everywhere.

- **`<opengrid-grid>`** — sorting, filtering, search, facets, grouping with totals,
  selection, editing, virtualized or paged; fully operable from the keyboard.
- **`<opengrid-table>`** — a plain semantic `<table>` for displaying data.
- **`<opengrid-pivot>`** — a pivot with subtotals and a grand total.
- **Export** — what the reader sees, every match of it, as CSV or JSON.

## Install

```sh
npm install @casoon/opengrid
```

The package holds the elements and the query engine. With a bundler, import from
`@casoon/opengrid`; without one, see
[Installation](https://github.com/casoon/opengrid/blob/main/docs/getting-started/installation.mdx).

## Data in the browser

The engine reads a CSV against a schema — it never guesses types.

```html
<opengrid-grid label="Orders" datasource="orders" columns="id,customer,amount"></opengrid-grid>

<script type="module">
  import { connect, createLocalProvider } from "@casoon/opengrid";
  import init, { Engine } from "@casoon/opengrid/engine/opengrid_wasm.js";

  await init();
  const engine = new Engine();
  const csv = await (await fetch("/orders.csv")).arrayBuffer();
  const schema = await (await fetch("/orders.schema.json")).text();
  engine.load_csv("orders", new Uint8Array(csv), schema);

  connect(document.querySelector("opengrid-grid"), {
    provider: createLocalProvider(engine),
  });
</script>
```

```json
{
  "fields": [
    { "name": "id", "type": "int64", "nullable": false },
    { "name": "customer", "type": "utf8", "nullable": true },
    { "name": "amount", "type": { "decimal": { "precision": 12, "scale": 2 } }, "nullable": true }
  ]
}
```

`createWorkerProvider()` runs the same engine in a worker instead.

## Data from a server

`opengrid-server` answers the same queries against PostgreSQL — the browser sends a query
object, never SQL, and the server enforces tokens, a per-tenant row filter and a field
allowlist.

```js
import { connect, createRestProvider } from "@casoon/opengrid";

connect(document.querySelector("opengrid-grid"), {
  provider: createRestProvider({ url: "https://example.org", source: "orders", token: "…" }),
});
```

See [Where queries run](https://github.com/casoon/opengrid/blob/main/docs/guides/where-queries-run.md).

## React, Vue, Svelte

```sh
npm install @casoon/opengrid @casoon/opengrid-react   # or -vue, -svelte
```

```jsx
import { OpengridGrid } from "@casoon/opengrid-react";

<OpengridGrid label="Orders" datasource="orders" columns="id,customer,amount"
              provider={provider} view={view} onViewChange={setView} />
```

Angular, Astro and server-rendered pages use `connect` directly —
[Frameworks](https://github.com/casoon/opengrid/blob/main/docs/guides/frameworks.md).

## Export

```js
import { loadOpengrid, exportRows } from "@casoon/opengrid";

const { module } = await loadOpengrid();
const query = module.get_query(grid);               // the reader's view, without a window
const blob = await exportRows(provider, query, { format: "csv" });
```

CSV per RFC 4180 with a guard against formula injection, or JSON; a pivot exports as shown
with `get_pivot`. See [Export](https://github.com/casoon/opengrid/blob/main/docs/guides/export.md).

## Documentation

- [The public API](https://github.com/casoon/opengrid/blob/main/docs/api.md) — elements,
  attributes, events, the view, styling, texts, errors
- [Guides](https://github.com/casoon/opengrid/tree/main/docs/guides) and the
  [project page](https://casoon.github.io/opengrid/)

## Accessibility

Keyboard operation under the WAI-ARIA grid pattern, one polite live region for every state,
no dragging required, and axe-core in every state of the end-to-end suite (Chromium, Firefox,
WebKit). **Not yet verified with a screen reader** — that pass follows this release.

## Status

Pre-1.0: a minor version may change the API, a patch version does not
([CHANGELOG](https://github.com/casoon/opengrid/blob/main/CHANGELOG.md)).

## Licence

MIT OR Apache-2.0.
