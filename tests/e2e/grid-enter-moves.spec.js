import { test, expect } from "@playwright/test";
import { shadow } from "./helpers.js";

// `enter-moves="down"` (issue #536): an `Enter` that commits an editor moves the
// focus to the same column of the next data row — `Shift`+`Enter` of the
// previous one — and opens nothing. Typing a column of values is
// `type`, `Enter`, `type`, `Enter`.

const QTY = 2;

const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
/** Where the focus is: `[row, col]` of the cell, or the tag of what else has it. */
const focused = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active?.matches("td[data-row]") ? [Number(active.dataset.row), Number(active.dataset.col)] : active?.tagName;
  });
const editorOpen = (page) => shadow(page, (root) => root.querySelector('[part="editor"]') !== null);
const text = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });
const changes = (page) => page.evaluate(() => window.__changes.map(({ key, column, value }) => ({ key, column, value })));

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-enter-moves.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => text(page, 0, 0)).not.toBeUndefined();
}

/** Opens the editor of the cell that has the focus, types `value` and presses `key`. */
async function typeInto(page, value, key = "Enter") {
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type(value);
  await page.keyboard.press(key);
}

test("Enter commits and moves to the same column of the next row, and opens nothing", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, QTY);
  await typeInto(page, "11");

  expect(await focused(page)).toEqual([1, QTY]);
  expect(await editorOpen(page)).toBe(false);
  expect(await text(page, 0, QTY)).toBe("11");
  expect(await changes(page)).toEqual([{ key: 1, column: "qty", value: "11" }]);

  // A second Enter opens it, and its Enter commits and moves on again.
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(true);
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("22");
  await page.keyboard.press("Enter");
  expect(await focused(page)).toEqual([2, QTY]);
  expect(await editorOpen(page)).toBe(false);
  expect(await changes(page)).toEqual([
    { key: 1, column: "qty", value: "11" },
    { key: 2, column: "qty", value: "22" },
  ]);
});

test("a column of values: type, Enter, type, Enter", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, QTY);
  for (const value of ["10", "20", "30"]) {
    await typeInto(page, value);
  }
  expect(await changes(page)).toEqual([
    { key: 1, column: "qty", value: "10" },
    { key: 2, column: "qty", value: "20" },
    { key: 3, column: "qty", value: "30" },
  ]);
  expect(await focused(page)).toEqual([3, QTY]);
});

test("Shift+Enter commits and moves up", async ({ page }) => {
  await open(page);
  await focusCell(page, 3, QTY);
  await typeInto(page, "44", "Shift+Enter");
  expect(await focused(page)).toEqual([2, QTY]);
  expect(await editorOpen(page)).toBe(false);
  expect(await changes(page)).toEqual([{ key: 4, column: "qty", value: "44" }]);
});

test("at the last row the focus stays, and at the first one with Shift", async ({ page }) => {
  await open(page, "?rows=3");
  await focusCell(page, 2, QTY);
  await typeInto(page, "7");
  expect(await focused(page)).toEqual([2, QTY]);
  expect(await editorOpen(page)).toBe(false);
  expect(await text(page, 2, QTY)).toBe("7");

  await focusCell(page, 0, QTY);
  await typeInto(page, "5", "Shift+Enter");
  expect(await focused(page)).toEqual([0, QTY]);
  expect(await editorOpen(page)).toBe(false);
  expect(await text(page, 0, QTY)).toBe("5");
});

test("a validator that blocks the commit means no move", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, QTY);
  await typeInto(page, "99");
  // The editor is still open on the cell, with the sentence under it, and nothing was saved.
  expect(await editorOpen(page)).toBe(true);
  expect(await focused(page)).toBe("INPUT");
  expect(await text(page, 1, QTY)).toBe("2");
  expect(await changes(page)).toEqual([]);

  // Corrected, the same Enter commits and moves.
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("9");
  await page.keyboard.press("Enter");
  expect(await focused(page)).toEqual([1, QTY]);
  expect(await changes(page)).toEqual([{ key: 1, column: "qty", value: "9" }]);
});

test("a value the column refuses is not committed, so nothing moves", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, QTY);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");
  // `qty` may not be empty: the editor closes on the old value, and nothing moved.
  expect(await changes(page)).toEqual([]);
  expect(await focused(page)).toEqual([0, QTY]);
  expect(await text(page, 0, QTY)).toBe("1");
});

