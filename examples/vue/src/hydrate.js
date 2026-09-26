// Hydration (plan point 81): the server's HTML for the grid, then Vue
// hydrating it in the browser. The test fetches the HTML from ssr.mjs and hands
// it to `window.__hydrate`, the way a server would put it into the page.
//
// The order is the real one: the markup is there before the element module has
// loaded, so Vue hydrates an element that is not defined yet, and `connect`
// loads the module afterwards.
import { createSSRApp, h } from "vue";
import { loadOpengrid } from "@casoon/opengrid";
import { OpengridGrid } from "@casoon/opengrid-vue";
import { MODULE_URL, ordersProvider } from "./provider.js";
import { SSR_PROPS } from "./ssr-props.js";

window.__hydrate = async (html) => {
  const provider = await ordersProvider();
  const root = document.getElementById("root");
  root.innerHTML = html;
  window.__serverNode = root.querySelector("opengrid-grid");
  loadOpengrid({ moduleUrl: MODULE_URL });
  window.__definedAtHydration = !!customElements.get("opengrid-grid");
  createSSRApp({ render: () => h(OpengridGrid, { ...SSR_PROPS, provider }) }).mount(root);
};
