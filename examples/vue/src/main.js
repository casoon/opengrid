// opengrid in Vue (plan point 78). Served from the repository root, like the
// React example: the element module and the engine load from their built
// places there.

import { createApp } from "vue";
import { loadOpengrid } from "@casoon/opengrid";
import { MODULE_URL, ordersProvider } from "./provider.js";
import App from "./App.vue";

// The element module from its built place (docs/api.md §Connecting → Loading).
await loadOpengrid({ moduleUrl: MODULE_URL });
createApp(App, { provider: await ordersProvider() }).mount("#app");
