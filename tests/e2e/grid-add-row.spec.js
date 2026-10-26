import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Adding a record from the keyboard (issue #183): the new row after the last
// one, the add on Enter in the last column, the required fields, the page's
// answers.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const rows = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => ({
        cells: [...tr.querySelectorAll('td[part~="cell"]')].map((td) => td.textContent.replace(/\s*\*$/, "")),
        added: tr.getAttribute("data-added"),
        draft: tr.hasAttribute("data-new-row"),
      })));

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-add-row.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(async () => (await rows(page)).length).toBe(3);
}

async function focusNewRow(page) {
  await shadow(page, (root) => root.querySelector('tr[data-new-row] td[data-col="0"]').focus());
}

/** Types one record into the new row, keyboard only, ending with the last column's Enter. */
async function enter(page, id, name, seats) {
  for (const text of [id, name, seats]) {
    await page.keyboard.press("Enter");
    await page.keyboard.type(text);
    await page.keyboard.press("Enter");
  }
}

test("the new row stands after the last row, named and with the required cells marked", async ({ page }) => {
  await open(page);
  const all = await rows(page);
  expect(all.map((row) => row.draft)).toEqual([false, false, true]);
  const marks = await shadow(page, (root) =>
    [...root.querySelectorAll('tr[data-new-row] td[part~="cell"]')].map((td) => td.getAttribute("aria-required")));
  expect(marks).toEqual(["true", "true", null]);
  expect(await shadow(page, (root) => root.querySelector("tr[data-new-row]").getAttribute("aria-label"))).toBe("New row");
  expect(await shadow(page, (root) => root.querySelector("table").getAttribute("aria-rowcount"))).toBe("4");
});

test("three records are added from the keyboard alone, each shown at once and fired once", async ({ page }) => {
  await open(page);
  await focusNewRow(page);
  await enter(page, "3", "C3", "10");
  await enter(page, "4", "D4", "11");
  await enter(page, "5", "E5", "12");

  const added = await page.evaluate(() => window.__added);
  expect(added.map((detail) => detail.values)).toEqual([
    { id: 3, name: "C3", seats: 10 },
    { id: 4, name: "D4", seats: 11 },
    { id: 5, name: "E5", seats: 12 },
  ]);
  expect(new Set(added.map((detail) => detail.provisionalKey)).size).toBe(3);
  // What is typed into the new row is not an edit of a cell.
  expect(await page.evaluate(() => window.__cellChanges ?? 0)).toBe(0);

  const all = await rows(page);
  expect(all.map((row) => row.cells[1])).toEqual(["A1", "B2", "C3", "D4", "E5", ""]);
  expect(all.map((row) => row.added)).toEqual([null, null, "unsaved", "unsaved", "unsaved", null]);
  expect(all.at(-1).draft).toBe(true);
  // The focus is in the first column of the fresh new row, and it is said.
  const focused = await shadow(page, (root) => {
    const active = root.activeElement;
    return { new: active?.closest("tr")?.hasAttribute("data-new-row"), col: active?.getAttribute("data-col") };
  });
  expect(focused).toEqual({ new: true, col: "0" });
  expect(await status(page)).toContain("3 not saved yet");

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("Ctrl+Enter adds from any column, and a missing required value refuses it, naming the cell", async ({ page }) => {
  await open(page);
  await focusNewRow(page);
  // Only the id: the name is required too.
  await page.keyboard.press("Enter");
  await page.keyboard.type("3");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Control+Enter");
  expect(await page.evaluate(() => window.__added)).toHaveLength(0);
  expect(await status(page)).toContain("needs a value");
  const invalid = await shadow(page, (root) =>
    [...root.querySelectorAll('tr[data-new-row] td[part~="cell"]')].map((td) => td.getAttribute("aria-invalid")));
  expect(invalid).toEqual([null, "true", null]);
  // The focus went to the cell that lacks the value.
  expect(await shadow(page, (root) => root.activeElement?.getAttribute("data-col"))).toBe("1");

  await page.keyboard.press("Enter");
  await page.keyboard.type("C3");
  await page.keyboard.press("Control+Enter");
  const added = await page.evaluate(() => window.__added);
  expect(added.map((detail) => detail.values)).toEqual([{ id: 3, name: "C3", seats: null }]);
});

test("the page answers: saved takes the mark off and gives the record its key, an error is said", async ({ page }) => {
  await open(page);
  await focusNewRow(page);
  await enter(page, "3", "C3", "10");
  await enter(page, "4", "D4", "11");
  const [first, second] = await page.evaluate(() => window.__added.map((detail) => detail.provisionalKey));

  await page.evaluate((key) => window.__module.set_row_state(document.querySelector("opengrid-grid"), key, "saved", 103), first);
  await page.evaluate(
    (key) => window.__module.set_row_state(document.querySelector("opengrid-grid"), key, "error", "Room D4 exists already."),
    second,
  );
  const all = await rows(page);
  expect(all.map((row) => row.added)).toEqual([null, null, "saved", "error", null]);
  expect(await status(page)).toContain("Room D4 exists already.");
});

test("a fresh source brings the saved rows and drops them from the added ones", async ({ page }) => {
  await open(page);
  await focusNewRow(page);
  await enter(page, "3", "C3", "10");
  const [key] = await page.evaluate(() => window.__added.map((detail) => detail.provisionalKey));
  await page.evaluate((key) => window.__module.set_row_state(document.querySelector("opengrid-grid"), key, "saved", 3), key);
  await page.evaluate(() =>
    window.__provider.loadRows(
      "rooms",
      [
        { id: 1, name: "A1", seats: 20 },
        { id: 2, name: "B2", seats: 30 },
        { id: 3, name: "C3", seats: 10 },
      ],
      window.__schema,
    ),
  );
  await expect.poll(async () => (await rows(page)).map((row) => row.cells[1])).toEqual(["A1", "B2", "C3", ""]);
  expect((await rows(page)).map((row) => row.added)).toEqual([null, null, null, null]);
});
