<script setup>
// A grid of rooms to edit through props and events alone (issue #206): a new
// row answered by `@row-add`, a paste, an undo, a row action — and a saved
// edit told through the template ref.
import { ref } from "vue";
import { OpengridGrid } from "@casoon/opengrid/vue";
import { ROOM_ACTIONS, ROOM_VALIDATORS } from "./provider.js";

defineProps({ provider: { type: Object, required: true } });

const grid = ref(null);

// For the tests: every editing event, in order.
window.__edits = [];
const record = (type) => (detail) => window.__edits.push({ type, ...detail });

async function onRowAdd(detail) {
  record("rowAdd")(detail);
  return { state: "saved", key: detail.values.id };
}

function onCellChange(detail) {
  record("cellChange")(detail);
  grid.value.setCellState(detail.key, detail.column, "saved");
}
</script>

<template>
  <OpengridGrid
    ref="grid"
    id="rooms"
    label="Rooms"
    datasource="rooms"
    columns="id,name,seats"
    row-key="id"
    add-row
    :provider="provider"
    :actions="ROOM_ACTIONS"
    :validators="ROOM_VALIDATORS"
    @row-add="onRowAdd"
    @cell-change="onCellChange"
    @paste="record('paste')($event)"
    @row-action="record('rowAction')($event)"
  />
</template>
