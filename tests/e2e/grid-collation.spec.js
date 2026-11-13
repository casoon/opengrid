import { readFileSync } from "node:fs";

import { test, expect } from "@playwright/test";

// Text sorted by a locale (issue #391, rule S4a): the grid's `collation`
// attribute, the collation module the providers load, and the engine in the
// tab and in a worker. What the rows must be is not written here: it is the
// order PostgreSQL gave the conformance cases, so the browser is checked
// against a database.

const expected = (name) =>
  JSON.parse(
    readFileSync(
      new URL(`../../crates/opengrid-conformance/collation-cases/${name}.json`, import.meta.url),
    ),
  ).expected.rows.map((row) => row[0]);

/** The ids of a grid's rows, in the order they are shown. */
const ids = (page, grid) =>
  page.evaluate((grid) => {
    const root = document.querySelector(grid).shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((row) => row.querySelector("td[data-row]"))
      .sort((a, b) => Number(a.getAttribute("aria-rowindex")) - Number(b.getAttribute("aria-rowindex")))
      .map((row) => Number(row.querySelectorAll("td")[1].textContent));
  }, grid);

const status = (page, grid) =>
  page.evaluate((grid) => document.querySelector(grid).shadowRoot.querySelector('[part="status"]').textContent, grid);

async function open(page) {
  const modules = [];
  page.on("request", (request) => {
    if (request.url().includes("/collation/")) {
      modules.push(request.url());
    }
  });
  await page.goto("/tests/e2e/fixtures/grid-collation.html");
  await page.waitForFunction(() => window.__ready === true);
  return modules;
}

test("without the attribute text sorts by code point, and no module is fetched for it", async ({ page }) => {
  const modules = await open(page);
  await expect.poll(() => ids(page, "#local")).toEqual(expected("s4a-binary-stays-code-point-order"));
  // The worker's grid names a locale, so the module is asked for — by the
  // worker. The tab's engine has not needed it.
  await expect.poll(() => ids(page, "#worker")).toHaveLength(40);
  const tab = await page.evaluate(() =>
    performance.getEntriesByType("resource").filter((entry) => entry.name.includes("/collation/")).length,
  );
  expect(tab, "the tab fetched no collation module").toBe(0);
  expect(modules.length, "the worker fetched it").toBeGreaterThan(0);
});

test("a grid in a worker sorts by its locale as PostgreSQL does", async ({ page }) => {
  await open(page);
  await expect.poll(() => ids(page, "#worker")).toEqual(expected("s4a-a-script-is-a-locale"));
});

test("the attribute sorts the tab's grid by the locale, and a new value sorts anew", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector("#local").setAttribute("collation", "und"));
  await expect.poll(() => ids(page, "#local")).toEqual(expected("s4a-root-orders-letters-before-case"));

  await page.evaluate(() => document.querySelector("#local").setAttribute("collation", "de"));
  await expect
    .poll(async () => (await ids(page, "#local")).slice(10, 18))
    .toEqual(expected("s4a-a-page-of-a-collated-order"));

  await page.evaluate(() => document.querySelector("#local").removeAttribute("collation"));
  await expect.poll(() => ids(page, "#local")).toEqual(expected("s4a-binary-stays-code-point-order"));
});

test("the query the grid answers for carries the collation", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector("#local").setAttribute("collation", "de"));
  await expect.poll(() => status(page, "#local")).not.toBe("");
  const query = await page.evaluate(() => window.__module.get_query(document.querySelector("#local")));
  expect(query.sort).toEqual([{ field: "word", direction: "asc", collation: "de" }]);

});

test("a collation that is not a locale is said, not guessed", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector("#local").setAttribute("collation", "German"));
  await expect.poll(() => status(page, "#local")).toContain("the collation attribute is not a locale");
});
