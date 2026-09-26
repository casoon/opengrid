// opengrid in Svelte 5 (plan point 79). Served from the repository root, like
// the React and Vue examples: the element module and the engine load from
// their built places there.

import { mount } from "svelte";
import { loadOpengrid } from "@casoon/opengrid";
import { MODULE_URL, ordersProvider } from "./provider.js";
import App from "./App.svelte";

// The element module from its built place (docs/api.md §Connecting → Loading).
await loadOpengrid({ moduleUrl: MODULE_URL });
mount(App, { target: document.getElementById("app"), props: { provider: await ordersProvider() } });
