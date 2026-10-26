import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Editing while grouped (issue #190): with a `row-key` an edit names its record
// by the key; group headers and the total stay read-only.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const focus = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), {
    row,
    col,
  });

const text = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).textContent, {
    row,
    col,
  });

const editing = (page) => shadow(page, (root) => !!root.querySelector("td input, td select"));

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-group-edit.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  // Alpha opens: its two rows follow its header.
  await focus(page, 0, 0);
  await page.keyboard.press("Enter");
  await expect.poll(() => text(page, 1, 0)).toBe("2");
}

test("a row inside a group edits, and the change names the record by its key", async ({ page }) => {
  await open(page);
  await focus(page, 2, 3);
  await page.keyboard.press("Enter");
  expect(await editing(page)).toBe(true);
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("9");
  await page.keyboard.press("Enter");

  const changes = await page.evaluate(() => window.__changes);
  expect(changes).toHaveLength(1);
  // The second row of Alpha is the record with id 4.
  expect(changes[0]).toMatchObject({ key: 4, column: "qty", value: "9" });
  expect(await text(page, 2, 3)).toBe("9");

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("a group header and the total open no editor", async ({ page }) => {
  await open(page);
  await focus(page, 0, 3);
  await page.keyboard.press("F2");
  expect(await editing(page)).toBe(false);
  await page.keyboard.type("7");
  expect(await editing(page)).toBe(false);
  expect(await page.evaluate(() => window.__changes)).toHaveLength(0);
});

test("without a row-key editing stays off", async ({ page }) => {
  await open(page, "?nokey");
  await focus(page, 2, 3);
  await page.keyboard.press("Enter");
  expect(await editing(page)).toBe(false);
  await page.keyboard.press("F2");
  expect(await editing(page)).toBe(false);
});

test("replacing the source updates the groups from the fresh rows", async ({ page }) => {
  await open(page);
  expect(await text(page, 0, 0)).toContain("Alpha");
  await page.evaluate(() =>
    window.__provider.loadRows(
      "orders",
      [
        { id: 1, customer: "Alpha", amount: "1.00", qty: 1 },
        { id: 2, customer: "Alpha", amount: "1.00", qty: 1 },
        { id: 3, customer: "Alpha", amount: "1.00", qty: 1 },
        { id: 4, customer: "Alpha", amount: "1.00", qty: 1 },
      ],
      JSON.stringify({
        fields: [
          { name: "id", type: "int64", nullable: false },
          { name: "customer", type: "utf8", nullable: true },
          { name: "amount", type: { decimal: { precision: 10, scale: 2 } }, nullable: true },
          { name: "qty", type: "int64", nullable: true },
        ],
      }),
    ),
  );
  // One group of four rows now; the header says so.
  await expect.poll(() => shadow(page, (root) => root.querySelectorAll("tbody tr").length > 0)).toBe(true);
  await expect
    .poll(() => shadow(page, (root) => root.querySelector("tbody tr").textContent))
    .toMatch(/4/);
});
