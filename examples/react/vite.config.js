import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const here = dirname(fileURLToPath(import.meta.url));

// Built twice (plan point 77): `dist/` against React 19, `dist-18/` against
// React 18 — the adapter promises both, so both are tested. Development
// builds on purpose: StrictMode mounts every component twice only there, and
// that double mount is one of the things the tests hold.
//
// React 18 comes in through npm aliases (`react-18`, `react-dom-18`). The
// aliases point at absolute paths: the adapter imports `react` from its own
// package, where `react-18` does not resolve, and one React in the bundle is
// the condition for hooks to work at all.
export default defineConfig(({ mode }) => {
  const react18 = mode === "react18";
  const from = (name) => resolve(here, "node_modules", name);
  return {
    base: "./",
    define: { "process.env.NODE_ENV": JSON.stringify("development") },
    resolve: {
      dedupe: ["react", "react-dom"],
      alias: react18
        ? [
            { find: /^react-dom(\/.*)?$/, replacement: `${from("react-dom-18")}$1` },
            { find: /^react(\/.*)?$/, replacement: `${from("react-18")}$1` },
          ]
        : [],
    },
    build: {
      outDir: react18 ? "dist-18" : "dist",
      emptyOutDir: true,
      minify: false,
      rollupOptions: {
        // The adapter says "use client" for React Server Components; a
        // client bundle has no use for it, and the bundler says so.
        onwarn(warning, warn) {
          if (warning.code !== "MODULE_LEVEL_DIRECTIVE") warn(warning);
        },
      },
    },
  };
});
