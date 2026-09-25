// opengrid in Vue (plan point 78). Served from the repository root, like the
// React example: the element module and the engine load from their built
// places there.

import { createApp } from "vue";
import { createLocalProvider, loadOpengrid } from "@casoon/opengrid";
import App from "./App.vue";

/** The engine in the tab, with the orders data set; counts what it is asked. */
async function ordersProvider() {
  const { default: init, Engine } = await import(
    /* @vite-ignore */ "/examples/engine-demo/pkg/opengrid_wasm.js"
  );
  await init();
  const engine = new Engine();
  const schema = await (await fetch("/crates/opengrid-conformance/data/orders.schema.json")).text();
  const csv = await (await fetch("/tests/e2e/fixtures/grid-virtual.csv")).arrayBuffer();
  engine.load_csv("orders", new Uint8Array(csv), schema);
  const local = createLocalProvider(engine);
  window.__queries = [];
  return {
    execute(query, mode) {
      window.__queries.push(query);
      return local.execute(query, mode);
    },
  };
}

// The element module from its built place (docs/api.md §Connecting → Loading).
await loadOpengrid({ moduleUrl: "/packages/opengrid/pkg/opengrid_web_components.js" });
createApp(App, { provider: await ordersProvider() }).mount("#app");
