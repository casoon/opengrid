// opengrid in React (plan point 77): the grid as a React component, its view
// held in React state, and a button that takes the grid out and puts it back —
// the way a route change does.
//
// Served from the repository root (`just serve-demo`, or the e2e server): the
// element module and the engine are loaded from their built places there.

import React, { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { createLocalProvider, loadOpengrid } from "@casoon/opengrid";
import { OpengridGrid } from "@casoon/opengrid-react";

const SAVED = { sort: [{ field: "customer", direction: "asc" }] };

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

function App({ provider }) {
  const [shown, setShown] = useState(true);
  const [view, setView] = useState({ sort: [{ field: "id", direction: "asc" }] });
  const [selected, setSelected] = useState(0);

  // For the tests: what React holds.
  useEffect(() => {
    window.__view = view;
  }, [view]);

  return (
    <main>
      <h1>opengrid in React {React.version}</h1>
      <p>
        <button type="button" onClick={() => setShown((now) => !now)}>
          {shown ? "Hide the grid" : "Show the grid"}
        </button>{" "}
        <button type="button" onClick={() => setView(SAVED)}>
          Restore the saved view
        </button>
      </p>
      <p id="selected">{selected} rows selected</p>
      {shown && (
        <OpengridGrid
          label="Orders"
          datasource="orders"
          columns="id,customer,country,amount,qty"
          windowSize={40}
          selection
          toolbar
          className="orders"
          provider={provider}
          view={view}
          onViewChange={setView}
          onSelectionChange={(detail) => setSelected(detail.count)}
        />
      )}
    </main>
  );
}

// The element module from its built place; a bundler would otherwise copy it
// under a hashed name (docs/api.md §Connecting → Loading).
await loadOpengrid({ moduleUrl: "/packages/opengrid/pkg/opengrid_web_components.js" });
const provider = await ordersProvider();
createRoot(document.getElementById("root")).render(
  <StrictMode>
    <App provider={provider} />
  </StrictMode>,
);
