import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// An action column (issue #189): a button per row and action, reachable by the
// arrow keys, pressed with Enter or Space, naming the record by its key.

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

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-actions.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

test("the actions are columns named by their labels, with a button on every row", async ({ page }) => {
  await open(page);
  const headers = await shadow(page, (root) =>
    [...root.querySelectorAll("thead th")].map((th) => th.textContent.trim()));
  expect(headers.slice(-2)).toEqual(["Open", "Remove"]);
  const buttons = await shadow(page, (root) =>
    [...root.querySelectorAll('tr[aria-rowindex="2"] button[part="action"]')].map((b) => b.textContent));
  expect(buttons).toEqual(["Open", "Remove"]);
  expect(await shadow(page, (root) => root.querySelector("table").getAttribute("aria-colcount"))).toBe("6");
});

test("the arrow keys reach the action cells and Enter and Space press them", async ({ page }) => {
  await open(page);
  await focus(page, 1, 3);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press(" ");
  // The row stays; the arrow at the last column stays too.
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");

  const actions = await page.evaluate(() => window.__actions);
  expect(actions).toHaveLength(3);
  // The grid sorts by its first column: row 1 is the record with id 2.
  expect(actions[0]).toMatchObject({ id: "open", key: 2, row: 1 });
  expect(actions[1]).toMatchObject({ id: "remove", key: 2 });
  expect(actions[2]).toMatchObject({ id: "remove", key: 2 });
});

test("a click on the button fires it, and the header of an action does not sort", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('tr[aria-rowindex="4"] button[data-action="open"]').click());
  const actions = await page.evaluate(() => window.__actions);
  expect(actions).toEqual([expect.objectContaining({ id: "open", key: 3 })]);

  await shadow(page, (root) => root.querySelector('thead th[part="action-header"]').focus());
  await page.keyboard.press("Enter");
  expect(await shadow(page, (root) => root.querySelector("thead th[aria-sort=ascending]")?.dataset.col)).toBe("0");
});

test("has no axe violations", async ({ page }) => {
  await open(page);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
