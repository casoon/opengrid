import { test, expect } from "@playwright/test";

// Issue #236: one keyboard-only walk through everything a grid offers — toolbar,
// filter row, header, cells, editor, pager — in the order a reader meets it, and
// back. The other specs each drive one part with the keyboard; none of them
// proves the parts follow one another without a trap or a gap between them.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(
        document.querySelector("opengrid-grid").shadowRoot,
        arg,
      ),
    { fn: fn.toString(), arg },
  );

/** Where the focus stands: inside the grid by `part`, or by id outside it. */
const where = (page) =>
  page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    const inside = host.shadowRoot.activeElement;
    if (inside) {
      return inside.getAttribute("part") ?? `density-${inside.dataset.density}`;
    }
    return document.activeElement?.id || document.activeElement?.tagName.toLowerCase() || null;
  });

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const ids = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll('tbody td[data-col="0"]')]
      .filter((td) => td.closest("tr").getAttribute("aria-rowindex"))
      .map((td) => td.textContent.trim()),
  );

/** Presses `key` and returns where the focus is afterwards. */
async function press(page, key) {
  await page.keyboard.press(key);
  return where(page);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-keyboard.html");
  await page.waitForFunction(() => window.__ready);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  await expect.poll(() => status(page)).toBe("5 matches");
});

test("Tab reaches every part once, in reading order, and leaves the grid at the end", async ({ page }) => {
  await page.focus("#before");
  const stops = [];
  for (let step = 0; step < 40; step++) {
    const stop = await press(page, "Tab");
    stops.push(stop);
    if (stop === "after") break;
  }

  expect(stops).toEqual([
    // The toolbar.
    "add-filter",
    "add-grouping",
    "filter-row-toggle",
    "columns-toggle",
    "density-compact",
    "density-normal",
    "density-comfortable",
    // The filter row: an operator and a value per column, and the way to clear.
    ...[0, 1, 2, 3].flatMap(() => ["filter-operator", "filter-value"]),
    "filter-clear",
    // The data: one tab stop for the whole table, the rest by arrow keys.
    "header",
    // The pager. First and previous are disabled on the first page and skipped.
    "page-next",
    "page-last",
    "after",
  ]);
});

test("Shift+Tab walks the same stops backwards", async ({ page }) => {
  await page.focus("#after");
  const stops = [];
  for (let step = 0; step < 40; step++) {
    const stop = await press(page, "Shift+Tab");
    stops.push(stop);
    if (stop === "add-filter") break;
  }
  // Last page, next page, the header, the filter row backwards, then the toolbar.
  expect(stops.slice(0, 3)).toEqual(["page-last", "page-next", "header"]);
  expect(stops.at(-1)).toBe("add-filter");
  expect(stops).toContain("filter-clear");
  expect(stops.filter((stop) => stop === "filter-value")).toHaveLength(4);
});

test("filter, header, cells, editor, Escape, commit and pager in one keyboard-only walk", async ({ page }) => {
  await page.focus("#before");
  // Into the filter row: the value field of `customer`.
  const column = () => shadow(page, (root) => root.activeElement.dataset.col ?? null);
  for (let step = 0; step < 40; step++) {
    await page.keyboard.press("Tab");
    if ((await where(page)) === "filter-value" && (await column()) === "1") break;
  }
  expect(await column()).toBe("1");
  await page.keyboard.type("Alpha");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("2 matches");
  await expect.poll(() => ids(page)).toEqual(["2", "4"]);
  // The reader is still in the field they typed in.
  expect(await where(page)).toBe("filter-value");

  // On to the table: the filter row ends, the header is the next stop.
  for (let step = 0; step < 10 && (await where(page)) !== "header"; step++) await page.keyboard.press("Tab");
  expect(await where(page)).toBe("header");

  // Arrow keys take over inside the table: down to the first cell, right to `customer`.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowRight");
  expect(await shadow(page, (root) => `${root.activeElement.tagName}:${root.activeElement.dataset.row}:${root.activeElement.dataset.col}`)).toBe(
    "TD:0:1",
  );

  // Enter opens the editor; Escape closes it, changes nothing, and gives the focus back to the cell.
  await page.keyboard.press("Enter");
  await expect.poll(() => shadow(page, (root) => !!root.querySelector('[part="editor"]'))).toBe(true);
  await page.keyboard.type("Zeta");
  await page.keyboard.press("Escape");
  await expect.poll(() => shadow(page, (root) => !!root.querySelector('[part="editor"]'))).toBe(false);
  expect(await shadow(page, (root) => `${root.activeElement.tagName}:${root.activeElement.dataset.row}:${root.activeElement.dataset.col}`)).toBe(
    "TD:0:1",
  );
  expect(await page.evaluate(() => window.__changes.length)).toBe(0);
  expect(await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="1"]').textContent)).toContain("Alpha");

  // The same again, committed: the page hears of it, the focus is on the cell.
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("Zeta");
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__changes.length)).toBe(1);
  expect(await page.evaluate(() => window.__changes[0])).toMatchObject({ column: "customer", value: "Zeta" });
  expect(await shadow(page, (root) => root.activeElement.tagName)).toBe("TD");

  // Tab leaves the table for the pager (two matches, page size two: one page, so
  // the pager has nothing to offer and the focus goes straight on).
  expect(await press(page, "Tab")).toBe("after");
});

test("the pager is reached and used with the keyboard alone", async ({ page }) => {
  await page.focus("#before");
  let stop = "";
  for (let step = 0; step < 40 && stop !== "page-next"; step++) stop = await press(page, "Tab");
  expect(stop).toBe("page-next");

  await page.keyboard.press("Enter");
  await expect.poll(() => ids(page)).toEqual(["3", "4"]);
  // The button that was pressed is still where the focus is, and now the pager
  // offers the way back as well.
  expect(await where(page)).toBe("page-next");
  expect(await shadow(page, (root) => root.querySelector('[part="page-previous"]').hasAttribute("disabled"))).toBe(false);

  await page.keyboard.press("Space");
  await expect.poll(() => ids(page)).toEqual(["5"]);
});

// The button that was pressed is the one that is disabled by the press: on the
// last page "next" has nothing to offer. A browser moves the focus off a
// disabled control once it has drawn a frame, to the document.
test.fixme("the focus does not fall to the document when the pager disables the button it stands on (#285)", async ({
  page,
}) => {
  await page.focus("#before");
  let stop = "";
  for (let step = 0; step < 40 && stop !== "page-next"; step++) stop = await press(page, "Tab");
  await page.keyboard.press("Enter");
  await expect.poll(() => ids(page)).toEqual(["3", "4"]);
  await page.keyboard.press("Space");
  await expect.poll(() => ids(page)).toEqual(["5"]);
  // Two frames: the point where a browser drops the focus from a disabled button.
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  expect(await where(page)).not.toBe("body");
});
