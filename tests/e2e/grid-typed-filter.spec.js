import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Typed filters (plan point 51).
//
// Until point 23 the grid had no types, so the filter row offered all eight
// comparisons for every column and sent every value as a string — against a
// number or a date that is a type error, which is why filtering only ever worked
// on text. The fixture's columns are `id` (int64, required), `customer` (utf8),
// `amount` (decimal 10,2) and `qty` (int64).

/** The operator options a column actually offers (hidden ones excluded). */
async function operators(page, column) {
  return page.evaluate((column) => {
    const select = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector(`select[data-col="${column}"]`);
    return [...select.options]
      .filter((option) => !option.hidden && !option.disabled)
      .map((option) => option.value);
  }, column);
}

/** The value input's type and step. */
async function valueInput(page, column) {
  return page.evaluate((column) => {
    const input = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector(`input[data-col="${column}"]`);
    return {
      type: input.getAttribute("type"),
      step: input.getAttribute("step"),
      disabled: input.disabled,
    };
  }, column);
}

async function status(page) {
  return page.evaluate(
    () =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('[part="status"]').textContent,
  );
}

/** Types a value into a column's filter and applies it. */
async function applyFilter(page, column, value) {
  await page.evaluate(
    ({ column, value }) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const input = root.querySelector(`input[data-col="${column}"]`);
      input.focus();
      input.value = value;
    },
    { column, value },
  );
  await page.keyboard.press("Enter");
}

/** Chooses an operator by its wire token. */
async function chooseOperator(page, column, token) {
  await page.evaluate(
    ({ column, token }) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const select = root.querySelector(`select[data-col="${column}"]`);
      select.value = token;
      select.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    },
    { column, token },
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("a column offers the operators its type allows", async ({ page }) => {
  // Text: substring tests belong here and nowhere else.
  const customer = await operators(page, 1);
  expect(customer).toContain("contains");
  expect(customer).toContain("starts_with");
  expect(customer).toContain("is_null");

  // A number has no substring, and `id` is required, so asking for its NULLs
  // would be a question with a constant answer.
  const id = await operators(page, 0);
  expect(id).not.toContain("contains");
  expect(id).not.toContain("starts_with");
  expect(id).not.toContain("is_null");
  expect(id).toEqual(["eq", "ne", "gt", "gte", "lt", "lte"]);
});

test("the value input fits the column", async ({ page }) => {
  expect(await valueInput(page, 1)).toMatchObject({ type: "text" });
  expect(await valueInput(page, 0)).toMatchObject({ type: "number", step: "1" });
  // A decimal input must accept its own scale, or the browser rounds the value
  // away before it is ever sent.
  expect(await valueInput(page, 2)).toMatchObject({ type: "number", step: "0.01" });
});

test("a numeric filter finds rows, not a type error", async ({ page }) => {
  await chooseOperator(page, 3, "gt");
  await applyFilter(page, 3, "3");

  // qty is 3, 2, 1, 4, 5 — two rows are greater than 3.
  await expect.poll(() => status(page)).toBe("2 matches");
  expect(
    await page.evaluate(() =>
      [
        ...document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelectorAll("tbody tr[aria-rowindex] td:nth-child(4)"),
      ].map((td) => td.textContent),
    ),
  ).toEqual(["4", "5"]);
});

test("a decimal filter keeps its precision", async ({ page }) => {
  await chooseOperator(page, 2, "lte");
  await applyFilter(page, 2, "20.00");

  // 5.00, 10.00 and 20.00 are at or below 20.00 — a comparison that only works
  // because the literal travels as a decimal rather than as text.
  await expect.poll(() => status(page)).toBe("3 matches");
});

test("an operator without a value disables its input", async ({ page }) => {
  await chooseOperator(page, 1, "is_null");
  await expect.poll(() => valueInput(page, 1).then((input) => input.disabled)).toBe(true);

  // Enter applies the filter from whichever control holds the focus — and the
  // value field is disabled now, so it has to be the operator control.
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('select[data-col="1"]')
      .focus(),
  );
  await page.keyboard.press("Enter");
  // Nothing in the fixture is null, so the filter holds and finds nothing.
  await expect.poll(() => status(page)).toBe("No matches");
});

test("a value the column cannot hold never becomes a query", async ({ page }) => {
  await chooseOperator(page, 3, "eq");
  // `2.5` is a number the input accepts but not an integer — and qty is int64.
  await applyFilter(page, 3, "2.5");

  await expect
    .poll(() => status(page))
    .toBe("qty: 2.5 is not a value for this column");
  // The grid is untouched: the query never ran.
  expect(
    await page.evaluate(
      () =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelectorAll("tbody tr[aria-rowindex]").length,
    ),
  ).toBe(5);

  // And a valid value clears it again.
  await applyFilter(page, 3, "2");
  await expect.poll(() => status(page)).toBe("1 match");
});

test("has no axe violations with typed filters", async ({ page }) => {
  await chooseOperator(page, 0, "gte");
  await applyFilter(page, 0, "2");
  await expect.poll(() => status(page)).toBe("4 matches");

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
