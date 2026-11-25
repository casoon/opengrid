import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// Issue #529: `row-key` may name a field outside `columns`. The key travels
// with every query that names records but is never drawn: not in the header,
// the column list, the filter row, exports or the view's columns. Events,
// selection, saving, undo and the detail panel name records by it as with a
// shown key. The fixture's ids run backwards from 1000, so a row's place never
// equals its key.

async function open(page, search = "") {
  await page.goto(`/tests/e2e/fixtures/grid-hidden-key.html${search}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

const headers = (page) =>
  shadow(page, (root) => [...root.querySelectorAll("thead th")].map((th) => th.textContent.trim()));

/** Every data row drawn: its place, its cells' text and whether it is selected. */
const rows = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr[aria-rowindex]")]
      .filter((tr) => !tr.hasAttribute("data-kind"))
      .map((tr) => ({
        row: Number(tr.querySelector("td[data-row]").getAttribute("data-row")),
        // A cell's own text, without the detail toggle drawn in the first one.
        cells: [...tr.querySelectorAll("td[data-col]")].map((td) =>
          [...td.childNodes]
            .filter((node) => !(node instanceof Element && node.matches("button")))
            .map((node) => node.textContent)
            .join("")
            .trim(),
        ),
        selected: tr.getAttribute("aria-selected"),
      })),
  );

async function edit(page, row, col, text) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), {
    row,
    col,
  });
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

const cellText = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, {
    row,
    col,
  });

test("the key is never drawn, yet an edit names its record and the page's answer finds the cell", async ({ page }) => {
  await open(page);
  expect((await headers(page)).join(" ")).not.toMatch(/\bid\b/i);
  const first = (await rows(page))[0];
  expect(first.cells).toEqual(["Room 01", "1"]);

  await edit(page, 0, 0, "Hall");
  const changes = await page.evaluate(() => window.__changes);
  expect(changes).toHaveLength(1);
  expect(changes[0]).toMatchObject({ row: 0, key: 1000, column: "name", value: "Hall" });

  await page.evaluate(() =>
    window.__module.set_cell_state(document.querySelector("opengrid-grid"), 1000, "name", "saved"),
  );
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').getAttribute("data-state")))
    .toBe("saved");

  await page.evaluate(() =>
    window.__module.set_values(document.querySelector("opengrid-grid"), [{ key: 999, column: "seats", value: 9 }]),
  );
  await expect.poll(() => cellText(page, 1, 1)).toBe("9");

  // Undo restores the record's old value and tells the page by its key again.
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').focus());
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => cellText(page, 0, 0)).toBe("Room 01");
  const undone = await page.evaluate(() => window.__changes.at(-1));
  expect(undone).toMatchObject({ key: 1000, column: "name", value: "Room 01" });
});

test("Space and a view select a record by its key, and the view does not name it", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="2"][data-col="0"]').focus());
  await page.keyboard.press("Space");
  await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.keys)).toEqual([998]);
  const view = await page.evaluate(() => window.__module.get_view(document.querySelector("opengrid-grid")));
  expect(view.selected).toEqual([998]);
  expect(JSON.stringify(view.columns ?? {})).not.toMatch(/"id"/);

  // A selection by key the page sets is drawn on the right row.
  await page.evaluate(() =>
    window.__module.set_view(document.querySelector("opengrid-grid"), { selected: [996] }),
  );
  await expect
    .poll(async () => (await rows(page)).filter((r) => r.selected === "true").map((r) => r.cells[0]))
    .toEqual(["Room 05"]);
});

test("every row of the filter is selected by its key", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').focus());
  await page.keyboard.press("ControlOrMeta+a");
  await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.count)).toBe(40);
  const keys = await page.evaluate(() => [...window.__selections.at(-1).keys].sort((a, b) => a - b));
  expect(keys).toEqual(Array.from({ length: 40 }, (_, index) => 961 + index));
});

test("the column list and the filter row leave the key out", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('[part="columns-toggle"]')?.click());
  const listed = await shadow(page, (root) =>
    [...root.querySelectorAll("input[data-column]")].map((input) => input.getAttribute("data-column")),
  );
  expect(listed).toEqual(["name", "seats"]);
});

test("the detail panel opens by key", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="3"] button[data-detail-toggle]').click());
  await expect.poll(() => page.evaluate(() => window.__details.at(-1)?.key)).toBe(997);
  const record = await page.evaluate(() => window.__details.at(-1).record);
  expect(record.name).toBe("Room 04");
});

test("grouped: an edit and a selection name the record by its key", async ({ page }) => {
  await open(page, "?group");
  // The groups seats 1 … 4 and the total; the first group holds rooms 01, 05, …
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await rows(page))[0]?.cells[0]).toBe("Room 01");
  const second = (await rows(page))[1];
  expect(second.cells[0]).toBe("Room 05");
  await edit(page, second.row, 0, "Lab");
  expect(await page.evaluate(() => window.__changes.at(-1))).toMatchObject({ key: 996, column: "name", value: "Lab" });
  await shadow(page, (root, row) => root.querySelector(`td[data-row="${row}"][data-col="0"]`).focus(), second.row);
  await page.keyboard.press("Space");
  await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.keys)).toEqual([996]);
  expect((await headers(page)).join(" ")).not.toMatch(/\bid\b/i);
});

test("get_query, the page's export, does not ask for the hidden key", async ({ page }) => {
  await open(page);
  const query = await page.evaluate(() => window.__module.get_query(document.querySelector("opengrid-grid")));
  expect(query.select).toEqual(["name", "seats"]);
});

test("paged: the second page still names records by key", async ({ page }) => {
  await open(page, "?paged");
  expect((await rows(page)).map((r) => r.cells[0])).toEqual(["Room 01", "Room 02", "Room 03", "Room 04", "Room 05"]);
  await shadow(page, (root) => root.querySelector('[part="page-next"]').click());
  await expect.poll(async () => (await rows(page))[0]?.cells[0]).toBe("Room 06");
  const row = (await rows(page))[0].row;
  await edit(page, row, 0, "Annex");
  expect(await page.evaluate(() => window.__changes.at(-1))).toMatchObject({ key: 995, column: "name", value: "Annex" });
});

test("has no axe violations", async ({ page }) => {
  await open(page);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("a row-key that is no field of the source is said in the status line", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-hidden-key.html?unknown");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => status(page)).toMatch(/nope/);
});
