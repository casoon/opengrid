import { test, expect } from "@playwright/test";

// The query of a view without an element (issue #144): `Engine.view_query`
// answers what the grid's `get_query()` answers once the same view is applied
// to a grid with the same `columns`. One translation, two places it is asked.

const COLUMNS = ["id", "customer", "country", "amount", "qty"];

const VIEWS = {
  "an empty view": {},
  "a sort, a filter row and a moved, hidden column": {
    sort: [
      { field: "amount", direction: "desc" },
      { field: "id", direction: "asc" },
    ],
    filters: [
      { column: "country", op: "eq", value: "DE" },
      { column: "qty", op: "gte", value: "5" },
    ],
    columns: { order: ["amount", "id"], hidden: ["customer"] },
  },
  "facets beside the filter row": {
    filters: [{ column: "customer", op: "contains", value: "a" }],
    facets: { country: { values: ["FR", "GB"] }, qty: { min: "10", max: "100" } },
  },
  "a grouping": { group: ["country"], sort: [{ field: "qty", direction: "asc" }] },
  "a null test": { filters: [{ column: "country", op: "is_null", value: "" }] },
};

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-view.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
});

for (const [name, view] of Object.entries(VIEWS)) {
  test(`${name}: view_query is get_query`, async ({ page }) => {
    const [element, engine] = await page.evaluate(
      async ({ view, columns }) => {
        const host = document.querySelector("opengrid-grid");
        const module = window.__opengridModule;
        module.set_view(host, view);
        // Applied, and its query answered: the types are the result's.
        await new Promise((resolve) => setTimeout(resolve, 300));
        const fromEngine = JSON.parse(
          window.__engine.view_query("orders", JSON.stringify(columns), JSON.stringify(view)),
        );
        return [module.get_query(host), fromEngine];
      },
      { view, columns: COLUMNS },
    );
    expect(engine).toEqual(element);
  });
}

test("a view naming an unknown column is refused, every problem named", async ({ page }) => {
  const message = await page.evaluate((columns) => {
    try {
      window.__engine.view_query(
        "orders",
        JSON.stringify(columns),
        JSON.stringify({ sort: [{ field: "nope", direction: "asc" }], group: ["also_not"] }),
      );
      return null;
    } catch (error) {
      return String(error.message ?? error);
    }
  }, COLUMNS);
  expect(message).toContain("nope");
  expect(message).toContain("also_not");
});

test("a filter value that is not a value of its column is refused", async ({ page }) => {
  const message = await page.evaluate((columns) => {
    try {
      window.__engine.view_query(
        "orders",
        JSON.stringify(columns),
        JSON.stringify({ filters: [{ column: "qty", op: "gte", value: "many" }] }),
      );
      return null;
    } catch (error) {
      return String(error.message ?? error);
    }
  }, COLUMNS);
  expect(message).toContain("qty");
  expect(message).toContain("many");
});
