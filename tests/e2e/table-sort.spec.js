import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-table>` with data (plan point 14).
//
// The fixture loads the real engine, attaches it as the provider and lets the
// element run the query. This spec proves the table mode contract: a native
// `<table>`, `aria-sort` transitions driven by the keyboard, reordered rows and
// no axe violations.

async function shadowFacts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    return [...root.querySelectorAll("thead th")].map((th) => ({
      column: th.getAttribute("data-column"),
      scope: th.getAttribute("scope"),
      ariaSort: th.getAttribute("aria-sort"),
      // The first span is the column name; the second is the sort mark, which
      // would otherwise be concatenated into this by `textContent`.
      buttonText: th.querySelector("button > span")?.textContent ?? null,
    }));
  });
}

async function ariaSortFor(page, column) {
  return page.evaluate((column) => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    return (
      root
        .querySelector(`th[data-column="${column}"]`)
        ?.getAttribute("aria-sort") ?? null
    );
  }, column);
}

/** The visible sort mark of a column's header (empty when unsorted). */
async function sortMark(page, column) {
  return page.evaluate(
    (column) =>
      document
        .querySelector("opengrid-table")
        .shadowRoot.querySelector(
          `th[data-column="${column}"] [part="sort-direction"]`,
        )
        ?.textContent ?? null,
    column,
  );
}

async function firstColumn(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    return [...root.querySelectorAll("tbody tr td:first-child")].map(
      (td) => td.textContent,
    );
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/table-data.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-table")?.shadowRoot;
    return !!root?.querySelector("tbody tr");
  });
});

test("renders a native table with scoped sort buttons", async ({ page }) => {
  const facts = await page.evaluate(() => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    return {
      mode: root.mode,
      table: !!root.querySelector("table"),
      caption: root.querySelector("caption")?.textContent ?? null,
      rows: root.querySelectorAll("tbody tr").length,
    };
  });
  expect(facts).toEqual({
    mode: "open",
    table: true,
    caption: "Bestellungen",
    rows: 5,
  });

  expect(await shadowFacts(page)).toEqual([
    { column: "customer", scope: "col", ariaSort: "none", buttonText: "customer" },
    { column: "amount", scope: "col", ariaSort: "none", buttonText: "amount" },
    { column: "qty", scope: "col", ariaSort: "none", buttonText: "qty" },
  ]);
});

test("keyboard toggles aria-sort and reorders the rows", async ({ page }) => {
  const button = page
    .locator("opengrid-table")
    .locator('button[data-column="customer"]');
  await button.focus();

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("ascending");
  await expect
    .poll(() => firstColumn(page))
    .toEqual(["Alpha", "Alpha", "Beta", "Beta", "Gamma"]);

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("descending");
  await expect
    .poll(() => firstColumn(page))
    .toEqual(["Gamma", "Beta", "Beta", "Alpha", "Alpha"]);

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("none");
});

test("sorting a second column clears the first one", async ({ page }) => {
  const table = page.locator("opengrid-table");
  await table.locator('button[data-column="customer"]').click();
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("ascending");

  await table.locator('button[data-column="qty"]').click();
  await expect.poll(() => ariaSortFor(page, "qty")).toBe("ascending");
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("none");
});

test("the header shows its sort direction, not only aria-sort", async ({
  page,
}) => {
  // Point 50: table mode had the same gap the grid had before point 49 — the
  // direction lived in `aria-sort` alone, so a sighted user could not see it
  // (WCAG 1.3.3). The glyphs are the grid's.
  const table = page.locator("opengrid-table");
  expect(await sortMark(page, "customer")).toBe("");

  await table.locator('button[data-column="customer"]').click();
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("ascending");
  expect(await sortMark(page, "customer")).toBe("\u00a0▲");
  expect(await sortMark(page, "qty")).toBe("");

  await table.locator('button[data-column="customer"]').click();
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("descending");
  expect(await sortMark(page, "customer")).toBe("\u00a0▼");

  // A third activation clears the sort, and with it the mark.
  await table.locator('button[data-column="customer"]').click();
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("none");
  expect(await sortMark(page, "customer")).toBe("");
});

test("the sort mark stays out of the button's accessible name", async ({
  page,
}) => {
  // The glyph is decoration; the direction is announced once, through
  // `aria-sort` on the `<th>`.
  const button = page
    .locator("opengrid-table")
    .locator('button[data-column="customer"]');
  await button.click();
  await expect.poll(() => ariaSortFor(page, "customer")).toBe("ascending");

  await expect(button).toHaveAccessibleName("customer");
  expect(
    await page.evaluate(() =>
      document
        .querySelector("opengrid-table")
        .shadowRoot.querySelector(
          'th[data-column="customer"] [part="sort-direction"]',
        )
        .getAttribute("aria-hidden"),
    ),
  ).toBe("true");
});

test("a late answer to an earlier query is not drawn over the newest", async ({ page }) => {
  // Two queries in a row, and the answer to the first comes last. The table has
  // to show the second one's columns, not the ones it was asked for before.
  const settled = await page.evaluate(async () => {
    const { loadOpengrid } = await import("/packages/opengrid/loader.js");
    const { module } = await loadOpengrid();
    const table = document.querySelector("opengrid-table");
    let calls = 0;
    const settled = [];
    module.set_provider(table, {
      async execute(json) {
        const call = ++calls;
        const { select } = JSON.parse(json);
        if (call === 1) await new Promise((resolve) => setTimeout(resolve, 500));
        settled.push(call);
        return JSON.stringify({
          total_count: 1,
          row_count: 1,
          columns: select.map((name) => ({ name, type: "utf8", nullable: true, values: [name] })),
        });
      },
    });
    table.setAttribute("columns", "customer");
    await new Promise((resolve) => setTimeout(resolve, 1200));
    return settled;
  });
  expect(settled).toEqual([2, 1]);
  const cells = await page.evaluate(() =>
    [...document.querySelector("opengrid-table").shadowRoot.querySelectorAll("tbody tr:first-child > *")].map(
      (cell) => cell.textContent,
    ),
  );
  expect(cells).toEqual(["customer"]);
});

test("has no axe violations", async ({ page }) => {
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
