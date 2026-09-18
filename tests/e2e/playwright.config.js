import { defineConfig, devices } from "@playwright/test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// End-to-end and accessibility tests for opengrid (plan point 46,
// plan/spezifikation/12-qualitaet.md §CI).
//
// The suite runs against the repo served from its root, because the fixture
// loads loader.js and the built WASM module under packages/opengrid/. Playwright
// is pinned in package.json; `just e2e` builds the module first.

const PORT = 8080;
// The `opengrid-server` the hybrid fixture queries; the port is also written in
// tests/e2e/fixtures/opengrid-e2e.toml.
const SERVER_PORT = 8082;
const baseURL = `http://127.0.0.1:${PORT}`;
// Serve the repo from its root: the fixture loads /packages/opengrid/loader.js
// and the built module under the same tree.
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

export default defineConfig({
  testDir: ".",
  // One worker: the fixture server is `python3 -m http.server`, which answers
  // one connection at a time; parallel workers fetching the WASM modules at once
  // intermittently get `ERR_CONNECTION_RESET`.
  workers: 1,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["github"], ["list"]] : "list",
  // Baselines are keyed by project only: the pinned Playwright version ships the
  // same Chromium everywhere, and the template stays platform-neutral so the
  // committed baselines are meaningful on CI. If a platform still renders
  // differently, regenerate with `pnpm run e2e:update`.
  snapshotPathTemplate: "{testDir}/__screenshots__/{arg}-{projectName}{ext}",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"] } },
    {
      name: "narrow",
      use: { ...devices["Desktop Chrome"], viewport: { width: 480, height: 900 } },
    },
  ],
  webServer: [
    {
      command: `python3 -m http.server ${PORT}`,
      cwd: repoRoot,
      url: `${baseURL}/tests/e2e/fixtures/table.html`,
      reuseExistingServer: !process.env.CI,
      timeout: 30_000,
    },
    {
      // The real gateway for the hybrid tests (point 28). Waiting on the port
      // rather than a URL: every endpoint needs a bearer token, so a health
      // check would be a 401 and Playwright would call that a failed start.
      command: "cargo run -p opengrid-server -- tests/e2e/fixtures/opengrid-e2e.toml",
      cwd: repoRoot,
      port: SERVER_PORT,
      reuseExistingServer: !process.env.CI,
      timeout: 180_000,
    },
  ],
});
