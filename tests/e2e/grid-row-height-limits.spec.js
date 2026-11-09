import { test, expect } from "@playwright/test";

// Issue #236: the row height after the grid was connected. grid-row-height.spec.js
// covers the value the page sets before the grid exists; these are the changes
// that come later.

const setHeight = (page, value) =>
  page.evaluate((value) => {
    document.querySelector("opengrid-grid").style.setProperty("--og-row-height", value);
  }, value);

/** Put the grid away and back: the move within the page that reads the height again. */
const moveGrid = (page) =>
  page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    const parent = host.parentNode;
    parent.removeChild(host);
    parent.appendChild(host);
  });

/** What is drawn: the sizer, and the box and the place of rows 0 to 2. */
const geometry = (page) =>
  page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const place = (row) => {
      const tr = root.querySelector(`td[data-row="${row}"]`)?.closest("tr");
      if (!tr) return null;
      return {
        top: Number(/translateY\((-?[\d.]+)px\)/.exec(tr.getAttribute("style"))?.[1]),
        // `offsetHeight` is the box CSS drew; the rows have a 1px rule below.
        height: tr.getBoundingClientRect().height,
      };
    };
    return {
      sizer: Number(/height:\s*([\d.e+]+)px/.exec(root.querySelector("tbody").getAttribute("style"))?.[1]),
      rows: [0, 1, 2].map(place),
    };
  });

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-row-height.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForSelector("opengrid-grid td[data-row]");
});

test("a grid put back in the page takes the row height its new place gives it", async ({ page }) => {
  expect((await geometry(page)).sizer).toBe(200 * 48);

  await setHeight(page, "64px");
  await moveGrid(page);

  await expect.poll(async () => (await geometry(page)).sizer).toBe(200 * 64);
  const { rows } = await geometry(page);
  expect(rows.map((row) => row.top)).toEqual([0, 64, 128]);
});

// A page that changes the property later (a theme switch, a class on a parent)
// gets rows placed at the new height, not rows that overlap.
test("a row height changed while the grid stays in the page moves the rows to it", async ({
  page,
}) => {
  await setHeight(page, "64px");

  await expect.poll(async () => (await geometry(page)).sizer).toBe(200 * 64);
  const { rows } = await geometry(page);
  expect(rows.map((row) => row.top)).toEqual([0, 64, 128]);
});

// The window math is in whole pixels; 40.5px is rounded up, not dropped for
// the default.
test("a fractional row height is the height the rows are placed at", async ({ page }) => {
  await setHeight(page, "40.5px");

  await expect.poll(async () => (await geometry(page)).sizer).not.toBe(200 * 48);
  const { sizer, rows } = await geometry(page);
  const step = rows[1].top - rows[0].top;
  // The page asked for 40.5px. The grid may round it, but not to the default:
  // the rows are placed where the page said they are tall.
  expect(Math.abs(step - 40.5)).toBeLessThanOrEqual(0.5);
  expect(sizer).toBe(200 * step);
});

test("a height that is not a pixel length keeps the default, and the grid stays consistent with it", async ({
  page,
}) => {
  // Documented: an unregistered custom property is not resolved, so `3rem`
  // reaches the grid as the text "3rem". The default holds for the window math.
  await setHeight(page, "3rem");
  await moveGrid(page);

  await expect.poll(async () => (await geometry(page)).sizer).toBe(200 * 42);
  const { rows } = await geometry(page);
  expect(rows.map((row) => row.top)).toEqual([0, 42, 84]);
});
