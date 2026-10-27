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

/** The rooms provider, handed from `main.ts` to the component (issue #206). */
export const ROOMS = new InjectionToken<Provider>("the rooms provider");

/**
 * A few rooms the page holds itself (`loadRows`), for the editing grid — the
 * same as the other examples' `roomsProvider`.
 */
export async function roomsProvider(): Promise<Provider> {
  const engineModule = "/examples/engine-demo/pkg/opengrid_wasm.js";
  const { default: init, Engine: EngineClass } = await import(engineModule);
  await init();
  const provider = createLocalProvider(new EngineClass());
  await provider.loadRows(
    "rooms",
    [
      { id: 1, name: "A1", seats: 10 },
      { id: 2, name: "B2", seats: 20 },
      { id: 3, name: "C3", seats: 30 },
    ],
    JSON.stringify({
      fields: [
        { name: "id", type: "int64", nullable: false },
        { name: "name", type: "utf8", nullable: false },
        { name: "seats", type: "int64", nullable: true },
      ],
    }),
  );
  return provider;
}
