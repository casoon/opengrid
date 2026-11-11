import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// A record as a form (issue #325): Shift+F2 opens it, saving is cell edits.

const dialog = (page) => shadow(page, (root) => root.querySelector('[part="record-dialog"]') !== null);
const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });
const field = (page, name) => page.locator("opengrid-grid").locator(`[part="record-dialog"] [aria-label="${name}"], [part="record-dialog"] label:text-is("${name}") + *`).first();

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-record.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

test("Shift+F2 opens the record as a labelled form, and a change is saved from the keyboard alone", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 3);
  await page.keyboard.press("Shift+F2");
  await expect.poll(() => dialog(page)).toBe(true);
  expect(await page.evaluate(() => window.__opens)).toEqual([{ key: 2, row: 1 }]);

  // One field per column, each named by its label; the page's own column is
  // shown and not editable. The focus is on the field of the cell it came from.
  const names = await shadow(page, (root) =>
    [...root.querySelectorAll('[part="record-dialog"] [data-record-field]')].map((wrap) => ({
      label: wrap.querySelector("label").textContent,
      disabled: wrap.querySelector("[data-record-col]").hasAttribute("disabled"),
    })),
  );
  expect(names).toEqual([
    { label: "id", disabled: false },
    { label: "name", disabled: true },
    { label: "room", disabled: false },
    { label: "seats", disabled: false },
    { label: "note", disabled: false },
    { label: "open", disabled: false },
  ]);
  expect(await shadow(page, (root) => root.activeElement?.closest('[part="record-dialog"]') !== null && root.activeElement.getAttribute("data-record-col"))).toBe("3");

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  // seats -> 99, then Tab to note and change it, Tab to open and tick it.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("99");
  await page.keyboard.press("Tab");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("moved");
  await page.keyboard.press("Tab");
  await page.keyboard.press(" ");
  await page.keyboard.press("Enter");

  await expect.poll(() => dialog(page)).toBe(false);
  // One event per changed field, in column order — as cell edits are.
  expect(await page.evaluate(() => window.__cells)).toEqual([
    { key: 2, row: 1, column: "seats", value: "99", previous: "1" },
    { key: 2, row: 1, column: "note", value: "moved", previous: "fine" },
    { key: 2, row: 1, column: "open", value: "true", previous: "false" },
  ]);
  expect([await cell(page, 1, 3), await cell(page, 1, 4)]).toEqual(["99", "moved"]);
  expect(await shadow(page, (root) => root.querySelectorAll('tr[aria-rowindex="3"] td[data-changed]').length)).toBe(3);
  expect(await status(page)).toContain("Record 2 saved, 3 fields changed");
  // The focus is back on the cell it came from.
  expect(await shadow(page, (root) => root.activeElement?.getAttribute("data-row") + ":" + root.activeElement?.getAttribute("data-col"))).toBe("1:3");
});

test("the whole record is one undo step", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 3);
  await page.keyboard.press("Shift+F2");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("50");
  await page.keyboard.press("Tab");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("two");
  await page.keyboard.press("Enter");
  await expect.poll(() => dialog(page)).toBe(false);
  await page.evaluate(() => (window.__cells.length = 0));

  await page.keyboard.press("Control+z");
  expect(await page.evaluate(() => window.__cells)).toEqual([
    { key: 2, row: 1, column: "seats", value: "1", previous: "50", undo: true },
    { key: 2, row: 1, column: "note", value: "fine", previous: "two", undo: true },
  ]);
  expect([await cell(page, 1, 3), await cell(page, 1, 4)]).toEqual(["1", "fine"]);
  expect(await status(page)).toContain("Undone: record 2, 2 fields");
  await page.keyboard.press("Shift+Control+z");
  expect([await cell(page, 1, 3), await cell(page, 1, 4)]).toEqual(["50", "two"]);
});

test("a field that cannot be saved keeps the form open, names the reason, and saves nothing", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 3);
  await page.keyboard.press("Shift+F2");
  // seats is required: empty is refused. note is blocked by the validator.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Delete");
  await page.keyboard.press("Tab");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("no");
  await page.keyboard.press("Enter");

  expect(await dialog(page)).toBe(true);
  expect(await page.evaluate(() => window.__cells)).toEqual([]);
  const errors = await shadow(page, (root) =>
    [...root.querySelectorAll('[part="record-dialog"] [data-record-field]')]
      .filter((wrap) => !wrap.querySelector("[data-record-error]").hidden)
      .map((wrap) => ({
        label: wrap.querySelector("label").textContent,
        text: wrap.querySelector("[data-record-error]").textContent,
        invalid: wrap.querySelector("[aria-invalid]")?.getAttribute("aria-describedby") === wrap.querySelector("[data-record-error]").id,
      })),
  );
  expect(errors.map((error) => error.label)).toEqual(["seats", "note"]);
  expect(errors[1].text).toBe("Say more than no.");
  expect(errors.every((error) => error.invalid)).toBe(true);
  // The focus is on the first field that is wrong.
  expect(await shadow(page, (root) => root.activeElement.getAttribute("data-record-col"))).toBe("3");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  // Fixed, it saves.
  await page.keyboard.type("5");
  await page.keyboard.press("Tab");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("yes");
  await page.keyboard.press("Enter");
  await expect.poll(() => dialog(page)).toBe(false);
  expect((await page.evaluate(() => window.__cells)).map((change) => change.column)).toEqual(["seats", "note"]);
});

test("Escape and Cancel change nothing, and a record saved unchanged fires nothing", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 3);
  await page.keyboard.press("Shift+F2");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("7");
  await page.keyboard.press("Escape");
  await expect.poll(() => dialog(page)).toBe(false);
  expect(await cell(page, 1, 3)).toBe("1");
  expect(await shadow(page, (root) => root.activeElement?.getAttribute("data-col"))).toBe("3");

  await page.keyboard.press("Shift+F2");
  await page.keyboard.press("Enter");
  await expect.poll(() => dialog(page)).toBe(false);
  expect(await page.evaluate(() => window.__cells)).toEqual([]);
  expect(await status(page)).toContain("Record 2: no field changed");
});

test("a page that keeps the event from its default opens no form", async ({ page }) => {
  await open(page);
  await page.evaluate(() => (window.__prevent = true));
  await focusCell(page, 1, 3);
  await page.keyboard.press("Shift+F2");
  expect(await page.evaluate(() => window.__opens)).toHaveLength(1);
  expect(await dialog(page)).toBe(false);
});
