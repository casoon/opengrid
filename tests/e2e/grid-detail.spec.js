import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow } from "./helpers.js";

// Master/detail (issue #318): a panel the page builds under an open row.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-detail.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

const rowTop = (page, row) =>
  shadow(
    page,
    (root, { row }) => {
      const tr = root.querySelector(`td[data-row="${row}"]`)?.closest("tr");
      return tr && { top: tr.style.transform, index: tr.getAttribute("aria-rowindex") };
    },
    { row },
  );

const table = (page) =>
  shadow(page, (root) => {
    const t = root.querySelector("table");
    return { rows: t.getAttribute("aria-rowcount"), height: root.querySelector("tbody").style.height };
  });

const button = (page, row) =>
  shadow(
    page,
    (root, { row }) => {
      const b = root.querySelector(`td[data-row="${row}"][data-col="0"] button[data-detail-toggle]`);
      return b && { expanded: b.getAttribute("aria-expanded"), label: b.getAttribute("aria-label") };
    },
    { row },
  );

const panel = (page, row) =>
  shadow(
    page,
    (root, { row }) => {
      const tr = root.querySelector(`tr[data-detail-row="${row}"]`);
      if (!tr) return null;
      const td = tr.querySelector("td");
      return {
        role: tr.getAttribute("role"),
        cellRole: td.getAttribute("role"),
        colspan: td.getAttribute("colspan"),
        label: tr.getAttribute("aria-label"),
        index: tr.getAttribute("aria-rowindex"),
        top: tr.style.transform,
        height: td.getBoundingClientRect().height,
        text: td.textContent,
      };
    },
    { row },
  );

test("a button opens a panel with its own row and cell; the rows below move down by its height", async ({ page }) => {
  await open(page);
  const before = await table(page);
  expect(await button(page, 1)).toEqual({ expanded: "false", label: "Show details" });
  const row2 = await rowTop(page, 2);

  await shadow(page, (root) => root.querySelector('td[data-row="1"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 1)).not.toBeNull();
  const opened = await panel(page, 1);
  expect(opened.role).toBe("row");
  expect(opened.cellRole).toBe("gridcell");
  expect(opened.label).toBe("Details: 2");
  expect(opened.text).toContain("Open Customer 2");
  // The node is 100 px; the cell adds its one line below.
  const height = Math.ceil(opened.height);
  expect(height).toBeGreaterThanOrEqual(100);
  expect(height).toBeLessThanOrEqual(102);
  expect(await button(page, 1)).toEqual({ expanded: "true", label: "Hide details" });

  // aria-rowcount counts the panel, the sizer is that much taller, and the rows
  // below sit 100 px lower and one index later.
  const after = await table(page);
  expect(Number(after.rows)).toBe(Number(before.rows) + 1);
  expect(parseInt(after.height)).toBe(parseInt(before.height) + height);
  const moved = await rowTop(page, 2);
  expect(parseInt(moved.top.match(/-?\d+/)[0])).toBe(parseInt(row2.top.match(/-?\d+/)[0]) + height);
  expect(Number(moved.index)).toBe(Number(row2.index) + 1);
  // The panel's own index sits between the master's and the next row's.
  expect(Number(opened.index)).toBe(Number((await rowTop(page, 1)).index) + 1);

  await shadow(page, (root) => root.querySelector('td[data-row="1"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 1)).toBeNull();
  expect(await table(page)).toEqual(before);
  expect(await rowTop(page, 2)).toEqual(row2);
});

test("Alt+Down opens and Alt+Up closes the panel of the active row, announced once, with an event", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="3"][data-col="1"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(() => panel(page, 3)).not.toBeNull();
  await expect(page.locator("opengrid-grid [part=status], opengrid-grid")).toBeTruthy();
  const status = () => shadow(page, (root) => root.querySelector('[part="status"]')?.textContent);
  await expect.poll(status).toContain("Details shown");
  // The focus stays on the cell.
  expect(await shadow(page, (root) => root.activeElement?.dataset.row)).toBe("3");
  await page.keyboard.press("Alt+ArrowUp");
  await expect.poll(() => panel(page, 3)).toBeNull();
  await expect.poll(status).toContain("Details hidden");
  expect(await page.evaluate(() => window.__events)).toEqual([
    { key: 4, row: 3, open: true },
    { key: 4, row: 3, open: false },
  ]);
});

test("Escape inside the panel closes it and returns to the row", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="1"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(() => panel(page, 0)).not.toBeNull();
  await shadow(page, (root) => root.querySelector('tr[data-detail-row="0"] a').focus());
  // Inside, the keys are the page's: an arrow does not move the grid's cell.
  await page.keyboard.press("ArrowDown");
  expect(await shadow(page, (root) => root.activeElement?.tagName)).toBe("A");
  await page.keyboard.press("Escape");
  await expect.poll(() => panel(page, 0)).toBeNull();
  expect(await shadow(page, (root) => root.activeElement?.dataset.row)).toBe("0");
});

test("scrolling with panels open keeps the row under the scroll position and the panel node", async ({ page }) => {
  await open(page);
  for (const row of [1, 4]) {
    await shadow(page, (root, { row }) => root.querySelector(`td[data-row="${row}"] button[data-detail-toggle]`).click(), { row });
    await expect.poll(() => panel(page, row)).not.toBeNull();
  }
  expect(await page.evaluate(() => window.__built)).toBe(2);
  // Far down and back: the panel is the same node, not built again.
  await shadow(page, (root) => (root.querySelector('[part~="viewport"]').scrollTop = 6000));
  await expect.poll(() => shadow(page, (root) => root.querySelector('td[data-row]:not([data-row="0"])') && Number(root.querySelector("tbody tr:not([data-detail-row]) td[data-row]").dataset.row))).toBeGreaterThan(100);
  expect(await panel(page, 1)).toBeNull();
  await shadow(page, (root) => (root.querySelector('[part~="viewport"]').scrollTop = 0));
  await expect.poll(() => panel(page, 4)).not.toBeNull();
  expect(await page.evaluate(() => window.__built)).toBe(2);
  // Scrolled to the very end, the last row's index is the logical one + 2 panels.
  await shadow(page, (root) => { const v = root.querySelector('[part~="viewport"]'); v.scrollTop = v.scrollHeight; });
  await expect.poll(() => shadow(page, (root) => [...root.querySelectorAll("tbody tr:not([data-detail-row])")].some((tr) => tr.getAttribute("aria-rowindex") === "503"))).toBe(true);
});

test("a grid without a row-key offers no panels, and axe finds nothing with one open", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="2"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 2)).not.toBeNull();
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("row-key"));
  await expect.poll(() => button(page, 2)).toBeNull();
  expect(await panel(page, 2)).toBeNull();
});

