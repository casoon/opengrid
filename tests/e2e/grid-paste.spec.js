import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Pasting a block from a spreadsheet (issue #184): anchored at the focused
// cell, shown before it happens, applied with Enter, dropped with Escape.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const summary = (page) =>
  shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent ?? null);
const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-paste.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

/** What a spreadsheet puts on the clipboard, pasted into the focused cell. */
async function paste(page, text) {
  await page.evaluate((text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
  }, text);
}

const block = (rows, make) =>
  Array.from({ length: rows }, (_, index) => make(index).join("\t")).join("\r\n") + "\r\n";

test("30 rows of 4 cells are shown first, applied by Enter as one event of 120 changes", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 1);
  await paste(page, block(30, (i) => [`New ${i + 1}`, "1.234,50", String(100 + i), `note ${i + 1}`]));

  // Nothing has changed yet; the preview says what would.
  await expect.poll(() => summary(page)).toBe("30 rows");
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await cell(page, 0, 1)).toBe("Item 1");
  expect(await status(page)).toContain("30 rows");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  await page.keyboard.press("Enter");
  const pasted = await page.evaluate(() => window.__pasted);
  expect(pasted).toHaveLength(1);
  expect(pasted[0].changes).toHaveLength(120);
  expect(pasted[0].added).toEqual([]);
  // Records by key, a German decimal as the exact decimal it is.
  expect(pasted[0].changes[0]).toMatchObject({ key: 1, row: 0, column: "name", value: "New 1", previous: "Item 1" });
  expect(pasted[0].changes[1]).toMatchObject({ key: 1, column: "price", value: "1234.50", previous: "1.00" });
  expect(pasted[0].changes[119]).toMatchObject({ key: 30, column: "note", value: "note 30" });
  // The loaded rows show it, marked unsaved, and the preview is gone.
  expect(await cell(page, 0, 1)).toContain("New 1");
  expect(await shadow(page, (root) => root.querySelector('[part="paste-preview"]'))).toBeNull();
  expect(await status(page)).toContain("120 values pasted");
});

test("a value that cannot be read is marked with its reason, and the paste is not applied", async ({ page }) => {
  await open(page);
  await focusCell(page, 2, 1);
  await paste(page, "A\tabc\nB\t5\n");
  await expect.poll(() => summary(page)).toBe("2 rows, 1 value not readable");
  const mark = await shadow(page, (root) => {
    const bad = root.querySelector('[part="paste-preview"] td[aria-invalid="true"]');
    return bad?.textContent;
  });
  // The price column is a number; "abc" is not one.
  expect(mark).toContain("“abc” cannot be read as number");

  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await status(page)).toContain("Not pasted");
  expect(await summary(page)).not.toBeNull();

  await page.keyboard.press("Escape");
  expect(await summary(page)).toBeNull();
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await status(page)).toContain("Paste discarded");
  // The focus is back on the cell the paste started from.
  expect(
    await shadow(page, (root) => {
      const active = root.activeElement;
      return `${active?.getAttribute("data-row")}:${active?.getAttribute("data-col")}`;
    }),
  ).toBe("2:1");
  expect(await cell(page, 2, 1)).toBe("Item 3");
});

const activeAt = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active?.getAttribute("data-row") ? `${active.getAttribute("data-row")}:${active.getAttribute("data-col")}` : null;
  });

/** Goes to the name column of the row `up` above the last one, by the keyboard (the pool holds only a window). */
async function goToRow(page, last, up) {
  await focusCell(page, 0, 1);
  await page.keyboard.press("Control+End");
  // The window follows; the key after it needs the cell in place.
  await expect.poll(() => activeAt(page)).toBe(`${last}:4`);
  await page.keyboard.press("Home");
  await expect.poll(() => activeAt(page)).toBe(`${last}:0`);
  for (let i = 0; i < up; i++) {
    await page.keyboard.press("ArrowUp");
  }
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => activeAt(page)).toBe(`${last - up}:1`);
}

test("rows past the end are cut off without add-row", async ({ page }) => {
  await open(page);
  // The last row is 39; the block starts one above it.
  await goToRow(page, 39, 1);
  await paste(page, block(4, (i) => [`Late ${i}`, "2,00"]));
  await expect.poll(() => summary(page)).toBe("2 rows, 4 values beyond the end are not pasted");
  await page.keyboard.press("Enter");
  const [pasted] = await page.evaluate(() => window.__pasted);
  expect(pasted.changes).toHaveLength(4);
  expect(pasted.added).toEqual([]);
});

test("with add-row the rows past the end become added rows, and a required column the block misses refuses them", async ({ page }) => {
  await open(page, "?add");
  // The new row is the last; the last source row is above it.
  await goToRow(page, 40, 1);
  // Without the id (required) the new row cannot be added.
  await paste(page, "Late 1\t2,00\t7\nLate 2\t3,00\t8\n");
  await expect.poll(() => summary(page)).toBe("2 rows, 1 value not readable");
  await page.keyboard.press("Escape");

  await page.keyboard.press("ArrowLeft");
  await paste(page, "40\tLate 1\t2,00\t7\n41\tLate 2\t3,00\t8\n");
  await expect.poll(() => summary(page)).toBe("2 rows");
  await page.keyboard.press("Enter");
  const [pasted] = await page.evaluate(() => window.__pasted);
  expect(pasted.changes).toHaveLength(4);
  expect(pasted.added).toHaveLength(1);
  expect(pasted.added[0].values).toMatchObject({ id: 41, name: "Late 2", price: "3.00", seats: 8 });
  expect(pasted.added[0].provisionalKey).toMatch(/^new-/);
});
