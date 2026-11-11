import { test, expect } from "@playwright/test";
import { status } from "./helpers.js";

// Issue #236: what the grid shows when a sort or a filter answer arrives while
// the reader has scrolled, and when answers arrive out of the order they were
// asked. The queries are held back by the fixture and let go by the test, so no
// timer and no frame decides the order (see grid-focus-result.spec.js).

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(
        document.querySelector("opengrid-grid").shadowRoot,
        arg,
      ),
    { fn: fn.toString(), arg },
  );

/**
 * The ids and customers of the rows the grid shows, by logical row. A pool row
 * with no `aria-rowindex` is parked: it is in the DOM and shows nothing.
 */
const drawn = (page) =>
  shadow(page, (root) => {
    const rows = new Map();
    for (const td of root.querySelectorAll("tbody td[data-row]")) {
      if (!td.closest("tr").getAttribute("aria-rowindex")) continue;
      const row = Number(td.dataset.row);
      const entry = rows.get(row) ?? {};
      entry[td.dataset.col] = td.textContent.trim();
      rows.set(row, entry);
    }
    return [...rows].sort((a, b) => a[0] - b[0]).map(([row, cells]) => ({ row, id: cells[0], customer: cells[1] }));
  });

/** Where the viewport stands, and which logical rows it covers. */
const viewport = (page) =>
  shadow(page, (root) => {
    const node = root.querySelector('[part="viewport"]');
    const rowHeight = 32;
    return {
      top: node.scrollTop,
      max: node.scrollHeight - node.clientHeight,
      first: Math.floor(node.scrollTop / rowHeight),
      last: Math.floor((node.scrollTop + node.clientHeight - 1) / rowHeight),
      rowcount: Number(root.querySelector("table").getAttribute("aria-rowcount")) - 1,
    };
  });

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-late.html");
  await page.evaluate(() => window.__opengridReady);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  await expect.poll(() => status(page)).toBe("60 matches");
}

const scrollTo = (page, top) =>
  shadow(page, (root, top) => (root.querySelector('[part="viewport"]').scrollTop = top), top);

async function filterCustomer(page, text) {
  await shadow(page, (root) => {
    const input = root.querySelector('input[part="filter-value"][data-col="1"]');
    input.focus();
    input.select();
  });
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

/** Every row the viewport covers is drawn, and none beyond the result is. */
async function expectCovered(page) {
  const view = await viewport(page);
  const rows = (await drawn(page)).map((entry) => entry.row);
  const last = Math.min(view.last, view.rowcount - 1);
  for (let row = view.first; row <= last; row++) {
    expect(rows, `row ${row} is in the viewport, so it is drawn`).toContain(row);
  }
  expect(Math.max(...rows)).toBeLessThan(view.rowcount);
  expect(view.top).toBeLessThanOrEqual(view.max);
}

test("a filter that leaves fewer rows than the reader had scrolled past draws what is in the viewport", async ({
  page,
}) => {
  await open(page);
  await scrollTo(page, 60 * 32);
  await expect.poll(async () => (await viewport(page)).first).toBeGreaterThan(40);

  await filterCustomer(page, "Kunde 1");
  await expect.poll(() => status(page)).toBe("10 matches");

  await expect.poll(() => drawn(page).then((rows) => rows.length)).toBeGreaterThan(0);
  await expectCovered(page);
  // Nothing the filter excluded is left standing in a recycled row.
  const rows = await drawn(page);
  for (const row of rows) expect(row.customer).toMatch(/^Kunde 1/);
});

test("a sort draws the rows of the new order, wherever the reader had scrolled to", async ({ page }) => {
  await open(page);
  await scrollTo(page, 30 * 32);
  await expect.poll(async () => (await viewport(page)).first).toBe(30);

  // The first sort is by id, ascending; Enter on its header turns it around.
  await shadow(page, (root) => root.querySelector('th[data-col="0"]').focus());
  await page.keyboard.press("Enter");
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('th[data-col="0"]').getAttribute("aria-sort")))
    .toBe("descending");

  // The header takes the focus to the top; whatever the scroll then is, the
  // rows drawn for it are the rows of the new order, not leftovers of the old.
  await expect.poll(() => drawn(page).then((rows) => rows[0]?.id)).not.toBeUndefined();
  await expectCovered(page);
  for (const { row, id } of await drawn(page)) {
    expect(Number(id), `row ${row} of 60 in descending order`).toBe(60 - row);
  }
});

test("a filter answer that arrives after a newer one does not replace it", async ({ page }) => {
  await open(page);
  await page.evaluate(() => (window.__hold = true));

  await filterCustomer(page, "Kunde 1");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBeGreaterThan(0);
  const first = await page.evaluate(() => window.__held.length);
  await filterCustomer(page, "Kunde 55");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBeGreaterThan(first);

  // The newer question is answered first, the older one afterwards.
  await page.evaluate(() => {
    window.__hold = false;
    const held = window.__held.splice(0);
    for (const release of held.reverse()) release();
  });

  await expect.poll(() => status(page)).toBe("1 match");
  // Give a late answer the chance it would need to overwrite the newer one.
  await page.waitForTimeout(200);
  expect(await status(page)).toBe("1 match");
  expect((await drawn(page)).map((row) => row.customer)).toEqual(["Kunde 55"]);
});

test("a sort answer that arrives after a filter does not bring the old rows back", async ({ page }) => {
  await open(page);
  await page.evaluate(() => (window.__hold = true));

  await shadow(page, (root) => root.querySelector('th[data-col="0"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBeGreaterThan(0);
  const first = await page.evaluate(() => window.__held.length);
  await filterCustomer(page, "Kunde 55");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBeGreaterThan(first);

  await page.evaluate(() => {
    window.__hold = false;
    const held = window.__held.splice(0);
    for (const release of held.reverse()) release();
  });

  await expect.poll(() => status(page)).toBe("1 match");
  await page.waitForTimeout(200);
  expect(await status(page)).toBe("1 match");
  expect((await drawn(page)).map((row) => row.customer)).toEqual(["Kunde 55"]);
});
