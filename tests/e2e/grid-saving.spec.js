import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// A page that saves every cell on its own (issue #153): the edit names its
// record, the page says what became of it, and fills a column of its own.
// Keyboard-only, like the editing spec; the page's calls are made by the test.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const cell = (page, row, col) =>
  shadow(
    page,
    (root, { row, col }) => {
      const td = root.querySelector(`td[data-row="${row}"][data-col="${col}"]`);
      return {
        text: td.textContent,
        changed: td.hasAttribute("data-changed"),
        state: td.getAttribute("data-state"),
        part: td.getAttribute("part"),
        invalid: td.getAttribute("aria-invalid"),
        readonly: td.getAttribute("aria-readonly"),
        focused: root.activeElement === td,
      };
    },
    { row, col },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-save.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

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

const setState = (page, ...args) =>
  page.evaluate((args) => window.__module.set_cell_state(document.querySelector("opengrid-grid"), ...args), args);

test("an edit names its record, and saved takes the mark off without moving the focus", async ({
  page,
}) => {
  await open(page);
  await edit(page, 0, 1, "Delta");

  const changes = await page.evaluate(() => window.__changes);
  expect(changes).toHaveLength(1);
  expect(changes[0]).toMatchObject({ row: 0, key: 1, column: "customer", value: "Delta" });

  await setState(page, 1, "customer", "saving");
  await expect.poll(() => cell(page, 0, 1)).toMatchObject({
    text: "Delta",
    changed: true,
    state: "saving",
    part: "cell cell-saving",
    focused: true,
  });

  await setState(page, 1, "customer", "saved");
  await expect.poll(() => cell(page, 0, 1)).toMatchObject({
    text: "Delta",
    changed: false,
    state: "saved",
    part: "cell cell-saved",
    invalid: null,
    focused: true,
  });
});

test("an error keeps the value, marks the cell and is said in the live region", async ({ page }) => {
  await open(page);
  await edit(page, 1, 1, "Omega");
  await setState(page, 2, "customer", "error", "Customer of order 2 was not saved.");

  await expect.poll(() => cell(page, 1, 1)).toMatchObject({
    text: "Omega",
    changed: true,
    state: "error",
    part: "cell cell-error",
    invalid: "true",
    focused: true,
  });
  await expect.poll(() => status(page)).toContain("Customer of order 2 was not saved.");
  // An untouched cell is a plain `cell`.
  expect(await cell(page, 0, 1)).toMatchObject({ part: "cell", state: null });
});

test("a page styles each state through its own part", async ({ page }) => {
  await open(page);
  await page.addStyleTag({
    content: "opengrid-grid::part(cell-error) { background-color: rgb(253, 232, 232); }",
  });
  await edit(page, 1, 1, "Omega");
  const background = () =>
    shadow(
      page,
      (root, { row, col }) =>
        getComputedStyle(root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)).backgroundColor,
      { row: 1, col: 1 },
    );
  expect(await background()).not.toBe("rgb(253, 232, 232)");
  await setState(page, 2, "customer", "error", "Not saved.");
  await expect.poll(background).toBe("rgb(253, 232, 232)");
  await setState(page, 2, "customer", "saved");
  await expect.poll(background).not.toBe("rgb(253, 232, 232)");
});

test("the page fills a read-only column without an event, and the value stays with its record", async ({
  page,
}) => {
  await open(page);
  expect(await cell(page, 0, 3)).toMatchObject({ readonly: "true" });

  await page.evaluate(() =>
    window.__module.set_values(document.querySelector("opengrid-grid"), [
      { key: 1, column: "qty", value: 42 },
    ]),
  );
  await expect.poll(() => cell(page, 0, 3)).toMatchObject({ text: "42", changed: false });
  expect(await page.evaluate(() => window.__changes.length)).toBe(0);

  // Enter on a read-only cell opens no editor — nor F2, nor a typed digit.
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="3"]').focus());
  for (const key of ["Enter", "F2", "4"]) {
    await page.keyboard.press(key);
    expect(await shadow(page, (root) => root.querySelector('[part="editor"]')), key).toBeNull();
  }
  expect(await cell(page, 0, 3)).toMatchObject({ text: "42", focused: true });

  // Sorted the other way round, order 1 sits elsewhere — and keeps the value.
  await page.evaluate(() =>
    window.__module.set_view(document.querySelector("opengrid-grid"), {
      sort: [{ field: "id", direction: "desc" }],
    }),
  );
  await expect
    .poll(() =>
      shadow(page, (root) => {
        const row = [...root.querySelectorAll('td[data-col="0"]')].find((td) => td.textContent === "1");
        return row && root.querySelector(`td[data-row="${row.dataset.row}"][data-col="3"]`).textContent;
      }),
    )
    .toBe("42");
});

test("has no axe violations with every state shown", async ({ page }) => {
  await open(page);
  await edit(page, 0, 1, "Delta");
  await edit(page, 1, 1, "Omega");
  await setState(page, 1, "customer", "saving");
  await setState(page, 2, "customer", "error", "Not saved.");
  await setState(page, 3, "customer", "saved");
  await page.evaluate(() =>
    window.__module.set_values(document.querySelector("opengrid-grid"), [
      { key: 1, column: "qty", value: 7 },
    ]),
  );
  await expect.poll(() => cell(page, 1, 1)).toMatchObject({ state: "error" });
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
