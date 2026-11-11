import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// A block of cells, marked and copied (issue #327).

const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });
const marked = (page) =>
  shadow(page, (root) => [...root.querySelectorAll("td[data-range]")].map((td) => `${td.dataset.row}:${td.dataset.col}`));

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-range.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

async function paste(page, text) {
  await page.evaluate((text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    const event = new ClipboardEvent("paste", {
      clipboardData: data, dataType: "text/plain", data: text, bubbles: true, cancelable: true, composed: true,
    });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
  }, text);
}

test("Shift+arrows mark a block, say its size, and any other key ends it", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 1);
  await page.keyboard.press("Shift+ArrowDown");
  expect(await status(page)).toContain("2 by 1 cells");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowRight");
  expect(await status(page)).toContain("3 by 2 cells");
  expect(await marked(page)).toEqual(["1:1", "1:2", "2:1", "2:2", "3:1", "3:2"]);
  // The row selection is another thing: nothing is selected, no cell says so.
  expect(await shadow(page, (root) => root.querySelectorAll('[aria-selected="true"]').length)).toBe(0);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  // Back up shrinks it from the focus edge.
  await page.keyboard.press("Shift+ArrowUp");
  expect(await marked(page)).toEqual(["1:1", "1:2", "2:1", "2:2"]);

  await page.keyboard.press("ArrowDown");
  expect(await marked(page)).toEqual([]);
});

test("Ctrl+C copies values, tab-separated, and the block pastes back unchanged", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 2);
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowRight");
  await page.keyboard.press("ControlOrMeta+c");
  // Values, not text: the number as it is, and the note that a spreadsheet
  // would run as a formula carries the guard.
  expect(await page.evaluate(() => window.__copied)).toEqual(["10\t'=1+1\n20\ta,b\n"]);
  expect(await status(page)).toContain("Copied 2 by 2 cells");

  // Notes 2 to 5 — a comma, a tab, a quote — in one column, and back into
  // another place of the same column.
  await focusCell(page, 1, 3);
  for (let i = 0; i < 3; i++) await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("ControlOrMeta+c");
  const copied = await page.evaluate(() => window.__copied.at(-1));
  await focusCell(page, 8, 3);
  await paste(page, copied);
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__pasted.length)).toBe(1);
  expect(await Promise.all([8, 9, 10, 11].map((row) => cell(page, row, 3)))).toEqual(["=1+1", "a,b", "two\tcells", 'say "hi"']);
});

test("Shift+click and dragging mark a block too", async ({ page }) => {
  await open(page);
  await focusCell(page, 2, 1);
  await shadow(page, (root) => root.querySelector('td[data-row="4"][data-col="2"]').scrollIntoView());
  const to = await page.locator("opengrid-grid").locator('td[data-row="4"][data-col="2"]').boundingBox();
  await page.keyboard.down("Shift");
  await page.mouse.click(to.x + 4, to.y + 4);
  await page.keyboard.up("Shift");
  expect(await marked(page)).toEqual(["2:1", "2:2", "3:1", "3:2", "4:1", "4:2"]);
  expect(await status(page)).toContain("3 by 2 cells");

  // A drag from another cell starts another block.
  const from = await page.locator("opengrid-grid").locator('td[data-row="6"][data-col="1"]').boundingBox();
  const end = await page.locator("opengrid-grid").locator('td[data-row="7"][data-col="2"]').boundingBox();
  await page.mouse.move(from.x + 4, from.y + 4);
  await page.mouse.down();
  await page.mouse.move(end.x + 4, end.y + 4, { steps: 4 });
  await page.mouse.up();
  expect(await marked(page)).toEqual(["6:1", "6:2", "7:1", "7:2"]);
  expect(await status(page)).toContain("2 by 2 cells");
});

test("a range that reaches rows that are not loaded copies nothing and says so", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 1);
  for (let i = 0; i < 25; i++) await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("ControlOrMeta+c");
  await expect.poll(() => status(page)).toContain("Not copied");
  // The browser's own copy was kept from putting anything on the clipboard.
  expect(await page.evaluate(() => window.__copied.filter(Boolean))).toEqual([]);
});
