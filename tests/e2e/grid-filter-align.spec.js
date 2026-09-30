// The filter row stands under the columns and scrolls with them (issue #62).
//
// Its groups used to sit at fixed 248 px steps, whatever the columns' widths,
// and the row scrolled on its own: at a phone's width the fields were under
// the wrong columns. Now each group is as wide as its column, starts where it
// starts, and one horizontal position is shared by the rows and the filter.
import { test, expect } from "@playwright/test";

const layout = (page) =>
  page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const viewport = root.querySelector('[part~="viewport"]');
    const filter = root.querySelector('[part="filter"]');
    return {
      viewport: viewport.scrollLeft,
      filter: filter.scrollLeft,
      columns: [...root.querySelectorAll("th[data-col]")].map((th) => {
        const col = th.dataset.col;
        const header = th.getBoundingClientRect();
        const group = root.querySelector(`[part="filter-operator"][data-col="${col}"]`).parentElement.getBoundingClientRect();
        return { col, header: [Math.round(header.left), Math.round(header.width)], group: [Math.round(group.left), Math.round(group.width)] };
      }),
    };
  });

const aligned = (columns) => columns.every(({ header, group }) => Math.abs(header[0] - group[0]) <= 1 && Math.abs(header[1] - group[1]) <= 1);

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady);
});

test("each filter group stands under its column, as wide as it", async ({ page }) => {
  await expect.poll(async () => aligned((await layout(page)).columns)).toBe(true);
  // Narrow as the page is, no column is narrower than its header needs.
  for (const { header } of (await layout(page)).columns) expect(header[1]).toBeGreaterThanOrEqual(64);
});

test("scrolling the rows scrolls the filter row, and the other way round", async ({ page }) => {
  await expect.poll(async () => aligned((await layout(page)).columns)).toBe(true);
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part~="viewport"]').scrollLeft = 200;
  });
  await expect.poll(async () => (await layout(page)).filter).toBe(200);
  expect(aligned((await layout(page)).columns)).toBe(true);

  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="filter"]').scrollLeft = 40;
  });
  await expect.poll(async () => (await layout(page)).viewport).toBe(40);
  expect(aligned((await layout(page)).columns)).toBe(true);
});

test("a column made wider takes its filter group with it", async ({ page }) => {
  await expect.poll(async () => aligned((await layout(page)).columns)).toBe(true);
  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.querySelector("opengrid-grid"), { customer: { width: 220 } }),
  );
  await expect.poll(async () => (await layout(page)).columns.find((column) => column.col === "1").group[1]).toBe(220);
  expect(aligned((await layout(page)).columns)).toBe(true);
});
