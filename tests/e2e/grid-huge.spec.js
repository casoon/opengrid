import { test, expect } from "@playwright/test";

// Issue #236: the sizer is `total * row_height`, and a browser caps the height
// of an element (Chromium at 33 554 432px). The fixture answers with the real
// engine's rows but claims a total of its own, so the scroll mechanics of a very
// long result are tested without a very long result.

async function open(page, total) {
  await page.addInitScript((total) => (window.__total = total), total);
  await page.goto("/tests/e2e/fixtures/grid-huge.html");
  await page.evaluate(() => window.__opengridReady);
  await page.waitForSelector("opengrid-grid td[data-row]");
}

/** Scrolls to the very bottom and says which logical rows are drawn then. */
async function lastRowsDrawn(page) {
  await page.evaluate(() => {
    const viewport = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]');
    viewport.scrollTop = viewport.scrollHeight;
  });
  // The window follows the scroll event; wait until it has moved away from the top.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const root = document.querySelector("opengrid-grid").shadowRoot;
        return Math.max(...[...root.querySelectorAll("td[data-row]")].map((td) => Number(td.dataset.row)));
      }),
    )
    .toBeGreaterThan(100);
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const rows = [...root.querySelectorAll("tbody tr[aria-rowindex] td[data-col='0']")].map((td) => Number(td.dataset.row));
    return { last: Math.max(...rows), count: Number(root.querySelector("table").getAttribute("aria-rowcount")) - 1 };
  });
}

test("a result below the browser's element height is scrolled to its last row", async ({ page }) => {
  // 1 000 000 rows of 32px are 32 000 000px, under the cap.
  await open(page, 1_000_000);
  const { last, count } = await lastRowsDrawn(page);
  expect(count).toBe(1_000_000);
  expect(last).toBe(999_999);
});

test.fixme("a result above the browser's element height is scrolled to its last row too (#284)", async ({ page }) => {
  // 3 000 000 rows of 32px ask for 96 000 000px; Chromium stops at 33 554 432.
  await open(page, 3_000_000);
  const { last, count } = await lastRowsDrawn(page);
  expect(count).toBe(3_000_000);
  expect(last).toBe(2_999_999);
});
