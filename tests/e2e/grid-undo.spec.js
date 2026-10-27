import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Undo and redo (issue #186): the grid's own history, the page told with the
// same events and a flag, the cell and the focus back where it happened.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const events = (page) => page.evaluate(() => window.__events);
const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent.replace(/\s*\*$/, ""), { row, col });
const column = (page, col) =>
  shadow(page, (root, col) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => tr.querySelector(`td[data-col="${col}"]`)?.textContent.replace(/\s*\*$/, "")), col);

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-undo.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(async () => (await column(page, 1)).length).toBe(7);
}

async function edit(page, row, col, text) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

test("a cell edit is undone and redone, with the same event and a flag, the focus on the cell", async ({ page }) => {
  await open(page);
  await edit(page, 1, 2, "99");
  expect(await cell(page, 1, 2)).toBe("99");

  await page.keyboard.press("Control+z");
  expect(await cell(page, 1, 2)).toBe("20");
  expect(await status(page)).toContain("Undone: seats in row 2");
  expect(await events(page)).toEqual([
    expect.objectContaining({ type: "opengrid-cell-change", key: 2, column: "seats", value: "99", previous: "20" }),
    expect.objectContaining({ type: "opengrid-cell-change", key: 2, column: "seats", value: "20", previous: "99", undo: true }),
  ]);
  expect(await shadow(page, (root) => `${root.activeElement?.getAttribute("data-row")}:${root.activeElement?.getAttribute("data-col")}`)).toBe("1:2");

  await page.keyboard.press("Control+Shift+z");
  expect(await cell(page, 1, 2)).toBe("99");
  expect(await status(page)).toContain("Redone: seats in row 2");
  expect((await events(page)).at(-1)).toMatchObject({ value: "99", previous: "20", redo: true });

  // Ctrl+Y goes forward too.
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Control+y");
  expect((await events(page)).at(-1)).toMatchObject({ value: "99", redo: true });
});

test("steps come back in reverse, and a new edit ends the way forward", async ({ page }) => {
  await open(page);
  await edit(page, 0, 2, "1");
  await edit(page, 1, 2, "2");
  await edit(page, 2, 2, "3");
  await page.keyboard.press("Control+z");
  await page.keyboard.press("Control+z");
  const undone = (await events(page)).filter((event) => event.undo).map((event) => event.key);
  expect(undone).toEqual([3, 2]);
  expect([await cell(page, 0, 2), await cell(page, 1, 2), await cell(page, 2, 2)]).toEqual(["1", "20", "30"]);

  await edit(page, 3, 2, "4");
  await page.keyboard.press("Control+Shift+z");
  expect(await status(page)).toContain("Nothing to redo");
  expect((await events(page)).filter((event) => event.redo)).toHaveLength(0);
});

test("an added row is undone as its add — the page is told to drop that key — and redone as a new add", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('tr[data-new-row] td[data-col="0"]').focus());
  for (const text of ["7", "R7", "70"]) {
    await page.keyboard.press("Enter");
    await page.keyboard.type(text);
    await page.keyboard.press("Enter");
  }
  expect(await column(page, 1)).toEqual(["R1", "R2", "R3", "R4", "R5", "R6", "R7", ""]);
  const [added] = await events(page);
  expect(added).toMatchObject({ type: "opengrid-row-add", values: { id: 7, name: "R7", seats: 70 } });

  await page.keyboard.press("Control+z");
  expect(await column(page, 1)).toEqual(["R1", "R2", "R3", "R4", "R5", "R6", ""]);
  const undone = (await events(page)).at(-1);
  expect(undone).toMatchObject({ type: "opengrid-row-add", undo: true, provisionalKey: added.provisionalKey });
  expect(await status(page)).toContain("Undone: row added");

  await page.keyboard.press("Control+Shift+z");
  expect(await column(page, 1)).toEqual(["R1", "R2", "R3", "R4", "R5", "R6", "R7", ""]);
  const redone = (await events(page)).at(-1);
  expect(redone).toMatchObject({ type: "opengrid-row-add", redo: true, values: { id: 7, name: "R7" } });
  expect(redone.provisionalKey).not.toBe(added.provisionalKey);
});

test("a paste is one step: one event back, cells and rows together", async ({ page }) => {
  await open(page);
  // Row 4 (R5) and row 5 (R6): both exist, so the block changes four cells.
  await shadow(page, (root) => root.querySelector('td[data-row="4"][data-col="1"]').focus());
  await page.evaluate(() => {
    const text = "P1\t5\nP2\t6\n";
    const data = new DataTransfer();
    data.setData("text/plain", text);
    const root = document.querySelector("opengrid-grid").shadowRoot;
    // Firefox reads `dataType` and `data`, not `clipboardData` (grid-paste.spec.js).
    root.activeElement.dispatchEvent(new ClipboardEvent("paste", {
      clipboardData: data, dataType: "text/plain", data: text, bubbles: true, cancelable: true, composed: true,
    }));
  });
  await expect.poll(() => shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent)).toBe("2 rows");
  await page.keyboard.press("Enter");
  expect(await cell(page, 4, 1)).toBe("P1");
  expect(await cell(page, 5, 2)).toBe("6");

  await page.keyboard.press("Control+z");
  expect([await cell(page, 4, 1), await cell(page, 5, 1), await cell(page, 4, 2), await cell(page, 5, 2)]).toEqual(["R5", "R6", "50", "60"]);
  const pastes = (await events(page)).filter((event) => event.type === "opengrid-paste");
  expect(pastes).toHaveLength(2);
  expect(pastes[1]).toMatchObject({ undo: true });
  expect(pastes[1].changes).toHaveLength(4);
  expect(pastes[1].changes[0]).toMatchObject({ column: "name", value: "R5", previous: "P1" });
  expect(await status(page)).toContain("Undone: paste, 4 values");
});

test("nothing to undo at first, and a fresh source ends the history", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="2"]').focus());
  await page.keyboard.press("Control+z");
  expect(await status(page)).toContain("Nothing to undo");

  await edit(page, 0, 2, "5");
  await page.evaluate(() => window.__provider.loadRows("rooms", window.__rows, window.__schema));
  await expect.poll(() => cell(page, 0, 2)).toBe("10");
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="2"]').focus());
  await page.keyboard.press("Control+z");
  expect(await status(page)).toContain("Nothing to undo");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
