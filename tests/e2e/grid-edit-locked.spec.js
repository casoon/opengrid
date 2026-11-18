import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// A grid-wide lock (issue #494): `edit-locked` on the element. Every way into
// an edit asks the same question, and the attribute is the whole state.

const events = (page) => page.evaluate(() => window.__events);
const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent.replace(/\s*\*$/, ""), { row, col });
const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
const lock = (page, on) =>
  page.evaluate((on) => document.querySelector("opengrid-grid").toggleAttribute("edit-locked", on), on);
const rowCount = (page) => shadow(page, (root) => root.querySelectorAll("tbody tr[aria-rowindex]").length);
const editorOpen = (page) => shadow(page, (root) => root.querySelector('[part="editor"]') !== null);
const LOCKED = "Editing is locked";

async function open(page, locked = true) {
  await page.goto("/tests/e2e/fixtures/grid-edit-locked.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => rowCount(page)).toBe(6);
  if (!locked) {
    await lock(page, false);
    await expect.poll(() => rowCount(page)).toBe(7);
  }
}

async function edit(page, row, col, text) {
  await focusCell(page, row, col);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

async function pasteInto(page, text) {
  await page.evaluate((text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    const event = new ClipboardEvent("paste", {
      clipboardData: data, dataType: "text/plain", data: text, bubbles: true, cancelable: true, composed: true,
    });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
  }, text);
}

test("a locked grid says so: aria-readonly on the grid and its cells, no new row", async ({ page }) => {
  await open(page);
  expect(await shadow(page, (root) => root.querySelector('table[role="grid"]').getAttribute("aria-readonly"))).toBe("true");
  expect(await shadow(page, (root) => root.querySelector('td[data-col="1"]').getAttribute("aria-readonly"))).toBe("true");
  // Six records, and no empty row at the end to type a seventh into.
  expect(await rowCount(page)).toBe(6);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("Enter, F2 and a typed character open no editor, and the refusal is said", async ({ page }) => {
  await open(page);
  for (const press of ["Enter", "F2", "x"]) {
    await focusCell(page, 1, 2);
    await page.keyboard.press(press);
    expect(await editorOpen(page)).toBe(false);
    await expect.poll(() => status(page)).toContain(LOCKED);
  }
  expect(await events(page)).toEqual([]);
});

test("paste is refused with a preview of nothing", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 1);
  await pasteInto(page, "A\tB\r\n");
  await expect.poll(() => status(page)).toContain(LOCKED);
  expect(await shadow(page, (root) => root.querySelector('[part="paste-summary"]') !== null)).toBe(false);
  expect(await events(page)).toEqual([]);
});

test("the bulk panel and the record form do not open", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('tr[aria-rowindex="2"] td[data-select="row"]').focus());
  await page.keyboard.press(" ");
  await focusCell(page, 1, 2);
  await page.keyboard.press("F2");
  await expect.poll(() => status(page)).toContain(LOCKED);
  expect(await shadow(page, (root) => root.querySelector('[part="bulk"]') !== null)).toBe(false);

  await focusCell(page, 1, 2);
  await page.keyboard.press("Shift+F2");
  expect(await shadow(page, (root) => root.querySelector('[part="record-dialog"]') !== null)).toBe(false);
  expect(await events(page)).toEqual([]);
});

test("unlocking restores editing, and sort, filter, data and focus stay where they were", async ({ page }) => {
  await open(page);
  // Sort by seats, descending: the reader's state the lock must not touch.
  await shadow(page, (root) => root.querySelector('th[data-col="2"]').focus());
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  const before = await shadow(page, (root) => [...root.querySelectorAll('td[data-col="1"]')].map((td) => td.textContent));
  const sort = await shadow(page, (root) => root.querySelector('th[data-col="2"]').getAttribute("aria-sort"));
  await focusCell(page, 2, 1);

  await lock(page, false);
  await expect.poll(() => rowCount(page)).toBe(7);
  expect(await shadow(page, (root) => root.querySelector('table[role="grid"]').getAttribute("aria-readonly"))).toBeNull();
  expect(await shadow(page, (root) => root.querySelector('td[data-col="1"]').getAttribute("aria-readonly"))).toBeNull();
  expect(await shadow(page, (root) => root.querySelector('th[data-col="2"]').getAttribute("aria-sort"))).toBe(sort);
  expect(await shadow(page, (root) => `${root.activeElement?.getAttribute("data-row")}:${root.activeElement?.getAttribute("data-col")}`)).toBe("2:1");
  expect(await shadow(page, (root) => [...root.querySelectorAll('td[data-col="1"]')].slice(0, 6).map((td) => td.textContent))).toEqual(before.slice(0, 6));

  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(true);
  await page.keyboard.press("Escape");
  await lock(page, true);
  await expect.poll(() => rowCount(page)).toBe(6);
  expect(await shadow(page, (root) => `${root.activeElement?.getAttribute("data-row")}:${root.activeElement?.getAttribute("data-col")}`)).toBe("2:1");
});

test("an editor open when the lock is set is cancelled, not committed", async ({ page }) => {
  await open(page, false);
  await focusCell(page, 1, 2);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("999");
  await lock(page, true);
  await expect.poll(() => editorOpen(page)).toBe(false);
  expect(await cell(page, 1, 2)).toBe("20");
  expect(await events(page)).toEqual([]);
});

test("undo after a re-lock changes nothing; unlocked, it still can", async ({ page }) => {
  await open(page, false);
  await edit(page, 1, 2, "99");
  expect(await cell(page, 1, 2)).toBe("99");
  await lock(page, true);
  await focusCell(page, 1, 2);
  await page.keyboard.press("Control+z");
  expect(await cell(page, 1, 2)).toBe("99");
  await expect.poll(() => status(page)).toContain(LOCKED);
  expect(await events(page)).toHaveLength(1);
  await page.keyboard.press("Control+y");
  expect(await events(page)).toHaveLength(1);

  await lock(page, false);
  await focusCell(page, 1, 2);
  await page.keyboard.press("Control+z");
  expect(await cell(page, 1, 2)).toBe("20");
});

test("unlocked, a column's own readonly still holds, and a new row can be typed", async ({ page }) => {
  await open(page, false);
  await page.evaluate(() =>
    window.__module.set_columns(document.querySelector("opengrid-grid"), { seats: { readonly: true } }),
  );
  await focusCell(page, 1, 2);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(false);
  await focusCell(page, 1, 1);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(true);
  await page.keyboard.press("Escape");
  expect(await rowCount(page)).toBe(7);
});
