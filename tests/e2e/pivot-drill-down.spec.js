import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The rows behind a pivot's cell (issue #330), on the demo page itself —
// examples/pivot-demo/browser.html is the pattern a page copies, so it is what
// has to be right: the pivot fires `opengrid-drill-down` with the cell's
// filter, the page shows the rows in a grid.
//
// The claim that matters is one number: the grid's matches are the cell's
// count. It is asked of **every** count cell of the pivot, and the cells are
// sorted into the kinds the filter has to get right — an ordinary group, the
// NULL group, the empty-string group (S14: not the NULL group), a subtotal and
// the grand total — so that none of them can go untested unnoticed. The
// conformance dataset has all of them: 7 orders without a country, 3 with "".

const GRID = "#behind opengrid-grid";

async function open(page) {
  await page.goto("/examples/pivot-demo/browser.html");
  await expect(page.locator('opengrid-pivot button[part="drill-down"]').first()).toBeVisible();
  await page.evaluate((grid) => {
    window.__drills = [];
    window.__totals = [];
    document
      .querySelector("opengrid-pivot")
      .addEventListener("opengrid-drill-down", (event) => window.__drills.push(event.detail));
    document
      .querySelector(grid)
      .addEventListener("opengrid-query", (event) => window.__totals.push(event.detail.total));
  }, GRID);
}

/** The `data-cell` of every value button the pivot shows. */
const cells = (page) =>
  page.evaluate(() =>
    [...document.querySelector("opengrid-pivot").shadowRoot.querySelectorAll("button[data-cell]")].map(
      (button) => button.getAttribute("data-cell"),
    ),
  );

/**
 * Presses the value at `cell` and waits for the grid's answer: the event's
 * detail, and how many rows the grid then says it has.
 */
async function drill(page, cell) {
  const before = await page.evaluate(() => window.__totals.length);
  await page.locator(`opengrid-pivot button[data-cell="${cell}"]`).click();
  await page.waitForFunction((before) => window.__totals.length > before, before);
  await expect(page.locator(`${GRID} [part="status"]`)).toHaveAttribute("data-state", /ready|empty/);
  return page.evaluate(() => ({ detail: window.__drills.at(-1), total: window.__totals.at(-1) }));
}

/** Drills into every count cell and requires the grid's rows to be its count. */
async function everyCountCell(page) {
  const seen = [];
  for (const cell of await cells(page)) {
    const { detail, total } = await drill(page, cell);
    if (detail.measure !== "Count") continue;
    expect(total, `cell ${cell}: ${JSON.stringify(detail.query.filter)}`).toBe(detail.value);
    seen.push(detail);
  }
  return seen;
}

const eq = (field, value) => ({ field, op: "eq", value });
const isNull = (field) => ({ field, op: "is_null" });

