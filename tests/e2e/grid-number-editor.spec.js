import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// The editor of a number column reads the page's notation and suits touch
// (issue #535): a text field with `inputmode`, `1,10` on a German page and `1.10`
// on an English one, group marks only where they stand right, and what cannot be
// read is refused under the editor with the reason — never committed as NULL.
// Columns: id (read-only key), price decimal(12,2), qty int64, ratio float64.

const PRICE = 1;
const QTY = 2;
const RATIO = 3;

const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });
const editor = (page) =>
  shadow(page, (root) => {
    const node = root.querySelector('[part="editor"]');
    if (!node) return null;
    return {
      tag: node.tagName.toLowerCase(),
      type: node.getAttribute("type"),
      inputmode: node.getAttribute("inputmode"),
      step: node.getAttribute("step"),
      value: node.value,
      invalid: node.getAttribute("aria-invalid"),
      focused: root.activeElement === node,
    };
  });
/** The sentence under the cell editor. */
const sentence = (page) =>
  shadow(page, (root) => root.querySelector('td [part="validation"]')?.textContent ?? null);

async function open(page, lang = "en") {
  await page.goto(`/tests/e2e/fixtures/grid-number.html?lang=${lang}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

/** Opens the editor on a cell, replaces its text and commits with Enter. */
async function enter(page, row, col, text) {
  await focusCell(page, row, col);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  if (text === "") await page.keyboard.press("Backspace");
  else await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

const changes = (page) => page.evaluate(() => window.__changes);

test("a number is a text field with a decimal keypad — a numeric one for integers", async ({ page }) => {
  await open(page);
  for (const [col, inputmode] of [
    [PRICE, "decimal"],
    [QTY, "numeric"],
    [RATIO, "decimal"],
  ]) {
    await focusCell(page, 0, col);
    await page.keyboard.press("Enter");
    // No `type="number"` and no step: the field is read by the grid, not the browser.
    expect(await editor(page)).toMatchObject({ tag: "input", type: "text", inputmode, step: null, focused: true });
    await page.keyboard.press("Escape");
  }
});

test("an English page opens 1.10, reads 1.10 and a group mark where it stands right, and refuses 1,10", async ({ page }) => {
  await open(page, "en");
  await focusCell(page, 0, PRICE);
  await page.keyboard.press("Enter");
  expect(await editor(page)).toMatchObject({ value: "1.10" });
  await page.keyboard.press("Escape");

  await enter(page, 0, PRICE, "2.5");
  await expect.poll(() => cell(page, 0, PRICE)).toContain("2.50");
  // The page is told the wire notation, at the scale of the column.
  expect(await changes(page)).toEqual([{ row: 0, key: 1, column: "price", value: "2.50", previous: "1.10" }]);

  await enter(page, 1, PRICE, "1,234.56");
  await expect.poll(() => cell(page, 1, PRICE)).toContain("1234.56");
  expect((await changes(page)).at(-1)).toMatchObject({ column: "price", value: "1234.56" });

  // On an English page a comma is a group mark: 1,10 is not read.
  const before = (await changes(page)).length;
  await enter(page, 2, PRICE, "1,10");
  expect(await sentence(page)).toBe("“1,10” is not a number");
  expect(await editor(page)).toMatchObject({ value: "1,10", invalid: "true", focused: true });
  await expect.poll(() => status(page)).toContain("Error: “1,10” is not a number");
  expect((await changes(page)).length).toBe(before);
  await page.keyboard.press("Escape");
  await expect.poll(() => editor(page)).toBeNull();
  expect(await cell(page, 2, PRICE)).toBe("1234.50");
});

test("a German page opens 1,10, reads 1,10 and 1.10 and a group mark where it stands right", async ({ page }) => {
  await open(page, "de");
  await focusCell(page, 0, PRICE);
  await page.keyboard.press("Enter");
  expect(await editor(page)).toMatchObject({ value: "1,10" });
  await page.keyboard.press("Escape");

  await enter(page, 0, PRICE, "2,5");
  await expect.poll(() => cell(page, 0, PRICE)).toContain("2.50");
  await enter(page, 1, PRICE, "1.10");
  await expect.poll(() => cell(page, 1, PRICE)).toContain("1.10");
  await enter(page, 2, PRICE, "1.234,56");
  await expect.poll(() => cell(page, 2, PRICE)).toContain("1234.56");
  expect(await changes(page)).toEqual([
    { row: 0, key: 1, column: "price", value: "2.50", previous: "1.10" },
    { row: 1, key: 2, column: "price", value: "1.10", previous: "1234.50" },
    { row: 2, key: 3, column: "price", value: "1234.56", previous: "1234.50" },
  ]);

  // An integer reads group marks too; a float the same notation.
  await enter(page, 3, QTY, "1.234");
  await expect.poll(() => cell(page, 3, QTY)).toBe("1234");
  await enter(page, 3, RATIO, "0,25");
  await expect.poll(() => cell(page, 3, RATIO)).toBe("0.25");
});

test("what cannot be read is refused under the editor with the reason, and fixing it commits", async ({ page }) => {
  await open(page, "de");

  // Two decimal marks.
  await enter(page, 0, PRICE, "1,1,0");
  expect(await sentence(page)).toBe("“1,1,0” is not a number");
  expect(await editor(page)).toMatchObject({ value: "1,1,0", invalid: "true", focused: true });
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  // Typing takes the sentence away; the next Enter says the next reason.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("12,34.5");
  expect(await sentence(page)).toBeNull();
  await page.keyboard.press("Enter");
  expect(await sentence(page)).toBe("“12,34.5” is not a number");

  // A group mark in the wrong place; more decimals than the scale, never rounded.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("12.34,5");
  await page.keyboard.press("Enter");
  expect(await sentence(page)).toBe("“12.34,5” is not a number");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("1,234");
  await page.keyboard.press("Enter");
  expect(await sentence(page)).toBe("“1,234” has more than 2 decimal places");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("99999999999,99");
  await page.keyboard.press("Enter");
  expect(await sentence(page)).toBe("“99999999999,99” is too large for this column");

  // Nothing was committed, as a number or as NULL.
  expect(await changes(page)).toEqual([]);

  // Fixed, the same editor commits.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("1,23");
  await page.keyboard.press("Enter");
  await expect.poll(() => editor(page)).toBeNull();
  await expect.poll(() => cell(page, 0, PRICE)).toContain("1.23");
  expect(await changes(page)).toEqual([{ row: 0, key: 1, column: "price", value: "1.23", previous: "1.10" }]);
});

test("an integer column says it wants a whole number, and an empty field is still NULL", async ({ page }) => {
  await open(page, "de");
  await enter(page, 0, QTY, "1,5");
  expect(await sentence(page)).toBe("“1,5” is not a whole number");
  await page.keyboard.press("Escape");
  expect(await cell(page, 0, QTY)).toBe("3");

  await enter(page, 0, QTY, "");
  await expect.poll(() => cell(page, 0, QTY)).toBe("");
  expect(await changes(page)).toEqual([{ row: 0, key: 1, column: "qty", value: "", previous: "3" }]);
});

test("a refused number in the new row keeps the editor, and the row does not move on", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-number.html?lang=de&add");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tr[data-new-row]")).toBeVisible();
  await shadow(page, (root) => root.querySelector('tr[data-new-row] td[data-col="1"]').focus());
  await page.keyboard.press("Enter");
  await page.keyboard.type("1,1,0");
  await page.keyboard.press("Enter");
  expect(await sentence(page)).toBe("“1,1,0” is not a number");
  expect(await editor(page)).toMatchObject({ value: "1,1,0", focused: true });

  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("2,5");
  await page.keyboard.press("Enter");
  await expect.poll(() => editor(page)).toBeNull();
  // Read, the row moves on to the next column.
  expect(await shadow(page, (root) => root.activeElement?.getAttribute("data-col"))).toBe("2");
});

test("a validator is given the number in the wire notation, whatever was typed", async ({ page }) => {
  await open(page, "de");
  await enter(page, 0, PRICE, "2,5");
  await expect.poll(() => cell(page, 0, PRICE)).toContain("2.50");
  const seen = await page.evaluate(() => window.__seen);
  // On every keystroke and on commit: read, it is `2.50`; never the typed `2,5`.
  expect(seen.at(-1)).toBe("2.50");
  expect(seen).not.toContain("2,5");
  // What cannot be read yet is given as it stands.
  await enter(page, 1, PRICE, "1,1,0");
  expect(await page.evaluate(() => window.__seen)).toContain("1,1,0");
});

test("a typed character opens the editor with it, in the page's notation", async ({ page }) => {
  await open(page, "de");
  await focusCell(page, 0, PRICE);
  await page.keyboard.press("7");
  await page.keyboard.type(",5");
  expect(await editor(page)).toMatchObject({ value: "7,5", focused: true });
  await page.keyboard.press("Enter");
  await expect.poll(() => cell(page, 0, PRICE)).toContain("7.50");
});

test("the record form uses the same field, refuses under it and saves the wire notation", async ({ page }) => {
  await open(page, "de");
  await focusCell(page, 0, PRICE);
  await page.keyboard.press("Shift+F2");
  const field = page.locator('opengrid-grid [part="record-dialog"] [data-record-col="1"]');
  await expect(field).toHaveAttribute("inputmode", "decimal");
  await expect(field).toHaveAttribute("type", "text");
  await expect(field).toHaveValue("1,10");
  await expect(page.locator('opengrid-grid [part="record-dialog"] [data-record-col="2"]')).toHaveAttribute("inputmode", "numeric");

  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("1,1,0");
  await page.keyboard.press("Enter");
  await expect(page.locator('opengrid-grid [part="record-dialog"] #og-record-error-1')).toHaveText("“1,1,0” is not a number");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  expect(await changes(page)).toEqual([]);

  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("2,5");
  await page.keyboard.press("Enter");
  await expect.poll(() => cell(page, 0, PRICE)).toContain("2.50");
  expect(await changes(page)).toEqual([{ row: 0, key: 1, column: "price", value: "2.50", previous: "1.10" }]);
});

test("the bulk panel uses the same field, refuses under it and sets the wire notation", async ({ page }) => {
  await open(page, "de");
  await shadow(page, (root) =>
    root.querySelector('tr[aria-rowindex="3"] td[data-select="row"] [part="select-mark"], tr[aria-rowindex="3"] td[data-select="row"]').focus(),
  );
  for (let i = 0; i < 3; i++) {
    await page.keyboard.press(" ");
    await page.keyboard.press("ArrowDown");
  }
  await focusCell(page, 1, PRICE);
  await page.keyboard.press("F2");
  const field = page.locator('opengrid-grid [part="bulk"] [part="editor"]');
  await expect(field).toHaveAttribute("inputmode", "decimal");
  await expect(field).toHaveAttribute("type", "text");

  await page.keyboard.type("abc");
  await page.keyboard.press("Enter");
  await expect(page.locator('opengrid-grid [part="bulk"] [part="validation"]')).toHaveText("“abc” is not a number");
  await expect(field).toHaveAttribute("aria-invalid", "true");
  expect(await page.evaluate(() => window.__bulk)).toEqual([]);

  // Typing takes the sentence away; a read number sets all three rows.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("1,5");
  await expect(page.locator('opengrid-grid [part="bulk"] [part="validation"]')).toHaveCount(0);
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__bulk)).toEqual([{ keys: [2, 3, 4], column: "price", value: "1.50" }]);
  expect([await cell(page, 1, PRICE), await cell(page, 2, PRICE), await cell(page, 3, PRICE)]).toEqual(["1.50", "1.50", "1.50"]);
});

test.describe("on a touch screen", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 800 } });

  test("the editors ask for the decimal keypad, and the record form and bulk panel the same", async ({ page }) => {
    await open(page, "de");
    // Two taps on a focused cell open the editor.
    await focusCell(page, 0, PRICE);
    await page.keyboard.press("Enter");
    expect(await editor(page)).toMatchObject({ type: "text", inputmode: "decimal", value: "1,10" });
    await page.keyboard.press("Escape");
    await focusCell(page, 0, QTY);
    await page.keyboard.press("Enter");
    expect(await editor(page)).toMatchObject({ type: "text", inputmode: "numeric", value: "3" });
    await page.keyboard.press("Escape");

    await focusCell(page, 0, PRICE);
    await page.keyboard.press("Shift+F2");
    const modes = await shadow(page, (root) =>
      [...root.querySelectorAll('[part="record-dialog"] [data-record-col]')].map((input) => [input.getAttribute("type"), input.getAttribute("inputmode")]),
    );
    expect(modes).toEqual([["text", "numeric"], ["text", "decimal"], ["text", "numeric"], ["text", "decimal"]]);
  });
});
