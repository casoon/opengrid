---
title: Quickstart
description: A table over a CSV file, with the engine running in the browser.
order: 2
---

This page puts an `<opengrid-table>` on a page and answers its queries with the engine in the
tab. It assumes the elements and the engine are in place as in [Installation](../installation/).

## 1. The element

```html
<opengrid-table label="Orders" datasource="orders"
                columns="id,customer,country,amount"></opengrid-table>
```

`label` is the table's accessible name, `datasource` becomes the query's `source`, and
`columns` lists the output fields in order.

## 2. A source

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

## 3. Load and attach

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