test("the view carries the open rows by key and set_view opens them as their rows load", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="1"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 1)).not.toBeNull();
  const view = await page.evaluate(() => window.__module.get_view(document.querySelector("opengrid-grid")));
  expect(view.detail).toEqual([2]);
  // Close, then restore the view: the panel is back.
  await shadow(page, (root) => root.querySelector('td[data-row="1"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 1)).toBeNull();
  await page.evaluate((view) => window.__module.set_view(document.querySelector("opengrid-grid"), view), view);
  await expect.poll(() => panel(page, 1)).not.toBeNull();
  // A key that is not loaded waits and is not an error.
  await page.evaluate(() => window.__module.set_view(document.querySelector("opengrid-grid"), { detail: [400] }));
  await expect.poll(() => panel(page, 1)).toBeNull();
  expect((await page.evaluate(() => window.__module.get_view(document.querySelector("opengrid-grid")))).detail).toEqual([400]);
});

test("a new result closes the panels: sorting puts other rows in their places", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="1"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 1)).not.toBeNull();
  await shadow(page, (root) => root.querySelector('th[data-col="1"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => panel(page, 1)).toBeNull();
  expect(Number((await table(page)).rows)).toBe(501);
});

test("a panel that grows moves the rows below it", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="0"] button[data-detail-toggle]').click());
  await expect.poll(() => panel(page, 0)).not.toBeNull();
  const before = await rowTop(page, 1);
  await shadow(page, (root) => (root.querySelector('tr[data-detail-row="0"] div[data-key]').style.height = "180px"));
  await expect
    .poll(async () => parseInt((await rowTop(page, 1)).top.match(/-?\d+/)[0]))
    .toBe(parseInt(before.top.match(/-?\d+/)[0]) + 80);
});
