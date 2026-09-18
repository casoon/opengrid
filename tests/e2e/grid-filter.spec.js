import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` sort and filter UI (plan point 18).
//
// The same fixture as `grid.spec.js` (five rows, `window-size="10"`): a
// type-agnostic filter row above the `role="grid"` table, and multi-column
// sorting from the header cells. Everything is driven with the keyboard — the
// filter controls are ordinary focusables outside the grid, the header cells
// use the roving tabindex. The filter values are sent as plain strings (the
// display schema is all `Utf8` in Phase B), so the operator matrix runs against
// the text column `customer`.
//
// Operator options are ordered as in `grid::FILTER_OPERATORS`:
// contains(0), starts_with(1), eq(2), ne(3), gt(4), gte(5), lt(6), lte(7).

/** The filter row facts. */
async function filterFacts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const filter = root.querySelector('[part="filter"]');
    const labels = [...root.querySelectorAll("select[data-col], input[data-col]")].map(
      (control) => control.getAttribute("aria-label"),
    );
    return {
      present: !!filter,
      outsideGrid: !filter?.closest('table[role="grid"]'),
      operators: [...root.querySelectorAll("select[data-col]")].length,
      values: [...root.querySelectorAll("input[data-col]")].length,
      labels,
      clear: root.querySelector('[part="filter-clear"]')?.textContent ?? null,
      status: root.querySelector('[part="status"]')?.textContent ?? null,
    };
  });
}

/** The `role="status"` result count. */
async function statusText(page) {
  return page.evaluate(
    () =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector(
        '[part="status"]',
      ).textContent,
  );
}

/** Focuses an element inside the shadow root. */
async function focusIn(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

/** The inner focused element, or null. */
async function innerActive(page) {
  return page.evaluate(() => {
    const element =
      document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!element) return null;
    return {
      tag: element.tagName.toLowerCase(),
      col: element.getAttribute("data-col"),
      part: element.getAttribute("part"),
    };
  });
}

/** Selects an operator by index using only the keyboard (Home + ArrowDown). */
async function chooseOperator(page, column, index) {
  await focusIn(page, `select[data-col="${column}"]`);
  await page.keyboard.press("Home");
  for (let step = 0; step < index; step += 1) {
    await page.keyboard.press("ArrowDown");
  }
}

/** Replaces the value input's text and applies the filter with Enter. */
async function applyFilter(page, column, value) {
  await focusIn(page, `input[data-col="${column}"]`);
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(value);
  await page.keyboard.press("Enter");
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

/** The `aria-sort` tokens of the header cells. */
async function ariaSorts(page) {
  return page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("thead th")].map(
      (th) => th.getAttribute("aria-sort"),
    ),
  );
}

/** The visible sort direction glyph of a column (empty when unsorted). */
async function sortDirection(page, column) {
  return page.evaluate(
    (column) =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`th[data-col="${column}"] [part="sort-direction"]`)
        .textContent,
    column,
  );
}

/** The visible multi-sort order index of a column (empty when unsorted/single). */
async function sortIndex(page, column) {
  return page.evaluate(
    (column) =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`th[data-col="${column}"] [part="sort-index"]`)
        .textContent,
    column,
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("renders a labelled, keyboard-focusable filter row above the grid", async ({
  page,
}) => {
  const rendered = await filterFacts(page);
  expect(rendered.present).toBe(true);
  expect(rendered.outsideGrid).toBe(true);
  expect(rendered.operators).toBe(4);
  expect(rendered.values).toBe(4);
  expect(rendered.labels).toContain("id operator");
  expect(rendered.labels).toContain("id value");
  expect(rendered.labels).toContain("customer operator");
  expect(rendered.clear).toBe("Clear");
  // The count comes from `total_count` (all five rows).
  expect(rendered.status).toBe("5 matches");

  // The controls are ordinary focusables reached with Tab, not part of the
  // roving-tabindex grid.
  await focusIn(page, 'select[data-col="0"]');
  expect(await innerActive(page)).toMatchObject({ tag: "select", part: "filter-operator" });
});

test("filters by every operator with the keyboard", async ({ page }) => {
  // contains(0)
  await chooseOperator(page, 1, 0);
  await applyFilter(page, 1, "l");
  await expect.poll(() => statusText(page)).toBe("2 matches");
  expect(await columnText(page, 1)).toEqual(["Alpha", "Alpha"]);

  // starts_with(1)
  await chooseOperator(page, 1, 1);
  await applyFilter(page, 1, "A");
  await expect.poll(() => statusText(page)).toBe("2 matches");
  expect(await columnText(page, 1)).toEqual(["Alpha", "Alpha"]);

  // eq(2)
  await chooseOperator(page, 1, 2);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("2 matches");
  expect(await columnText(page, 1)).toEqual(["Beta", "Beta"]);

  // ne(3)
  await chooseOperator(page, 1, 3);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("3 matches");
  expect(await columnText(page, 1)).toEqual(["Gamma", "Alpha", "Alpha"]);

  // gt(4)
  await chooseOperator(page, 1, 4);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("1 match");
  expect(await columnText(page, 1)).toEqual(["Gamma"]);

  // gte(5)
  await chooseOperator(page, 1, 5);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("3 matches");
  expect(await columnText(page, 1)).toEqual(["Gamma", "Beta", "Beta"]);

  // lt(6)
  await chooseOperator(page, 1, 6);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("2 matches");
  expect(await columnText(page, 1)).toEqual(["Alpha", "Alpha"]);

  // lte(7)
  await chooseOperator(page, 1, 7);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("4 matches");
  expect(await columnText(page, 1)).toEqual(["Alpha", "Beta", "Alpha", "Beta"]);
});

test("the value input keeps the focus after Enter", async ({ page }) => {
  await chooseOperator(page, 1, 0);
  await applyFilter(page, 1, "Al");
  await expect.poll(() => statusText(page)).toBe("2 matches");
  expect(await innerActive(page)).toMatchObject({ tag: "input", col: "1" });
});

test("the Clear button resets the filter and the count", async ({ page }) => {
  await chooseOperator(page, 1, 2);
  await applyFilter(page, 1, "Beta");
  await expect.poll(() => statusText(page)).toBe("2 matches");

  await focusIn(page, '[part="filter-clear"]');
  await page.keyboard.press("Enter");
  await expect.poll(() => statusText(page)).toBe("5 matches");
  expect(await columnText(page, 1)).toEqual([
    "Gamma",
    "Alpha",
    "Beta",
    "Alpha",
    "Beta",
  ]);
});

test("Shift+Enter from a header adds and removes an additional sort key", async ({
  page,
}) => {
  // A plain Enter on `customer` makes it the single sort key.
  await focusIn(page, 'th[data-col="1"]');
  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSorts(page)).toEqual([
    "none",
    "ascending",
    "none",
    "none",
  ]);

  // Shift+Enter on `amount` appends it as the second key (order preserved).
  await focusIn(page, 'th[data-col="2"]');
  await page.keyboard.press("Shift+Enter");
  await expect.poll(() => ariaSorts(page)).toEqual([
    "none",
    "ascending",
    "ascending",
    "none",
  ]);
  await expect.poll(() => sortIndex(page, 1)).toBe("1");
  await expect.poll(() => sortIndex(page, 2)).toBe("2");
  // Customer ascending, then amount ascending.
  expect(await columnText(page, 1)).toEqual([
    "Alpha",
    "Alpha",
    "Beta",
    "Beta",
    "Gamma",
  ]);
  expect(await columnText(page, 2)).toEqual(["20.00", "40.00", "5.00", "10.00", "30.00"]);
  // The focus stays on the activated header.
  expect(await innerActive(page)).toMatchObject({ tag: "th", col: "2" });

  // A second Shift+Enter toggles the appended key to descending.
  await page.keyboard.press("Shift+Enter");
  await expect.poll(() => ariaSorts(page)).toEqual([
    "none",
    "ascending",
    "descending",
    "none",
  ]);
  await expect.poll(() => sortIndex(page, 2)).toBe("2");
  expect(await columnText(page, 2)).toEqual(["40.00", "20.00", "10.00", "5.00", "30.00"]);

  // A third Shift+Enter removes it again; only the primary key remains.
  await page.keyboard.press("Shift+Enter");
  await expect.poll(() => ariaSorts(page)).toEqual([
    "none",
    "ascending",
    "none",
    "none",
  ]);
  expect(await sortIndex(page, 1)).toBe("");
  expect(await sortIndex(page, 2)).toBe("");

  // A plain Enter replaces the whole sort with this single column.
  await focusIn(page, 'th[data-col="2"]');
  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSorts(page)).toEqual([
    "none",
    "none",
    "ascending",
    "none",
  ]);
  expect(await columnText(page, 2)).toEqual(["5.00", "10.00", "20.00", "30.00", "40.00"]);
});

test("the header shows the sort direction, not only aria-sort", async ({ page }) => {
  // Plan point 49: before it, a sighted user could not tell ascending from
  // descending — the direction lived in `aria-sort` alone (WCAG 1.3.3).
  // The grid starts sorted by its first column (rule S6).
  await expect.poll(() => ariaSorts(page)).toEqual(["ascending", "none", "none", "none"]);
  expect(await sortDirection(page, 0)).toBe("▲");
  expect(await sortDirection(page, 1)).toBe("");

  await focusIn(page, 'th[data-col="1"]');
  await page.keyboard.press("Enter");
  await expect.poll(() => sortDirection(page, 1)).toBe("▲");
  expect(await sortDirection(page, 0)).toBe("");

  await page.keyboard.press("Enter");
  await expect.poll(() => ariaSorts(page).then((sorts) => sorts[1])).toBe("descending");
  expect(await sortDirection(page, 1)).toBe("▼");

  // Multi-sort: every key shows its own direction next to its order index.
  await focusIn(page, 'th[data-col="2"]');
  await page.keyboard.press("Shift+Enter");
  await expect.poll(() => sortDirection(page, 2)).toBe("▲");
  expect(await sortDirection(page, 1)).toBe("▼");
  expect(await sortIndex(page, 1)).toBe("1");
  expect(await sortIndex(page, 2)).toBe("2");

  // Each activation re-runs the query, so wait for one before sending the next.
  await page.keyboard.press("Shift+Enter");
  await expect.poll(() => sortDirection(page, 2)).toBe("▼");
  await page.keyboard.press("Shift+Enter");
  // The key is gone: neither direction nor order index is left behind.
  await expect.poll(() => sortDirection(page, 2)).toBe("");
  expect(await sortIndex(page, 2)).toBe("");
  expect(await sortIndex(page, 1)).toBe("");
});

test("the sort marks stay out of the header's accessible name", async ({ page }) => {
  // The glyph is decoration for the eye; the direction reaches assistive
  // technology through `aria-sort`, and must not be announced a second time as
  // part of the column's name. Driven into the **multi-sort** state, because
  // that is the only one where the order index is non-empty too.
  await focusIn(page, 'th[data-col="1"]');
  await page.keyboard.press("Enter");
  await expect.poll(() => sortDirection(page, 1)).toBe("▲");
  await page.keyboard.press("Enter");
  await expect.poll(() => sortDirection(page, 1)).toBe("▼");
  await focusIn(page, 'th[data-col="2"]');
  await page.keyboard.press("Shift+Enter");
  await expect.poll(() => sortIndex(page, 1)).toBe("1");
  expect(await sortDirection(page, 2)).toBe("▲");

  for (const [column, name] of [
    [1, "customer"],
    [2, "amount"],
  ]) {
    const header = page.locator("opengrid-grid").locator(`th[data-col="${column}"]`);
    await expect(header).toHaveAccessibleName(name);
  }
  expect(
    await page.evaluate(() =>
      [
        ...document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelectorAll('th[data-col="1"] span'),
      ]
        .filter((span) => span.getAttribute("part")?.startsWith("sort-"))
        .map((span) => `${span.getAttribute("part")}:${span.getAttribute("aria-hidden")}`),
    ),
  ).toEqual(["sort-direction:true", "sort-index:true"]);
});

test("has no axe violations with the filter row rendered", async ({ page }) => {
  await chooseOperator(page, 1, 0);
  await applyFilter(page, 1, "Al");
  await expect.poll(() => statusText(page)).toBe("2 matches");
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});