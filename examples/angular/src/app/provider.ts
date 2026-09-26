// The engine in the tab with the orders data set, as the example's provider
// (plan point 81) — the same as the other examples' src/provider.js. Served
// from the repository root: the engine module and the data load from their
// built places there. Counts what it is asked, for the tests.
import { InjectionToken } from "@angular/core";
import { createLocalProvider, type Engine, type Provider } from "@casoon/opengrid";

/** The provider, handed from `main.ts` to the component. */
export const ORDERS = new InjectionToken<Provider>("the orders provider");

/** The element module from its built place (docs/api.md §Connecting → Loading). */
export const MODULE_URL = "/packages/opengrid/pkg/opengrid_web_components.js";

export async function ordersProvider(): Promise<Provider> {
  // A variable, so the bundler leaves the import to the browser.
  const engineModule = "/examples/engine-demo/pkg/opengrid_wasm.js";
  const { default: init, Engine: EngineClass } = await import(engineModule);
  await init();
  const engine: Engine = new EngineClass();
  const schema = await (await fetch("/crates/opengrid-conformance/data/orders.schema.json")).text();
  const csv = await (await fetch("/tests/e2e/fixtures/grid-virtual.csv")).arrayBuffer();
  engine.load_csv("orders", new Uint8Array(csv), schema);
  const local = createLocalProvider(engine);
  const queries: string[] = [];
  (window as unknown as { __queries: string[] }).__queries = queries;
  return {
    execute(query, mode) {
      queries.push(query);
      return local.execute(query, mode);
    },
  };
}
