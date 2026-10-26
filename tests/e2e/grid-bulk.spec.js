import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// One value for every selected row (issue #187): F2 or the column menu opens
// the column's editor once, and one event names the records.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const panel = (page) => shadow(page, (root) => root.querySelector('[part="bulk"]')?.getAttribute("aria-label") ?? null);
const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-bulk.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

/** Selects rows 1, 2 and 3 by the keyboard: Space on each selection cell. */
async function selectThree(page) {
  await shadow(page, (root) => root.querySelector('tr[aria-rowindex="3"] td[data-select="row"] [part="select-mark"], tr[aria-rowindex="3"] td[data-select="row"]').focus());
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press(" ");
    await page.keyboard.press("ArrowDown");
  }
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

test("F2 with rows selected opens the column's editor once, and Enter sets it for all of them", async ({ page }) => {
  await open(page);
  await selectThree(page);
  await focusCell(page, 1, 3);
  await page.keyboard.press("F2");
  await expect.poll(() => panel(page)).toBe("Set seats for 3 rows selected");
  // No cell editor opened beside it.
  expect(await shadow(page, (root) => root.querySelectorAll('td [part="editor"]').length)).toBe(0);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  await page.keyboard.type("77");
  await page.keyboard.press("Enter");
  const events = await page.evaluate(() => window.__bulk);
  expect(events).toEqual([{ keys: [2, 3, 4], column: "seats", value: "77" }]);
  expect(await panel(page)).toBeNull();
  expect([await cell(page, 1, 3), await cell(page, 2, 3), await cell(page, 3, 3)]).toEqual(["77", "77", "77"]);
  expect(await shadow(page, (root) => root.querySelectorAll('td[data-col="3"][data-changed]').length)).toBe(3);
  expect(await status(page)).toContain("seats set for 3 rows");
});

test("Escape drops the panel, a read-only column refuses it, and a column with choices offers them", async ({ page }) => {
  await open(page);
  await selectThree(page);
  await focusCell(page, 1, 2);
  await page.keyboard.press("F2");
  await expect.poll(() => panel(page)).toBe("Set room for 3 rows selected");
  expect(await shadow(page, (root) => root.querySelector('[part="bulk"] select [value="B2"], [part="bulk"] select option[value="B2"]') !== null)).toBe(true);
  await page.keyboard.press("Escape");
  expect(await panel(page)).toBeNull();
  expect(await page.evaluate(() => window.__bulk)).toHaveLength(0);

  await focusCell(page, 1, 1);
  await page.keyboard.press("F2");
  expect(await panel(page)).toBeNull();
  expect(await status(page)).toContain("name is read-only");
});

test("the column menu offers it only with rows selected, and it reaches rows that are not loaded", async ({ page }) => {
  await open(page);
  // Nothing selected: no entry.
  await shadow(page, (root) => root.querySelector('th[data-col="3"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await expect
    .poll(() => shadow(page, (root) => [...root.querySelectorAll('[part="column-menu"] [role^="menuitem"]')].map((item) => item.textContent)))
    .not.toContain("Set for selected rows");
  await page.keyboard.press("Escape");

  // Everything selected, 40 rows of which 12 are loaded.
  await focusCell(page, 0, 3);
  await page.keyboard.press("Control+a");
  await shadow(page, (root) => root.querySelector('th[data-col="3"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await expect
    .poll(() => shadow(page, (root) => [...root.querySelectorAll('[part="column-menu"] [role^="menuitem"]')].map((item) => item.textContent)))
    .toContain("Set for selected rows");
  await shadow(page, (root) => [...root.querySelectorAll('[part="column-menu"] [role^="menuitem"]')].find((item) => item.textContent === "Set for selected rows").click());
  await expect.poll(() => panel(page)).toBe("Set seats for 40 rows selected");
  await page.keyboard.type("5");
  await page.keyboard.press("Enter");
  const [event] = await page.evaluate(() => window.__bulk);
  expect(event.keys).toEqual(Array.from({ length: 40 }, (_, index) => index + 1));
  expect(event.value).toBe("5");
});
