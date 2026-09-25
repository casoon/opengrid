<script setup>
import { ref, watchEffect } from "vue";
import { OpengridGrid } from "@casoon/opengrid-vue";

defineProps({ provider: { type: Object, required: true } });

const SAVED = { sort: [{ field: "customer", direction: "asc" }] };
const GERMAN = { lang: "de", matchesOne: "{count} Treffer", matchesOther: "{count} Treffer" };
const view = ref({ sort: [{ field: "id", direction: "asc" }] });
const selected = ref(0);
const german = ref(false);

// For the tests: what Vue holds.
watchEffect(() => {
  window.__view = view.value;
});
</script>

<template>
  <section>
    <p>
      <button type="button" @click="view = SAVED">Restore the saved view</button>
      <button type="button" :aria-pressed="german" @click="german = !german">German</button>
    </p>
    <p id="selected">{{ selected }} rows selected</p>
    <OpengridGrid
      v-model:view="view"
      label="Orders"
      datasource="orders"
      columns="id,customer,country,amount,qty"
      :window-size="40"
      selection
      toolbar
      class="orders"
      :provider="provider"
      :texts="german ? GERMAN : undefined"
      @selection-change="(detail) => (selected = detail.count)"
    />
  </section>
</template>
