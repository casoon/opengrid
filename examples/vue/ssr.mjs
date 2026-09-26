// Renders the grid component on the server and prints the HTML (plan point
// 78). Run by tests/e2e/vue.spec.js with Node, from here, where `vue` and the
// adapter resolve.
import { createSSRApp, h } from "vue";
import { renderToString } from "vue/server-renderer";
import { OpengridGrid } from "@casoon/opengrid-vue";
import { SSR_PROPS } from "./src/ssr-props.js";

const app = createSSRApp({ render: () => h(OpengridGrid, SSR_PROPS) });
process.stdout.write(await renderToString(app));
