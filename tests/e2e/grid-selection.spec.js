import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Selection and the event contract (plan point 35).
//
// The selection names **logical** rows, so it has to survive the recycling of
// the DOM rows — and it has to disappear when sorting or filtering changes
// which rows sit in those positions.

/** Focuses a cell inside the shadow root. */
async function focusCell(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

/** The logical rows the DOM currently marks as selected. */
async function selectedRows(page) {
  return page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-selected") === "true")
      .map((tr) => Number(tr.getAttribute("aria-rowindex")) - 2),
  );
}

/** Every `opengrid-selection-change` the page has seen. */
async function events(page) {
  return page.evaluate(() => window.__selection ?? []);
}

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-long.html");
  await page.waitForFunction(() => document.querySelector("opengrid-grid")?.shadowRoot);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  // Listen on the host: the contract says the event bubbles and is composed.
  await page.evaluate(() => {
    window.__selection = [];
    document.addEventListener("opengrid-selection-change", (event) => {
      window.__selection.push(event.detail);
    });
  });
}

test("Space selects the focused row and the page hears about it", async ({ page }) => {
  await open(page);
  await focusCell(page, 'td[data-row="2"][data-col="0"]');
  await page.keyboard.press(" ");

  await expect.poll(() => selectedRows(page)).toEqual([2]);
  const seen = await events(page);
  expect(seen.at(-1)).toEqual({ rows: [2], count: 1 });

  // The event reaches the document, so it bubbled out of the shadow root.
  await page.keyboard.press(" ");
  await expect.poll(() => selectedRows(page)).toEqual([]);
  expect((await events(page)).at(-1)).toEqual({ rows: [], count: 0 });
});

test("Shift+Space extends from the last plain selection", async ({ page }) => {
  await open(page);
  await focusCell(page, 'td[data-row="1"][data-col="0"]');
  await page.keyboard.press(" ");
  await focusCell(page, 'td[data-row="4"][data-col="0"]');
  await page.keyboard.press("Shift+ ");

  await expect.poll(() => selectedRows(page)).toEqual([1, 2, 3, 4]);
});

test("Ctrl+A selects every matching row, not only the loaded ones", async ({ page }) => {
  await open(page);
  await focusCell(page, 'td[data-row="0"][data-col="0"]');
  await page.keyboard.press("Control+a");

  const seen = await events(page);
  const total = await page.evaluate(
    () =>
      Number(
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector("table")
          .getAttribute("aria-rowcount"),
      ) - 1,
  );
  expect(seen.at(-1).count).toBe(total);
  expect(seen.at(-1).count).toBeGreaterThan(await page.evaluate(
    () => document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr").length,
  ));
});

test("a selection survives scrolling, because it names logical rows", async ({ page }) => {
  await open(page);
  await focusCell(page, 'td[data-row="1"][data-col="0"]');
  await page.keyboard.press(" ");
  await expect.poll(() => selectedRows(page)).toEqual([1]);

  // `Ctrl+End` moves the **active** cell, not only the scroll position: the
  // focused row is pinned and never recycled (point 17), so leaving it behind
  // is the only way the pool really turns over.
  await page.keyboard.press("Control+End");
  await expect
    .poll(() => selectedRows(page))
    .toEqual([], "no recycled slot inherited the mark");

  await page.keyboard.press("Control+Home");
  await expect
    .poll(() => selectedRows(page))
    .toEqual([1], "the row was off-screen, not unselected");
});

test("sorting drops the selection, and says so", async ({ page }) => {
  await open(page);
  await focusCell(page, 'td[data-row="1"][data-col="0"]');
  await page.keyboard.press(" ");
  await expect.poll(() => selectedRows(page)).toEqual([1]);

  await focusCell(page, 'th[data-col="1"]');
  await page.keyboard.press("Enter");

  await expect.poll(() => selectedRows(page)).toEqual([]);
  expect((await events(page)).at(-1)).toEqual({ rows: [], count: 0 });
  // A selection that vanishes without a word is a trap.
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
            .textContent,
      ),
    )
    .toContain("Selection cleared");
});

test("has no axe violations with a selection", async ({ page }) => {
  await open(page);
  await focusCell(page, 'td[data-row="2"][data-col="0"]');
  await page.keyboard.press(" ");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
