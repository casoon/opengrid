import { test, expect } from "@playwright/test";

// A source loaded again, and a grid rebuilt around the same records (issue #226).
// The fixture shows one source in a grid, a tree, a table and a pivot.

const shadow = (page, id, fn, arg) =>
  page.evaluate(
    ({ id, fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(document.getElementById(id).shadowRoot, arg),
    { id, fn: fn.toString(), arg },
  );

/** A grid's column, by its schema index. */
const column = (page, id, col) =>
  shadow(
    page,
    id,
    (root, col) =>
      [...root.querySelectorAll("tbody tr[aria-rowindex]")].map((tr) =>
        tr.querySelector(`td[data-col="${col}"]`)?.textContent.replace(/\s*\*$/, ""),
      ),
    col,
  );
/** A table's column, by its place. */
const tableColumn = (page, id, col) =>
  shadow(page, id, (root, col) =>
    [...root.querySelectorAll("tbody tr")].map((tr) => tr.querySelectorAll("td")[col]?.textContent), col);
const rowsOf = (page) =>
  shadow(page, "plain", (root) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => ({
        cells: [...tr.querySelectorAll('td[part~="cell"]')].map((td) => td.textContent.replace(/\s*\*$/, "")),
        added: tr.getAttribute("data-added"),
        draft: tr.hasAttribute("data-new-row"),
      })));
const status = (page, id) => shadow(page, id, (root) => root.querySelector('[part="status"]').textContent);
const headers = (page) =>
  shadow(page, "plain", (root) =>
    [...root.querySelectorAll("thead th[data-col]")].map((th) => th.querySelector("span")?.textContent ?? th.textContent));

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-replace.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(async () => (await column(page, "plain", 1)).length).toBeGreaterThan(7);
}

const east = (n) =>
  Array.from({ length: n }, (_, index) => ({ id: 100 + index, name: `E${index + 1}`, zone: "east", seats: 1 }));

test("a replaced source clears the selection and says so", async ({ page }) => {
  await open(page);
  await shadow(page, "plain", (root) => root.querySelector('td[data-row="5"][data-col="1"]').focus());
  await page.keyboard.press("Space");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Space");
  expect((await page.evaluate(() => window.__selection)).at(-1)).toEqual([4, 5]);

  await page.evaluate((rows) => window.__replace("rooms", rows), east(3));
  await expect.poll(() => column(page, "plain", 1)).toEqual(["E1", "E2", "E3", ""]);
  expect((await page.evaluate(() => window.__selection)).at(-1)).toEqual([]);
  expect(await shadow(page, "plain", (root) => root.querySelectorAll('tr[aria-selected="true"]').length)).toBe(0);
  expect(await status(page, "plain")).toContain("Selection cleared");
});

test("a replaced source asks a tree for its levels again", async ({ page }) => {
  await open(page);
  const names = () => column(page, "tree", 0);
  await expect.poll(names).toEqual(["Alpha", "Beta"]);
  await page.evaluate(() => window.__replace("units", [
    { id: 10, parent_id: null, name: "Gamma", zone: "east" },
    { id: 11, parent_id: null, name: "Delta", zone: "west" },
    { id: 12, parent_id: 11, name: "Delta child", zone: "west" },
  ]));
  await expect.poll(names).toEqual(["Delta", "Gamma"]);
});

const facetValues = (page) =>
  shadow(page, "plain", (root) =>
    [...root.querySelectorAll('fieldset[data-facet="zone"] [part="facet-value"]')].map(
      (item) => item.querySelector("span:not([part])").textContent,
    ));

test("a replaced source lists its own values in a facet", async ({ page }) => {
  await open(page);
  await expect.poll(() => facetValues(page)).toEqual(["north", "south"]);
  await page.evaluate((rows) => window.__replace("rooms", rows), east(3));
  await expect.poll(() => facetValues(page)).toEqual(["east"]);
});

test("another datasource lists its own values in a facet, and drops the added rows", async ({ page }) => {
  await open(page);
  await expect.poll(() => facetValues(page)).toEqual(["north", "south"]);
  await add(page, "20", "X1", "north", "7");
  await expect.poll(async () => (await rowsOf(page)).some((row) => row.added)).toBe(true);
  await page.evaluate((rows) => window.__replace("rooms2", rows), east(3));
  await page.evaluate(() => document.getElementById("plain").setAttribute("datasource", "rooms2"));
  await expect.poll(() => facetValues(page)).toEqual(["east"]);
  await expect.poll(() => column(page, "plain", 1)).toEqual(["E1", "E2", "E3", ""]);
});

test("a replaced source reaches the table and the pivot", async ({ page }) => {
  await open(page);
  await expect.poll(() => tableColumn(page, "table", 1)).toContain("R8");
  await page.evaluate((rows) => window.__replace("rooms", rows), east(3));
  await expect.poll(() => tableColumn(page, "table", 1)).toEqual(["E1", "E2", "E3"]);
  await expect
    .poll(() => shadow(page, "pivot", (root) => root.textContent))
    .toContain("east");
  expect(await shadow(page, "pivot", (root) => root.textContent)).not.toContain("north");
});

/** Types one record into the new row, keyboard only, ending with the last column's Enter. */
async function add(page, ...values) {
  await shadow(page, "plain", (root) => root.querySelector('tr[data-new-row] td[data-col="0"]').focus());
  for (const text of values) {
    await page.keyboard.press("Enter");
    await page.keyboard.type(text);
    await page.keyboard.press("Enter");
  }
}

async function moveSeatsLeft(page) {
  await shadow(page, "plain", (root) => root.querySelector('th[data-col="3"]').focus());
  await page.keyboard.press("Control+ArrowLeft");
  await expect.poll(() => headers(page)).toEqual(["id", "name", "seats", "zone"]);
}

test("moving a column keeps the added row, in the new order, and the page can still answer for it", async ({ page }) => {
  await open(page);
  await add(page, "20", "X1", "north", "7");
  const [{ provisionalKey }] = await page.evaluate(() => window.__added);
  await moveSeatsLeft(page);

  await expect.poll(async () => (await rowsOf(page)).map((row) => row.added)).toContain("unsaved");
  const added = (await rowsOf(page)).find((row) => row.added);
  expect(added.cells).toEqual(["20", "X1", "7", "north"]);

  await page.evaluate((key) => window.__module.set_row_state(document.getElementById("plain"), key, "saved", 20), provisionalKey);
  expect((await rowsOf(page)).find((row) => row.cells[0] === "20").added).toBe("saved");
});

test("moving a column keeps the undo history, and an edit follows its column", async ({ page }) => {
  await open(page);
  await shadow(page, "plain", (root) => root.querySelector('td[data-row="0"][data-col="3"]').focus());
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("99");
  await page.keyboard.press("Enter");
  await add(page, "20", "X1", "north", "7");
  await moveSeatsLeft(page);
  await expect.poll(async () => (await rowsOf(page)).some((row) => row.added)).toBe(true);

  // The add goes first, then the edit: seats was the fourth column and is the
  // third now, and the step went with it.
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await rowsOf(page)).some((row) => row.added)).toBe(false);
  await page.keyboard.press("Control+z");
  await expect
    .poll(() => page.evaluate(() => window.__changes.at(-1)))
    .toMatchObject({ key: 1, column: "seats", value: "10", previous: "99", undo: true });
});
