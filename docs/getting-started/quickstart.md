---
title: Quickstart
description: A grid over rows the page holds, then a table over a CSV file with a schema — the engine running in the browser.
order: 2
---

This page puts a grid on a page and answers its queries with the engine in the browser. It
assumes the elements and the engine are in place as in [Installation](../installation/).

## Rows to a grid

```html
<opengrid-grid label="Orders"></opengrid-grid>

<script type="module">
  import { fromRows } from "./opengrid/loader.js";
  const rows = [
    { id: 1, customer: "Ada", country: "UK", amount: 120 },
    { id: 2, customer: "Grace", country: "US", amount: 80 },
  ];
  await fromRows(document.querySelector("opengrid-grid"), rows);
</script>
```

`label` is the grid's accessible name. `fromRows` reads the types from the rows, runs the engine
in a worker, loads the rows and shows every field. What it reads, and what it refuses:
[Data you already have](../../guides/data-you-have/).

## When you need exact types

A money column wants a decimal, not a float, and a CSV file has no types at all. Then the engine
reads against an explicit schema; this part puts an `<opengrid-table>` over a CSV file.

### 1. The element

```html
<opengrid-table label="Orders" datasource="orders"
                columns="id,customer,country,amount"></opengrid-table>
```

`label` is the table's accessible name, `datasource` becomes the query's `source`, and
`columns` lists the output fields in order.

### 2. A source

The engine never guesses types: a CSV is read against an explicit schema. The repository's
conformance dataset is a working pair to start from —
`crates/opengrid-conformance/data/orders.csv` and `orders.schema.json`.

```json
{
  "fields": [
    { "name": "id", "type": "int64", "nullable": false },
    { "name": "customer", "type": "utf8", "nullable": true },
    { "name": "amount", "type": { "decimal": { "precision": 12, "scale": 2 } }, "nullable": true }
  ]
}
```

### 3. Load and attach

```html
<script type="module">
  import { loadOpengrid, createLocalProvider } from "./opengrid/loader.js";
  import init, { Engine } from "./opengrid/engine/opengrid_wasm.js";

  await init();
  const engine = new Engine();
  const schema = await (await fetch("./orders.schema.json")).text();
  const csv = await (await fetch("./orders.csv")).arrayBuffer();
  engine.load_csv("orders", new Uint8Array(csv), schema);

  const loader = await loadOpengrid();
  loader.module.set_provider(
    document.querySelector("opengrid-table"),
    createLocalProvider(engine),
  );
</script>
```

`loadOpengrid()` loads the WASM module and registers the elements. If the module cannot be
loaded, it installs a plain-DOM stand-in and answers `{ fallback: true }`, so the page does not
break.

Serve the files over HTTP — a module script does not load from `file://`. With a bundler,
import from `@casoon/opengrid` and `@casoon/opengrid/engine/opengrid_wasm.js` instead of the
relative paths.

## Next

- Swap `<opengrid-table>` for `<opengrid-grid>` to get selection, editing, filtering and
  virtualization. [The public API](../../api/) lists its attributes and keys.
- Run the engine in a worker, or on a server: [Where queries run](../../guides/where-queries-run/).
