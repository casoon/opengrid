// Renders the grid component on the server and prints the HTML (plan point
// 79). The adapter ships `.svelte` sources, so they are compiled first — by
// Vite with the Svelte plugin, the way SvelteKit or an Astro island would —
// rooted where this file sits, so the same script runs in the workspace and in
// the scratch project of scripts/check-adapter-package.sh.
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { createServer } from "vite";

const server = await createServer({
  root: dirname(fileURLToPath(import.meta.url)),
  configFile: false,
  logLevel: "silent",
  appType: "custom",
  server: { middlewareMode: true, hmr: false, ws: false },
  plugins: [svelte()],
});
try {
  const { render } = await server.ssrLoadModule("svelte/server");
  const { OpengridGrid } = await server.ssrLoadModule("@casoon/opengrid-svelte");
  const { body } = render(OpengridGrid, {
    props: {
      label: "Orders",
      datasource: "orders",
      columns: "id,customer",
      windowSize: 40,
      selection: true,
      toolbar: false,
      class: "orders",
      texts: { lang: "de" },
    },
  });
  process.stdout.write(body);
} finally {
  await server.close();
}
