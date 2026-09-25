<script setup>
// Two tabs under <KeepAlive> — switching away detaches the grid and switching
// back brings it back as it was — and a switch that takes the whole thing out,
// the way leaving the route does.
import { ref, version } from "vue";
import GridTab from "./GridTab.vue";
import OtherTab from "./OtherTab.vue";

defineProps({ provider: { type: Object, required: true } });
const tab = ref("grid");
const shown = ref(true);
</script>

<template>
  <main>
    <h1>opengrid in Vue {{ version }}</h1>
    <p>
      <button type="button" @click="shown = !shown">
        {{ shown ? "Leave the page" : "Come back" }}
      </button>
    </p>
    <template v-if="shown">
      <p>
        <button type="button" :aria-pressed="tab === 'grid'" @click="tab = 'grid'">Orders</button>
        <button type="button" :aria-pressed="tab === 'other'" @click="tab = 'other'">Other</button>
      </p>
      <KeepAlive>
        <GridTab v-if="tab === 'grid'" :provider="provider" />
        <OtherTab v-else />
      </KeepAlive>
    </template>
  </main>
</template>
