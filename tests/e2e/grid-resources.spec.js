// The resource report (issue #70): the grid says what every query cost, the
// engine providers say what the engine holds.
import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const ROWS = readFileSync(fileURLToPath(new URL("./fixtures/grid-orders.csv", import.meta.url)), "utf8")
  .trim()
  .split("\n").length - 1;

/** Sorts the grid anew and answers the `opengrid-query` events it fired. */
async function queryEvents(page) {
  return page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    const { loadOpengrid } = await import("/packages/opengrid/loader.js");
    const { module } = await loadOpengrid();
    const seen = [];
    grid.addEventListener("opengrid-query", (event) => seen.push(event.detail));
    module.set_view(grid, { sort: [{ field: "amount", direction: "desc" }] });
    for (let i = 0; i < 50 && seen.length === 0; i += 1) await new Promise((r) => setTimeout(r, 50));
    return seen;
  });
}

test("a query through the worker reports where it ran, how long and how much", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/worker-grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector("td[data-row]"));
  const events = await queryEvents(page);
  expect(events.length).toBeGreaterThan(0);
  const last = events.at(-1);
  expect(last).toMatchObject({ kind: "worker", form: "binary", total: ROWS });
  expect(last.rows).toBeGreaterThan(0);
  expect(last.rows).toBeLessThanOrEqual(ROWS);
  expect(last.ms).toBeGreaterThanOrEqual(0);
  expect(last.bytes).toBeGreaterThan(0);
  expect(last.memory).toBeGreaterThan(0);
});

test("the worker provider says what its engine holds", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/worker-grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  const stats = await page.evaluate(() => window.__provider.stats());
  expect(stats.kind).toBe("worker");
  expect(stats.memory).toBeGreaterThan(0);
  expect(stats.sources).toHaveLength(1);
  expect(stats.sources[0]).toMatchObject({ name: "orders", rows: ROWS, columns: 4 });
  // Two int64 columns, one decimal, one text: at least the fixed-width part.
  expect(stats.sources[0].bytes).toBeGreaterThanOrEqual(ROWS * (8 + 8 + 16));
});

test("the local provider says the same about the engine in the tab", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  const stats = await page.evaluate(() => window.__localProvider.stats());
  expect(stats.kind).toBe("local");
  expect(stats.sources[0]).toMatchObject({ name: "orders", rows: ROWS, columns: 4 });
});

test("a provider of the page's own is reported with its kind and its JSON", async ({ page }) => {
  // The phase F fixture's provider is a plain object that answers JSON.
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector("td[data-row]"));
  const events = await queryEvents(page);
  expect(events.at(-1)).toMatchObject({ kind: "local", form: "json" });
  expect(events.at(-1).bytes).toBeGreaterThan(0);
});
