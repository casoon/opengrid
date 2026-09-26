# @casoon/opengrid-vue

The accessible grid, table and pivot of [`@casoon/opengrid`](https://www.npmjs.com/package/@casoon/opengrid)
as components for Vue 3.3 and later: `OpengridGrid`, `OpengridTable` and `OpengridPivot`.

```sh
npm install @casoon/opengrid @casoon/opengrid-vue
```

```vue
<script setup>
import { ref } from "vue";
import { createRestProvider } from "@casoon/opengrid";
import { OpengridGrid } from "@casoon/opengrid-vue";

const provider = createRestProvider({ url: "https://example.org", source: "orders", token: "…" });
const view = ref(null);
</script>

<template>
  <OpengridGrid v-model:view="view" label="Orders" datasource="orders"
                columns="id,customer,amount" :provider="provider" />
</template>
```

The element's attributes are props; the provider, texts, formats, presentation and the view
are options — the view bound the way Vue binds anything. No `compilerOptions.isCustomElement` is needed — the adapter renders the element. Under `<KeepAlive>` the grid comes back with its view and selection.

For data in the browser instead of a server, pass `createLocalProvider(engine)` — see the
[`@casoon/opengrid` README](https://github.com/casoon/opengrid#data-in-the-browser). Everything
the adapters share, and what is tested, is in
[Frameworks](https://github.com/casoon/opengrid/blob/main/docs/guides/frameworks.md).

Licensed under MIT or Apache-2.0, at your option.
