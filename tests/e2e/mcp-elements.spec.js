import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// The elements in an MCP Apps host (#140, E40): the host's CSP allows no
// WebAssembly, so the view carries the wasm2js bundle. The engine answers from
// Node, as it will in the MCP server.

const repo = process.cwd();
const { initSync, Engine } = await import(join(repo, "examples/engine-demo/pkg/opengrid_wasm.js"));
initSync({ module: readFileSync(join(repo, "examples/engine-demo/pkg/opengrid_wasm_bg.wasm")) });
const engine = new Engine();
engine.load_csv(
  "orders",
  readFileSync(join(repo, "tests/e2e/fixtures/grid-virtual.csv")),
  readFileSync(join(repo, "crates/opengrid-conformance/data/orders.schema.json"), "utf8"),
);

test.beforeEach(async ({ page }) => {
  await page.exposeFunction("askServer", (query) => engine.execute(query));
  await page.goto("/tests/e2e/fixtures/mcp-elements.html");
  await page.waitForFunction(() => window.__ready === true);
});

test("the grid runs without WebAssembly, under the host's CSP", async ({ page }) => {
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  const table = page.locator("opengrid-grid table");
  await expect(table).toHaveAttribute("aria-rowcount", "201");
  await expect(page.locator('opengrid-grid [part="status"]')).toContainText("200");

  // Sorting is a new query through the provider, answered from Node.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="3"]').focus(),
  );
  await page.keyboard.press("Enter");
  await expect(page.locator('opengrid-grid th[data-col="3"]')).toHaveAttribute("aria-sort", "ascending");

  // Nothing was blocked: no eval, no network, no WebAssembly asked for.
  expect(await page.evaluate(() => window.__violations)).toEqual([]);

  // And the page really cannot compile WebAssembly: the CSP is a host's.
  const compiles = await page.evaluate(async () => {
    try {
      await WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
      return true;
    } catch {
      return false;
    }
  });
  expect(compiles).toBe(false);
});
