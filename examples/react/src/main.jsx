// opengrid in React (plan point 77): the grid as a React component, its view
// held in React state, and a button that takes the grid out and puts it back —
// the way a route change does.
//
// Served from the repository root (`just serve-demo`, or the e2e server): the
// element module and the engine are loaded from their built places there.

import React, { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { loadOpengrid } from "@casoon/opengrid";
import { MODULE_URL, ordersProvider } from "./provider.js";
import { OpengridGrid } from "@casoon/opengrid-react";

const SAVED = { sort: [{ field: "customer", direction: "asc" }] };
const GERMAN = { lang: "de", matchesOne: "{count} Treffer", matchesOther: "{count} Treffer" };

function App({ provider }) {
  const [shown, setShown] = useState(true);
  const [view, setView] = useState({ sort: [{ field: "id", direction: "asc" }] });
  const [selected, setSelected] = useState(0);
  const [german, setGerman] = useState(false);
  const [hidden, setHidden] = useState(false);
  const grid = useRef(null);

  // For the tests: what React holds, the element behind the ref, and how often
  // React mounted this — twice under StrictMode in development.
  useEffect(() => {
    window.__view = view;
  }, [view]);
  useEffect(() => {
    window.__mounts = (window.__mounts ?? 0) + 1;
    window.__grid = grid;
  }, []);

  return (
    <main>
      <h1>opengrid in React {React.version}</h1>
      <p>
        <button type="button" onClick={() => setShown((now) => !now)}>
          {shown ? "Hide the grid" : "Show the grid"}
        </button>{" "}
        <button type="button" onClick={() => setView(SAVED)}>
          Restore the saved view
        </button>{" "}
        <button type="button" aria-pressed={german} onClick={() => setGerman((now) => !now)}>
          German
        </button>{" "}
        <button type="button" aria-pressed={hidden} onClick={() => setHidden((now) => !now)}>
          Hidden
        </button>
      </p>
      <p id="selected">{selected} rows selected</p>
      {shown && (
        <OpengridGrid
          ref={grid}
          hidden={hidden}
          label="Orders"
          datasource="orders"
          columns="id,customer,country,amount,qty"
          windowSize={40}
          selection
          toolbar
          className="orders"
          provider={provider}
          texts={german ? GERMAN : undefined}
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
await loadOpengrid({ moduleUrl: MODULE_URL });
const provider = await ordersProvider();
createRoot(document.getElementById("root")).render(
  <StrictMode>
    <App provider={provider} />
  </StrictMode>,
);
