// Hydration (plan point 81): the server's HTML for the grid, then React
// hydrating it in the browser. The test fetches the HTML from ssr.mjs and hands
// it to `window.__hydrate`, the way a server would put it into the page.
//
// The order is the real one: the markup is there before the element module has
// loaded, and `connect` loads it. React hydrates on its own schedule, so
// whether the element is defined by then is React's timing, not asserted —
// Vue and Svelte hydrate at once, and their tests assert it.
import React from "react";
import { hydrateRoot } from "react-dom/client";
import { loadOpengrid } from "@casoon/opengrid";
import { OpengridGrid } from "@casoon/opengrid-react";
import { MODULE_URL, ordersProvider } from "./provider.js";
import { SSR_PROPS } from "./ssr-props.js";

window.__hydrate = async (html) => {
  const provider = await ordersProvider();
  const root = document.getElementById("root");
  root.innerHTML = html;
  window.__serverNode = root.firstElementChild;
  loadOpengrid({ moduleUrl: MODULE_URL });
  hydrateRoot(root, <OpengridGrid {...SSR_PROPS} provider={provider} />);
};
