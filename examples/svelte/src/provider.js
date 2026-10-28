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

/**
 * A few rooms the page holds itself (`loadRows`), for the editing grid
 * (issue #206): typed into, pasted into, given actions.
 */
export async function roomsProvider() {
  const { default: init, Engine } = await import(
    /* @vite-ignore */ "/examples/engine-demo/pkg/opengrid_wasm.js"
  );
  await init();
  const provider = createLocalProvider(new Engine());
  await provider.loadRows("rooms", ROOMS, ROOMS_SCHEMA);
  return provider;
}

const ROOMS = [
  { id: 1, name: "A1", seats: 10 },
  { id: 2, name: "B2", seats: 20 },
  { id: 3, name: "C3", seats: 30 },
];

const ROOMS_SCHEMA = JSON.stringify({
  fields: [
    { name: "id", type: "int64", nullable: false },
    { name: "name", type: "utf8", nullable: false },
    { name: "seats", type: "int64", nullable: true },
  ],
});

/** The rooms grid's options, stable across renders (rule 1 in docs/guides/frameworks.md). */
export const ROOM_ACTIONS = [{ id: "remove", label: "Remove" }];
export const ROOM_VALIDATORS = {
  seats: (value) => (Number(value) < 0 ? { level: "error", message: "Seats cannot be negative." } : null),
};

/** The element module from its built place (docs/api.md §Connecting → Loading). */
export const MODULE_URL = "/packages/opengrid/pkg/opengrid_web_components.js";
