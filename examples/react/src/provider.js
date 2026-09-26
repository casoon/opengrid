// The engine in the tab with the orders data set, as the examples' provider
// (plan points 77–81). Served from the repository root: the engine module and
// the data load from their built places there. Counts what it is asked, for
// the tests.
import { createLocalProvider } from "@casoon/opengrid";

export async function ordersProvider() {
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

/** The element module from its built place (docs/api.md §Connecting → Loading). */
export const MODULE_URL = "/packages/opengrid/pkg/opengrid_web_components.js";
