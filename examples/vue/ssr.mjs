// Renders the grid component on the server and prints the HTML (plan point
// 78). Run by tests/e2e/vue.spec.js with Node, from here, where `vue` and the
// adapter resolve.
import { createSSRApp, h } from "vue";
import { renderToString } from "vue/server-renderer";
import { OpengridGrid } from "@casoon/opengrid-vue";

const app = createSSRApp({
  render: () =>
    h(OpengridGrid, {
      label: "Orders",
      datasource: "orders",
      columns: "id,customer",
      windowSize: 40,
      selection: true,
      toolbar: false,
      class: "orders",
      texts: { lang: "de" },
    }),
});
process.stdout.write(await renderToString(app));
