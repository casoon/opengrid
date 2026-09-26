import { readFileSync } from "node:fs";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

// The Svelte version for the heading — read here, so the page does not bundle
// the compiler to ask it.
const { version } = JSON.parse(
  readFileSync(new URL("./node_modules/svelte/package.json", import.meta.url), "utf8"),
);

// A development build on purpose (plan point 79): Svelte's warnings exist
// only there, and the tests fail on any of them.
export default defineConfig({
  base: "./",
  plugins: [svelte({ compilerOptions: { dev: true } })],
  define: { __SVELTE_VERSION__: JSON.stringify(version) },
  build: {
    emptyOutDir: true,
    minify: false,
    // Two pages: the example, and the hydration check (plan point 81).
    rollupOptions: {
      input: {
        index: fileURLToPath(new URL("./index.html", import.meta.url)),
        hydrate: fileURLToPath(new URL("./hydrate.html", import.meta.url)),
      },
    },
  },
});
