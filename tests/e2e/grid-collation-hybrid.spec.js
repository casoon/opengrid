import { readFileSync } from "node:fs";

import { test, expect } from "@playwright/test";

// A sort by a locale over a source that has no collations (issue #408, rule
// S4a): the planner leaves it to the engine in the tab, and the hybrid provider
// gives that engine's planner its collator before it finishes the plan — the
// line `withCollator(planner, …)` of loader.js, which nothing else reaches.
// The expected order is the one PostgreSQL gave the conformance case.

const expected = JSON.parse(
  readFileSync(
    new URL(
      "../../crates/opengrid-conformance/collation-cases/s4a-root-orders-letters-before-case.json",
      import.meta.url,
    ),
  ),
).expected.rows.map((row) => row[0]);

const ids = (page) =>
  page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((row) => row.querySelector("td[data-row]"))
      .sort((a, b) => Number(a.getAttribute("aria-rowindex")) - Number(b.getAttribute("aria-rowindex")))
      .map((row) => Number(row.querySelectorAll("td")[1].textContent));
  });

async function open(page) {
  const modules = [];
  page.on("request", (request) => {
    if (request.url().includes("/collation/")) {
      modules.push(request.url());
    }
  });
  await page.goto("/tests/e2e/fixtures/grid-collation-hybrid.html");
  await page.waitForFunction(() => window.__ready === true);
  return modules;
}

test("a source without collations: the engine in the tab sorts by the locale", async ({ page }) => {
  const modules = await open(page);
  await expect.poll(() => ids(page)).toEqual(expected);

  // The plan says why: the sort is the client's, and the source was not asked
  // for it.
  const plan = await page.evaluate(() => window.__plans.at(-1));
  expect(plan.client?.sort?.[0]).toMatchObject({ field: "word", collation: "und" });
  expect(plan.source.sort ?? []).toEqual([]);

  // And the tab fetched the collation module, for its planner.
  expect(modules.length, "the tab loaded the collation module").toBeGreaterThan(0);
});
