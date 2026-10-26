import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Rows the page already holds (issue #182): loaded as objects, replaced under
// the same name, and a grid on the source keeps its view when that happens.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const column = (page, col) =>
  shadow(page, (root, col) =>
    // The pool keeps spare rows with no content; a record always has its id.
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.querySelector('td[data-col="0"]')?.textContent.trim())
      .map((tr) => tr.querySelector(`td[data-col="${col}"]`).textContent), col);

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-rows.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid")).toBeVisible();
}

const replace = (page, rows) =>
  page.evaluate((rows) => window.__provider.loadRows("people", rows, window.__schema), rows);

test("the rows are the source, with dates and nulls as they were written", async ({ page }) => {
  await open(page);
  await expect.poll(() => column(page, 1)).toEqual(["Ada", "Grace", "Edsger"]);
  expect(await column(page, 2)).toEqual(["1815-12-10", "1906-12-09", ""]);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("loading under the same name replaces the source and the grid keeps its view", async ({
  page,
}) => {
  await open(page);
  await expect.poll(() => column(page, 1)).toEqual(["Ada", "Grace", "Edsger"]);
  // The focus is on the record Grace.
  await shadow(page, (root) => root.querySelector('td[data-row="1"][data-col="1"]').focus());

  await replace(page, [
    { id: 0, name: "Zero", born: null },
    { id: 1, name: "Ada", born: null },
    { id: 2, name: "Grace Hopper", born: null },
    { id: 3, name: "Edsger", born: null },
  ]);

  await expect.poll(() => column(page, 1)).toEqual(["Zero", "Ada", "Grace Hopper", "Edsger"]);
  // Grace is row 2 now; the focus followed the record, not the position.
  await expect
    .poll(() =>
      shadow(page, (root) => {
        const active = root.activeElement;
        return active && active.closest("tr").textContent.includes("Grace Hopper");
      }),
    )
    .toBe(true);
});

test("a row that does not fit is refused with its place named, and the old rows stay", async ({
  page,
}) => {
  await open(page);
  await expect.poll(() => column(page, 1)).toEqual(["Ada", "Grace", "Edsger"]);
  const message = await page.evaluate(() =>
    window.__provider
      .loadRows("people", [{ id: 1, name: "A" }, { id: "x", name: "B" }], window.__schema)
      .then(() => null, (error) => error.message),
  );
  expect(message).toContain("row 1");
  expect(message).toContain("id");
  expect(await column(page, 1)).toEqual(["Ada", "Grace", "Edsger"]);
});
