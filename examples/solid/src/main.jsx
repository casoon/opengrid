// opengrid in Solid 2 (#510): the grid as a Solid component, its view held in a
// signal, a lock that refuses the reader's view, and a button that takes the
// grid out and puts it back — the way a route change does. Below it, a grid of
// rooms to edit through props and callbacks alone (issue #206).
//
// Served from the repository root (`just serve-demo`, or the e2e server): the
// element module and the engine are loaded from their built places there.

import { createEffect, createSignal, Show } from "solid-js";
import { render } from "@solidjs/web";
import { loadOpengrid } from "@casoon/opengrid";
import { MODULE_URL, ROOM_ACTIONS, ROOM_COLUMNS, ROOM_VALIDATORS, ordersProvider, roomsProvider } from "./provider.js";
import { OpengridGrid } from "@casoon/opengrid/solid";

const SAVED = { sort: [{ field: "customer", direction: "asc" }] };
const GERMAN = { lang: "de", matchesOne: "{count} Treffer", matchesOther: "{count} Treffer" };

// For the tests: every editing callback, in order.
window.__edits = [];
const record = (type) => (detail) => window.__edits.push({ type, ...detail });

function Rooms(props) {
  let grid;
  return (
    <OpengridGrid
      ref={grid}
      id="rooms"
      label="Rooms"
      datasource="rooms"
      columns="id,name,seats"
      rowKey="id"
      addRow
      provider={props.provider}
      presentation={ROOM_COLUMNS}
      actions={ROOM_ACTIONS}
      validators={ROOM_VALIDATORS}
      onRowAdd={async (detail) => {
        record("rowAdd")(detail);
        return { state: "saved", key: detail.values.id };
      }}
      onCellChange={(detail) => {
        record("cellChange")(detail);
        grid.setCellState(detail.key, detail.column, "saved");
      }}
      onPaste={record("paste")}
      onRowAction={record("rowAction")}
    />
  );
}

function App(props) {
  const [shown, setShown] = createSignal(true);
  const [view, setView] = createSignal({ sort: [{ field: "id", direction: "asc" }] });
  const [selected, setSelected] = createSignal(0);
  const [german, setGerman] = createSignal(false);
  const [hidden, setHidden] = createSignal(false);
  const [locked, setLocked] = createSignal(false);
  const [provider, setProvider] = createSignal(props.provider);

  // For the tests: what Solid holds.
  createEffect(view, (now) => {
    window.__view = now;
  });

  return (
    <main>
      <h1>opengrid in Solid {__SOLID_VERSION__}</h1>
      <p>
        <button type="button" onClick={() => setShown((now) => !now)}>
          {shown() ? "Hide the grid" : "Show the grid"}
        </button>{" "}
        <button type="button" onClick={() => setView(SAVED)}>
          Restore the saved view
        </button>{" "}
        <button type="button" aria-pressed={german()} onClick={() => setGerman((now) => !now)}>
          German
        </button>{" "}
        <button type="button" aria-pressed={hidden()} onClick={() => setHidden((now) => !now)}>
          Hidden
        </button>{" "}
        <button type="button" aria-pressed={locked()} onClick={() => setLocked((now) => !now)}>
          Lock the view
        </button>{" "}
        <button
          type="button"
          onClick={() => setProvider((now) => ({ execute: (query, mode) => now.execute(query, mode) }))}
        >
          New provider
        </button>
      </p>
      <p id="selected">{selected()} rows selected</p>
      <Show when={shown()}>
        <OpengridGrid
          ref={(element) => {
            // For the tests: the element behind the ref.
            window.__grid = element;
          }}
          hidden={hidden()}
          label="Orders"
          datasource="orders"
          columns="id,customer,country,amount,qty"
          windowSize={40}
          selection
          toolbar
          class={["orders", { wide: false }]}
          provider={provider()}
          texts={german() ? GERMAN : undefined}
          view={view()}
          onViewChange={(next) => {
            if (!locked()) setView(next);
          }}
          onSelectionChange={(detail) => setSelected(detail.count)}
        />
      </Show>
      <Rooms provider={props.rooms} />
    </main>
  );
}

// The element module from its built place; a bundler would otherwise copy it
// under a hashed name (docs/api.md §Connecting → Loading).
await loadOpengrid({ moduleUrl: MODULE_URL });
const provider = await ordersProvider();
const rooms = await roomsProvider();
render(() => <App provider={provider} rooms={rooms} />, document.getElementById("root"));
