import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import solid from "@solidjs/vite-plugin";
import { defineConfig } from "vite";

const here = dirname(fileURLToPath(import.meta.url));

// The Solid version for the heading — read here, so the page does not bundle
// anything to ask it.
const { version } = JSON.parse(
  readFileSync(new URL("./node_modules/solid-js/package.json", import.meta.url), "utf8"),
);

// A development build on purpose (#510): Solid's warnings — a read outside a
// tracking scope, a write inside an owned one — exist only there, and the tests
// fail on any of them. The adapter imports `solid-js` from the package's own
// directory, so both are deduplicated to the one the example has.
export default defineConfig({
  base: "./",
  plugins: [solid({ dev: true })],
  define: { __SOLID_VERSION__: JSON.stringify(version) },
  resolve: { dedupe: ["solid-js", "@solidjs/web"] },
  build: {
    emptyOutDir: true,
    minify: false,
    // Two pages: the example, and the hydration check (plan point 81).
    rollupOptions: {
      input: { index: resolve(here, "index.html"), hydrate: resolve(here, "hydrate.html") },
    },
  },
});
