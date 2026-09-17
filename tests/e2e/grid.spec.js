import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` (plan points 16/17).
//
// The fixture loads the real engine, attaches it as the provider and lets the
// element run the query. This spec proves the grid-mode contract: a native
// `<table role="grid">` with correct counts and 1-based `aria-rowindex`, exactly
// one roving `tabindex="0"`, and every key of the WAI-ARIA grid matrix
// (plan/spezifikation/09-accessibility.md §Tastatur im Grid Mode). The fixture
// dataset has five rows and the element a `window-size="10"` pool, so all rows
// fit the window; the virtualizing behaviour of point 17 is covered by
// `grid-virtual.spec.js`.
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

/** The header row plus the assigned (visible) data rows' `aria-rowindex`. */
async function rowindexes(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const header = root.querySelector("thead tr")?.getAttribute("aria-rowindex");
    const data = [...root.querySelectorAll("tbody tr")]
      .map((tr) => tr.getAttribute("aria-rowindex"))
      .filter((value) => value);
    return [header, ...data];
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

/** The rendered text of one column, top to bottom (assigned rows only). */
async function columnText(page, column) {
  return page.evaluate((column) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => tr.querySelectorAll("td")[column].textContent);
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
    return !!root?.querySelector("td[data-row]");
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
    zeroTabindex: 1,
  });
  expect(await rowindexes(page)).toEqual(["1", "2", "3", "4", "5", "6"]);
  // The default sort (rule S6) is visible on the first column.
  expect(rendered.ariaSorts).toEqual(["ascending", "none", "none", "none"]);
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
  expect(await rowindexes(page)).toEqual(["1", "2", "3", "4", "5", "6"]);

  await page.keyboard.press("Control+Home");
  await expect.poll(() => activeCell(page)).toMatchObject({ tag: "th", col: "0" });
  expect(await rowindexes(page)).toEqual(["1", "2", "3", "4", "5", "6"]);
});

test("PageDown and PageUp move by a viewport and clamp to the result", async ({
  page,
}) => {
  // The 160px viewport shows five 32px rows, so PageDown from the header lands
  // on the last row and PageUp returns to the first.
  await focusCell(page, 'th[data-col="0"]');

  await page.keyboard.press("PageDown");
  await expect.poll(() => activeCell(page)).toMatchObject({ tag: "td", row: "4" });

  await page.keyboard.press("PageUp");
  await expect.poll(() => activeCell(page)).toMatchObject({ tag: "td", row: "0" });
});

test("Enter on a header toggles aria-sort and reorders the rows", async ({
  page,
}) => {
  // `customer` starts unsorted; the default `id` order is Gamma…Beta.
  await focusCell(page, 'th[data-col="1"]');

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("ascending");
  await expect
    .poll(() => columnText(page, 1))
    .toEqual(["Alpha", "Alpha", "Beta", "Beta", "Gamma"]);

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("descending");
  await expect
    .poll(() => columnText(page, 1))
    .toEqual(["Gamma", "Beta", "Beta", "Alpha", "Alpha"]);

  // Clearing the sort falls back to the default `id` ascending order.
  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSortFor(page, 1)).toBe("none");
  await expect.poll(() => ariaSortFor(page, 0)).toBe("ascending");
  await expect
    .poll(() => columnText(page, 1))
    .toEqual(["Gamma", "Alpha", "Beta", "Alpha", "Beta"]);
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
