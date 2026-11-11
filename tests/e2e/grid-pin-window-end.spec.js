import { test, expect } from "@playwright/test";
import { shadow } from "./helpers.js";

// Issue #309: the focused row is pinned in its own pool slot (point 17). A window
// that ends at the last row (or starts at the first) must not lose that row to the
// pin. grid-long.html: 60 rows, a pool of 10, a viewport of about 6 rows.

const LAST = 59;

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-long.html");
  await page.evaluate(() => window.__opengridReady);
  await page.waitForSelector("opengrid-grid td[data-row]");
});

const focusRow = (page, row) =>
  shadow(page, (root, row) => root.querySelector(`td[data-row="${row}"][data-col="0"]`).focus(), row);

const scrollTo = (page, where) =>
  shadow(
    page,
    (root, where) => {
      const viewport = root.querySelector('[part="viewport"]');
      viewport.scrollTop = where === "end" ? viewport.scrollHeight : where;
    },
    where,
  );

const drawn = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr[aria-rowindex] td[data-col='0']")]
      .map((td) => Number(td.dataset.row))
      .sort((a, b) => a - b),
  );

/** Rows whose box lies inside the viewport below the header. */
const shown = (page) =>
  shadow(page, (root) => {
    const viewport = root.querySelector('[part="viewport"]');
    const bottom = viewport.getBoundingClientRect().top + viewport.clientHeight;
    const head = root.querySelector("thead").getBoundingClientRect().bottom;
    return [...root.querySelectorAll("tbody tr[aria-rowindex]")]
      .filter((tr) => {
        const box = tr.getBoundingClientRect();
        return box.bottom > head + 0.5 && box.top < bottom - 0.5;
      })
      .map((tr) => Number(tr.querySelector("td[data-col='0']").dataset.row))
      .sort((a, b) => a - b);
  });

const activeRow = (page) =>
  shadow(page, (root) => {
    const node = root.activeElement;
    return node && node.dataset.row !== undefined ? Number(node.dataset.row) : null;
  });

test("a far focused row does not cost the last row at the end of the result", async ({ page }) => {
  await focusRow(page, 3);
  await scrollTo(page, "end");
  await expect.poll(async () => (await shown(page)).at(-1)).toBe(LAST);
  // The pin is still the pin: the focus stands where it stood.
  expect(await activeRow(page)).toBe(3);
  expect(await drawn(page)).toContain(3);
  // The last row is reachable from the keyboard.
  await page.keyboard.press("Control+End");
  await expect.poll(() => activeRow(page)).toBe(LAST);
  await page.keyboard.press("ArrowUp");
  await expect.poll(() => activeRow(page)).toBe(LAST - 1);
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => activeRow(page)).toBe(LAST);
});

test("a focused row inside the window at the end loses nothing", async ({ page }) => {
  await scrollTo(page, 55 * 32);
  await expect.poll(() => drawn(page).then((rows) => rows.includes(55))).toBe(true);
  await focusRow(page, 55);
  await scrollTo(page, "end");
  await expect.poll(async () => (await shown(page)).at(-1)).toBe(LAST);
  expect(await drawn(page)).toEqual([50, 51, 52, 53, 54, 55, 56, 57, 58, 59]);
  expect(await activeRow(page)).toBe(55);
});

test("a far focused row does not cost the first row at the start of the result", async ({ page }) => {
  await scrollTo(page, 30 * 32);
  await expect.poll(() => drawn(page).then((rows) => rows.includes(30))).toBe(true);
  await focusRow(page, 30);
  await scrollTo(page, 0);
  await expect.poll(async () => (await shown(page))[0]).toBe(0);
  expect(await activeRow(page)).toBe(30);
  expect(await drawn(page)).toContain(30);
});
