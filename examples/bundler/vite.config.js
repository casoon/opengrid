import { defineConfig } from "vite";

// Nothing but the defaults, on purpose (#238): minified, production mode, the
// bundler left to find the WebAssembly modules and the worker by itself. Only
// `base` is set, so that the page works when served from a sub-path.
export default defineConfig({
  base: "./",
  build: { emptyOutDir: true },
});