test("Escape discards and does not move", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, QTY);
  await page.keyboard.press("Enter");
  await page.keyboard.type("5");
  await page.keyboard.press("Escape");
  expect(await focused(page)).toEqual([1, QTY]);
  expect(await changes(page)).toEqual([]);
});

test("the move passes a group header, and the total", async ({ page }) => {
  await open(page, "?group");
  // Alpha opens (rows 3 and 6 follow its header), then Beta, which sits at row 3.
  await focusCell(page, 0, 0);
  await page.keyboard.press("Enter");
  await expect.poll(() => text(page, 1, 0)).toBe("3");
  await focusCell(page, 3, 0);
  await page.keyboard.press("Enter");
  await expect.poll(() => text(page, 4, 0)).toBe("1");
  // 0 Alpha · 1 id 3 · 2 id 6 · 3 Beta · 4 id 1 · 5 id 4 · 6 Gamma · 7 Total

  // Down from the last row of Alpha: past Beta's header to the first row of Beta.
  await focusCell(page, 2, QTY);
  await typeInto(page, "60");
  expect(await focused(page)).toEqual([4, QTY]);
  expect(await changes(page)).toEqual([{ key: 6, column: "qty", value: "60" }]);

  // Up again, past the header.
  await typeInto(page, "10", "Shift+Enter");
  expect(await focused(page)).toEqual([2, QTY]);

  // The first row of Alpha has only a header above it, and the last of Beta only
  // a closed group and the total below it: the focus stays.
  await focusCell(page, 1, QTY);
  await typeInto(page, "30", "Shift+Enter");
  expect(await focused(page)).toEqual([1, QTY]);
  await focusCell(page, 5, QTY);
  await typeInto(page, "40");
  expect(await focused(page)).toEqual([5, QTY]);
  expect(await editorOpen(page)).toBe(false);
});

test("a column typed far past the rendered rows: every value lands on its record, the cell is in sight", async ({ page }) => {
  await open(page, "?rows=200");
  await focusCell(page, 0, QTY);
  // Forty rows down: well past the twelve the grid draws at once.
  for (let row = 0; row < 40; row++) {
    await typeInto(page, String(1000 + row));
    await expect.poll(() => focused(page)).toEqual([row + 1, QTY]);
  }
  expect(await editorOpen(page)).toBe(false);
  const done = await changes(page);
  expect(done).toHaveLength(40);
  expect(done[39]).toEqual({ key: 40, column: "qty", value: "1039" });
  expect(done.every((change, row) => change.key === row + 1 && change.value === String(1000 + row))).toBe(true);
  // The cell that has the focus is in the viewport.
  const visible = await shadow(page, (root) => {
    const cell = root.activeElement.getBoundingClientRect();
    const view = root.querySelector('[part="viewport"]').getBoundingClientRect();
    return cell.top >= view.top && cell.bottom <= view.bottom;
  });
  expect(visible).toBe(true);
  // And back up with Shift+Enter, past the same boundary.
  await typeInto(page, "5", "Shift+Enter");
  await expect.poll(() => focused(page)).toEqual([39, QTY]);
});

test("a choice column commits with Enter and moves on, too", async ({ page }) => {
  await open(page, "?choices");
  await focusCell(page, 0, 1);
  await page.keyboard.press("Enter");
  await page.keyboard.type("Gam");
  await page.keyboard.press("Enter");
  expect(await changes(page)).toEqual([{ key: 1, column: "customer", value: "Gamma" }]);
  expect(await focused(page)).toEqual([1, 1]);
  expect(await editorOpen(page)).toBe(false);
});

test("under edit-locked nothing opens, and nothing moves", async ({ page }) => {
  await open(page, "?locked");
  await focusCell(page, 0, QTY);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(false);
  expect(await focused(page)).toEqual([0, QTY]);
  await page.keyboard.press("Shift+Enter");
  expect(await editorOpen(page)).toBe(false);
  expect(await focused(page)).toEqual([0, QTY]);
  expect(await changes(page)).toEqual([]);
});

test("without the attribute Enter commits and the focus stays", async ({ page }) => {
  await open(page, "?plain");
  await focusCell(page, 0, QTY);
  await typeInto(page, "11");
  expect(await focused(page)).toEqual([0, QTY]);
  expect(await text(page, 0, QTY)).toBe("11");
  await typeInto(page, "12", "Shift+Enter");
  expect(await focused(page)).toEqual([0, QTY]);
});

test("another value of the attribute is none", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("enter-moves", "right"));
  await focusCell(page, 0, QTY);
  await typeInto(page, "11");
  expect(await focused(page)).toEqual([0, QTY]);
});
