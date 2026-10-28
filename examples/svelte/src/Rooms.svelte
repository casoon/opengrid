<script>
  // A grid of rooms to edit through props and callbacks alone (issue #206): a
  // new row answered by `onrowadd`, a paste, an undo, a row action — and a
  // saved edit told through `bind:this`.
  import { OpengridGrid } from "@casoon/opengrid/svelte";
  import { ROOM_ACTIONS, ROOM_VALIDATORS } from "./provider.js";

  let { provider } = $props();
  let grid;

  // For the tests: every editing callback, in order.
  window.__edits = [];
  const record = (type) => (detail) => window.__edits.push({ type, ...detail });
</script>

<OpengridGrid
  bind:this={grid}
  id="rooms"
  label="Rooms"
  datasource="rooms"
  columns="id,name,seats"
  rowKey="id"
  addRow
  {provider}
  actions={ROOM_ACTIONS}
  validators={ROOM_VALIDATORS}
  onrowadd={async (detail) => {
    record("rowAdd")(detail);
    return { state: "saved", key: detail.values.id };
  }}
  oncellchange={(detail) => {
    record("cellChange")(detail);
    grid.setCellState(detail.key, detail.column, "saved");
  }}
  onpaste={record("paste")}
  onrowaction={record("rowAction")}
/>
