// Renders the grid component on the server and prints the HTML (plan point
// 77). Run by tests/e2e/react.spec.js with Node — no window, no document, no
// customElements — from here, where `react` and the adapter resolve.
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { OpengridGrid } from "@casoon/opengrid-react";

process.stdout.write(
  renderToString(
    createElement(OpengridGrid, {
      label: "Orders",
      datasource: "orders",
      columns: "id,customer",
      windowSize: 40,
      selection: true,
      toolbar: false,
      className: "orders",
      texts: { lang: "de" },
    }),
  ),
);
