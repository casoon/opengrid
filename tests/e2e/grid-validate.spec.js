import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Validation while typing, without blocking (issue #188): the sentence under
// the editor on every keystroke, on the cell afterwards, said once; an error
// blocks only where the column asks.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const under = (page) => shadow(page, (root) => root.querySelector('[part="validation"]')?.textContent ?? null);
const cellMarks = (page, row, col) =>
  shadow(
    page,
    (root, { row, col }) => {
      const td = root.querySelector(`td[data-row="${row}"][data-col="${col}"]`);
      return {
        validation: td.getAttribute("data-validation"),
        description: td.getAttribute("aria-description"),
        invalid: td.getAttribute("aria-invalid"),
        text: td.textContent,
      };
    },
    { row, col },
  );

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-validate.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function startEdit(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
}

test("a warning shows under the editor as the reader types, is said once, and does not block", async ({ page }) => {
  await open(page);
  await startEdit(page, 0, 2);
  await page.keyboard.type("5");
  expect(await under(page)).toBeNull();
  await page.keyboard.type("0");
  expect(await under(page)).toBeNull();
  await page.keyboard.type("0");
  // 500 seats: the validator talks on the keystroke.
  await expect.poll(() => under(page)).toBe("More than 60 seats is a hall.");
  expect(await status(page)).toContain("Warning: More than 60 seats is a hall.");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  // Another keystroke, the same words: no second announcement (the line is the same text).
  const said = await status(page);
  await page.keyboard.type("1");
  expect(await status(page)).toBe(said);

  // The warning does not block: the commit goes through.
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__changes)).toHaveLength(1);
  expect(await under(page)).toBeNull();
  expect(await cellMarks(page, 0, 2)).toMatchObject({
    validation: "warning",
    description: "More than 60 seats is a hall.",
    invalid: null,
    text: expect.stringContaining("5001"),
  });
  // The validator saw the text and the record by field name.
  const calls = await page.evaluate(() => window.__calls);
  expect(calls.at(-1)).toMatchObject({ value: "5001", row: { id: 1, name: "A1" } });
});

test("an error blocks only in the column that asks, and the editor stays with its sentence", async ({ page }) => {
  await open(page);
  await startEdit(page, 1, 1);
  await page.keyboard.type("C3");
  await expect.poll(() => under(page)).toBe("C3 exists already.");
  await page.keyboard.press("Enter");
  // Blocked: the editor is still there, nothing was committed.
  expect(await shadow(page, (root) => !!root.querySelector("td [part=editor]"))).toBe(true);
  expect(await page.evaluate(() => window.__changes)).toHaveLength(0);
  expect(await status(page)).toContain("Error: C3 exists already.");

  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("E5");
  await expect.poll(() => under(page)).toBeNull();
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__changes)).toHaveLength(1);
  expect(await cellMarks(page, 1, 1)).toMatchObject({ validation: null });
});

test("a type the column cannot hold always blocks, and the page can report a finding of its own", async ({ page }) => {
  await open(page);
  // `id` is required: nothing to commit, whatever a validator thinks.
  await startEdit(page, 2, 0);
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__changes)).toHaveLength(0);
  expect(await status(page)).toContain("id needs a value");

  await page.evaluate(() =>
    window.__module.set_cell_state(document.querySelector("opengrid-grid"), 4, "seats", "invalid", "Seats must match the hall."),
  );
  expect(await cellMarks(page, 3, 2)).toMatchObject({
    validation: "error",
    description: "Seats must match the hall.",
    invalid: "true",
  });
  expect(await status(page)).toContain("Seats must match the hall.");
});