test.describe("pivot drill-down", () => {
  test("the rows behind every cell are as many as its count", async ({ page }) => {
    await open(page);
    const seen = await everyCountCell(page);

    // Every kind of cell was among them, with the filter that kind needs.
    const of = (key) => seen.filter(({ row }) => row.length === 1 && row[0] === key);
    expect(of("DE").length).toBeGreaterThan(0);
    for (const { query, column } of of("DE")) {
      expect(query).toEqual({
        source: "orders",
        filter: { and: [eq("country", "DE"), column[0] === null ? isNull("ordered_year") : eq("ordered_year", column[0])] },
      });
    }
    // NULL is `is_null` — `eq null` would match nothing — and the empty
    // string is a value of its own.
    expect(of(null).length).toBeGreaterThan(0);
    for (const { query } of of(null)) {
      expect(query.filter.and[0]).toEqual(isNull("country"));
    }
    expect(of("").length).toBeGreaterThan(0);
    for (const { query } of of("")) {
      expect(query.filter.and[0]).toEqual(eq("country", ""));
    }
    // The grand total lies in no row group: its filter is its column alone.
    const totals = seen.filter(({ row }) => row.length === 0);
    expect(totals.length).toBeGreaterThan(0);
    for (const { query } of totals) {
      expect(query.filter.field).toBe("ordered_year");
    }
    // The two groups are different rows: 7 orders have no country, 3 have "".
    const sum = (details) => details.reduce((all, { value }) => all + value, 0);
    expect(sum(of(null))).toBe(7);
    expect(sum(of(""))).toBe(3);
    expect(sum(totals)).toBe(50);
  });

  test("subtotals name their group alone, and the pivot's own filter leads", async ({ page }) => {
    await open(page);
    const own = { or: [{ field: "qty", op: "gte", value: 3 }, isNull("note")] };
    await page.evaluate((own) => {
      const pivot = document.querySelector("opengrid-pivot");
      pivot.removeAttribute("columns");
      pivot.setAttribute("rows", "country,customer");
      pivot.setAttribute("filter", JSON.stringify(own));
    }, own);
    await expect(page.locator('opengrid-pivot tbody tr[data-level="2"]').first()).toBeVisible();
    await expect(page.locator('opengrid-pivot [part="status"]')).toHaveAttribute("data-state", "ready");

    const seen = await everyCountCell(page);
    const level = (length) => seen.filter(({ row }) => row.length === length);
    expect(level(2).length).toBeGreaterThan(0);
    expect(level(1).length).toBeGreaterThan(0);
    for (const { query, row } of level(2)) {
      expect(query.filter.and).toHaveLength(3);
      expect(query.filter.and[0]).toEqual(own);
      expect(query.filter.and[1]).toEqual(row[0] === null ? isNull("country") : eq("country", row[0]));
    }
    // A subtotal: the pivot's filter and its country — nothing about the
    // customers it sums over. The NULL country's subtotal among them.
    for (const { query, row } of level(1)) {
      expect(query.filter.and).toEqual([own, row[0] === null ? isNull("country") : eq("country", row[0])]);
    }
    expect(level(1).some(({ row }) => row[0] === null)).toBe(true);
    expect(level(1).some(({ row }) => row[0] === "")).toBe(true);
    // The grand total: the pivot's filter, whole and alone.
    expect(level(0)).toHaveLength(1);
    expect(level(0)[0].query).toEqual({ source: "orders", filter: own });
  });

  test("from the keyboard: Enter shows the rows, and the way back ends on the cell", async ({ page }) => {
    await open(page);
    const [first, second] = await cells(page);
    const button = page.locator(`opengrid-pivot button[data-cell="${second}"]`);
    // Named by what it shows and what it does (WCAG 2.5.3, 4.1.2).
    const text = await button.textContent();
    await expect(button).toHaveAttribute("aria-label", `${text}, show rows`);
    expect(first).not.toBe(second);

    await button.focus();
    await page.keyboard.press("Enter");
    await expect(page.locator("#behind")).toBeVisible();
    // The focus is on the heading that names the cell ...
    await expect(page.locator("#behind-title")).toBeFocused();
    await expect(page.locator("#behind-title")).toHaveText(/^Rows behind (Quantity|Count) of /);
    await expect(page.locator(`${GRID} [part="status"]`)).toHaveAttribute("data-state", "ready");
    // ... the next stop is the way back, and it ends on the cell pressed.
    await page.keyboard.press("Tab");
    await expect(page.locator("#back")).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.locator("#behind")).toBeHidden();
    await expect(button).toBeFocused();

    // Space presses it as well: it is a native button.
    const before = await page.evaluate(() => window.__drills.length);
    await page.keyboard.press("Space");
    await expect.poll(() => page.evaluate(() => window.__drills.length)).toBe(before + 1);
  });

  test("a clicked value has the focus, and after a new answer the way back still ends in the pivot", async ({
    page,
  }) => {
    await open(page);
    const [, , third] = await cells(page);
    await drill(page, third);
    // The pivot was the last thing focused before the page moved the focus.
    await expect(page.locator("#behind-title")).toBeFocused();
    // A new answer: every button of the pivot is a new node.
    await page.evaluate(() =>
      document
        .querySelector("opengrid-pivot")
        .setAttribute("sort", '[{"field":"country","direction":"desc"}]'),
    );
    await expect(page.locator('opengrid-pivot th[aria-sort="descending"]')).toBeVisible();
    await page.locator("#back").click();
    const focused = await page.evaluate(() =>
      document.querySelector("opengrid-pivot").shadowRoot.activeElement?.getAttribute("part"),
    );
    expect(focused).toBe("drill-down");
  });

  test("the event bubbles, is composed and not cancelable, and the pressed value has the focus", async ({
    page,
  }) => {
    await open(page);
    await page.evaluate(() => {
      // Capturing, so it hears the event before the page's own listener
      // moves the focus on.
      document.addEventListener(
        "opengrid-drill-down",
        (event) => {
          window.__event = {
            bubbles: event.bubbles,
            composed: event.composed,
            cancelable: event.cancelable,
            target: event.target.tagName.toLowerCase(),
            focused: event.target.shadowRoot.activeElement?.getAttribute("data-cell"),
          };
        },
        true,
      );
    });
    const [cell] = await cells(page);
    await page.locator(`opengrid-pivot button[data-cell="${cell}"]`).click();
    expect(await page.evaluate(() => window.__event)).toEqual({
      bubbles: true,
      composed: true,
      cancelable: false,
      target: "opengrid-pivot",
      focused: cell,
    });
  });

  test("without the attribute a value is text, and setting it asks nothing", async ({ page }) => {
    await open(page);
    const queries = await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      window.__pivotQueries = 0;
      pivot.addEventListener("opengrid-query", () => (window.__pivotQueries += 1));
      pivot.removeAttribute("drill-down");
      return window.__pivotQueries;
    });
    await expect(page.locator("opengrid-pivot tbody td").first()).toBeVisible();
    await expect(page.locator("opengrid-pivot tbody button")).toHaveCount(0);
    await page.evaluate(() => document.querySelector("opengrid-pivot").setAttribute("drill-down", ""));
    await expect(page.locator('opengrid-pivot button[part="drill-down"]').first()).toBeVisible();
    // A cell without a value has no button: nothing to show, nothing to name.
    const { empty, buttons } = await page.evaluate(() => {
      const cells = [...document.querySelector("opengrid-pivot").shadowRoot.querySelectorAll("tbody td")];
      return {
        empty: cells.filter((td) => td.textContent === "" && td.querySelector("button")).length,
        buttons: cells.filter((td) => td.querySelector("button")).length,
      };
    });
    expect(empty).toBe(0);
    expect(buttons).toBeGreaterThan(0);
    expect(await page.evaluate(() => window.__pivotQueries)).toBe(queries);
  });

  test("the pivot with its buttons and the rows behind a cell have no axe violations", async ({ page }) => {
    await open(page);
    const [cell] = await cells(page);
    await drill(page, cell);
    const { violations } = await new AxeBuilder({ page })
      .include("opengrid-pivot")
      .include("#behind")
      .analyze();
    expect(violations).toEqual([]);
  });
});
