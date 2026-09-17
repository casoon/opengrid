import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` (plan point 16).
//
// The fixture loads the real engine, attaches it as the provider and lets the
// element run the query. This spec proves the grid-mode contract: a native
// `<table role="grid">` with correct counts and 1-based `aria-rowindex`, exactly
// one roving `tabindex="0"`, and every key of the WAI-ARIA grid matrix
// (plan/spezifikation/09-accessibility.md §Tastatur im Grid Mode). The fixture
// dataset has five rows and the element a `page-size="2"`, so paging is
// observable: page 0 has rows 2–3, page 1 rows 4–5, page 2 row 6.
//
// Paging needs a total order (rule S6), so the grid starts sorted by its first
// column (`id`, ascending); the data is therefore in insertion order.

/** The inner focused element, or null. */
async function activeCell(page) {
  return page.evaluate(() => {
    const element = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!element) return null;
    return {
      tag: element.tagName.toLowerCase(),
      row: element.getAttribute("data-row"),
      col: element.getAttribute("data-col"),
      tabindex: element.getAttribute("tabindex"),
    };
  });
}

/** The rendered grid facts. */
async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const table = root.querySelector("table");
    const cells = [...root.querySelectorAll("th, td")];
    return {
      role: table.getAttribute("role"),
      ariaLabel: table.getAttribute("aria-label"),
      rowcount: table.getAttribute("aria-rowcount"),
      colcount: table.getAttribute("aria-colcount"),
      rowindexes: [...root.querySelectorAll("tr")].map((tr) =>
        tr.getAttribute("aria-rowindex"),
      ),
      tabindexes: cells.map((cell) => cell.getAttribute("tabindex")),
      zeroTabindex: cells.filter((cell) => cell.getAttribute("tabindex") === "0")
        .length,
      ariaSorts: [...root.querySelectorAll("thead th")].map((th) =>
        th.getAttribute("aria-sort"),
      ),
    };
  });
}

/** The `aria-sort` token of a column. */
async function ariaSortFor(page, column) {
  return page.evaluate((column) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return (
      root
        .querySelector(`th[data-col="${column}"]`)
        ?.getAttribute("aria-sort") ?? null
    );
  }, column);
}

/** The rendered text of one column, top to bottom. */
async function columnText(page, column) {
  return page.evaluate((column) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll(`tbody td:nth-child(${column + 1})`)].map(
      (cell) => cell.textContent,
    );
  }, column);
}

/** Focuses a cell inside the shadow root. */
async function focusCell(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("tbody tr");
  });
});

test("renders role=grid with correct counts, rowindexes and one tabindex=0", async ({
  page,
}) => {
  const rendered = await facts(page);
  expect(rendered).toMatchObject({
    role: "grid",
    ariaLabel: "Bestellungen",
    rowcount: "6",
    colcount: "4",
    rowindexes: ["1", "2", "3"],
    zeroTabindex: 1,
  });
  // The default sort (rule S6) is visible on the first column.
  expect(rendered.ariaSorts).toEqual(["ascending", "none", "none", "none"]);
  // Every cell is part of the roving tabindex: exactly one 0, the rest -1.
  expect(new Set(rendered.tabindexes)).toEqual(new Set(["0", "-1"]));
});

test("ArrowDown moves into the first data row and ArrowUp back into the header", async ({
  page,
}) => {
  await focusCell(page, 'th[data-col="0"]');

  await page.keyboard.press("ArrowDown");
  expect(await activeCell(page)).toMatchObject({ tag: "td", row: "0", col: "0" });

  await page.keyboard.press("ArrowUp");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "0" });
});

test("ArrowRight and ArrowLeft move by one column and clamp at the edges", async ({
  page,
}) => {
  await focusCell(page, 'th[data-col="0"]');

  await page.keyboard.press("ArrowRight");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "1" });

  await page.keyboard.press("ArrowLeft");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "0" });

  // Clamped: left at the first column stays, right at the last column stays.
  await page.keyboard.press("ArrowLeft");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "0" });

  await focusCell(page, 'th[data-col="3"]');
  await page.keyboard.press("ArrowRight");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "3" });
});

test("Home and End move to the first and last column of the row", async ({
  page,
}) => {
  await focusCell(page, 'td[data-row="0"][data-col="1"]');

  await page.keyboard.press("End");
  expect(await activeCell(page)).toMatchObject({ tag: "td", row: "0", col: "3" });

  await page.keyboard.press("Home");
  expect(await activeCell(page)).toMatchObject({ tag: "td", row: "0", col: "0" });
});

test("Ctrl+End jumps to the last cell and Ctrl+Home back to the first", async ({
  page,
}) => {
  await focusCell(page, 'th[data-col="0"]');

  await page.keyboard.press("Control+End");
  await expect.poll(() => activeCell(page)).toMatchObject({
    tag: "td",
    row: "4",
    col: "3",
  });
  // The last page was loaded: one row, row index 6.
  expect((await facts(page)).rowindexes).toEqual(["1", "6"]);

  await page.keyboard.press("Control+Home");
  await expect.poll(() => activeCell(page)).toMatchObject({ tag: "th", col: "0" });
  expect((await facts(page)).rowindexes).toEqual(["1", "2", "3"]);
});

test("PageDown and PageUp move by a page and load it", async ({ page }) => {
  await focusCell(page, 'th[data-col="0"]');

  await page.keyboard.press("PageDown");
  await expect.poll(() => facts(page).then((f) => f.rowindexes)).toEqual([
    "1",
    "4",
    "5",
  ]);
  expect(await activeCell(page)).toMatchObject({ tag: "td", row: "2" });

  await page.keyboard.press("PageDown");
  await expect.poll(() => facts(page).then((f) => f.rowindexes)).toEqual([
    "1",
    "6",
  ]);

  await page.keyboard.press("PageUp");
  await expect.poll(() => facts(page).then((f) => f.rowindexes)).toEqual([
    "1",
    "4",
    "5",
  ]);
});

test("Enter on a header toggles aria-sort and reorders the rows", async ({
  page,
}) => {
  // `customer` starts unsorted; the default `id` order is Gamma, Alpha.
  await focusCell(page, 'th[data-col="1"]');

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("ascending");
  await expect.poll(() => columnText(page, 1)).toEqual(["Alpha", "Alpha"]);

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("descending");
  await expect.poll(() => columnText(page, 1)).toEqual(["Gamma", "Beta"]);

  // Clearing the sort falls back to the default `id` ascending order.
  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("none");
  await expect.poll(() => ariaSortFor(page, 0)).toBe("ascending");
  await expect.poll(() => columnText(page, 1)).toEqual(["Gamma", "Alpha"]);
});

test("Space on a header toggles aria-sort", async ({ page }) => {
  await focusCell(page, 'th[data-col="1"]');

  await page.keyboard.press("Space");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("ascending");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "1" });
});

test("Escape returns focus to the first cell", async ({ page }) => {
  await focusCell(page, 'td[data-row="1"][data-col="3"]');

  await page.keyboard.press("Escape");
  expect(await activeCell(page)).toMatchObject({ tag: "th", col: "0" });
});

test("Tab leaves the grid forwards and Shift+Tab backwards", async ({ page }) => {
  await focusCell(page, 'th[data-col="0"]');
  await page.keyboard.press("Tab");
  expect(await page.evaluate(() => document.activeElement?.id)).toBe("after");

  await focusCell(page, 'th[data-col="0"]');
  await page.keyboard.press("Shift+Tab");
  expect(await page.evaluate(() => document.activeElement?.id)).toBe("before");
});

test("has no axe violations", async ({ page }) => {
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
