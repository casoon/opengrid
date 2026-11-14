import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { defineConfig } from "vite";

// One page per grid: pages/<name>.html (and pages/local/<name>.html for grids kept out of the repository). Built, not served from source, because the first-rows
// figure includes downloading and compiling what the page ships — as a visitor would.
const dir = (sub) => {
  try {
    return readdirSync(resolve(import.meta.dirname, "pages", sub)).map((file) => [sub, file]);
  } catch {
    return [];
  }
};
const pages = Object.fromEntries(
  [...readdirSync(resolve(import.meta.dirname, "pages")).filter((f) => f.endsWith(".html")).map((f) => ["", f]), ...dir("local")]
    .filter(([, file]) => file.endsWith(".html"))
    .map(([sub, file]) => [`${sub ? `${sub}/` : ""}${file.replace(/\.html$/, "")}`, resolve(import.meta.dirname, "pages", sub, file)]),
);

// Cross-origin isolated, so `performance.measureUserAgentSpecificMemory()` can say how much
// memory a page holds, WebAssembly included.
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  build: { target: "es2022", rollupOptions: { input: pages }, chunkSizeWarningLimit: 4000 },
  preview: { headers: isolation, port: 4173, strictPort: true },
  worker: { format: "es" },
});
