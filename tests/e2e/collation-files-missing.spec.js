import { test, expect } from "@playwright/test";

// Issue #415: a page that serves the package's files itself — the live demos do, by a list of
// files — and leaves `collation.js` and `collation/` out. Since 0.17.0 `loader.js` and
// `worker.js` imported `collation.js` statically, so the loader failed to load or the worker
// died without a word and the grid never showed a row. Now the file is fetched by the first
// query that names a locale, and by no other.

const query = (collation) =>
  JSON.stringify({
    source: "orders",
    select: ["customer"],
    sort: [{ field: "customer", ...(collation ? { collation } : {}) }],
    limit: 3,
  });

async function openWithoutCollation(page) {
  const asked = [];
  await page.route(/\/packages\/opengrid\/collation(\.js|\/)/, (route) => {
    asked.push(route.request().url());
    return route.fulfill({ status: 404, body: "not found" });
  });
  await page.goto("/tests/e2e/fixtures/worker-grid.html");
  await page.waitForFunction(() => window.__provider !== undefined);
  return asked;
}

test("without collation.js the worker and the grid work, and nothing asks for it", async ({ page }) => {
  const asked = await openWithoutCollation(page);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  // A query with no locale — and one with `binary` — is answered by the worker as ever.
  for (const collation of [undefined, "binary"]) {
    const answer = await page.evaluate((json) => window.__provider.execute(json).then((r) => r.byteLength ?? r.length), query(collation));
    expect(answer).toBeGreaterThan(0);
  }
  expect(asked, "no query without a locale fetches the collation files").toEqual([]);
});

test("without collation.js a sort by a locale says what is missing", async ({ page }) => {
  const asked = await openWithoutCollation(page);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  const message = await page.evaluate(
    (json) => window.__provider.execute(json).then(() => "answered", (error) => String(error?.message ?? error)),
    query("de"),
  );
  expect(message).toContain("collation.js");
  expect(asked.length, "the file was asked for, by that query").toBeGreaterThan(0);
  // And the grid still answers the next query without a locale.
  const again = await page.evaluate((json) => window.__provider.execute(json).then((r) => r.byteLength ?? r.length), query());
  expect(again).toBeGreaterThan(0);
});
