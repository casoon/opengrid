// Renders the grid component on the server and prints the HTML (#510). Run by
// tests/e2e/solid.spec.js with Node — no window, no document, no
// customElements — from here, where `solid-js` and the adapter resolve. No JSX
// to compile: the adapter is plain JavaScript, and so is this.
import { createComponent } from "solid-js";
import { renderToString } from "@solidjs/web";
import { OpengridGrid } from "@casoon/opengrid/solid";
import { SSR_PROPS } from "./src/ssr-props.js";

process.stdout.write(renderToString(() => createComponent(OpengridGrid, SSR_PROPS)));
