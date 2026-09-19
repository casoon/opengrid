import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Editing (plan point 37, decision: the component edits, the page saves).
//
// Every test here is keyboard-only. The grid has no write path — the engine's
// contract is a query — so what a commit produces is an **event**, and the test
// checks that the event carries what a page would need to store it.

async function focusCell(page, row, col) {
  await page.evaluate(
    ({ row, col }) => {
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`td[data-row="${row}"][data-col="${col}"]`)
        .focus();
    },
    { row, col },
  );
}

async function editor(page) {
  return page.evaluate(() => {
    const node = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="editor"]');
    if (!node) return null;
    return {
      tag: node.tagName.toLowerCase(),
      type: node.getAttribute("type"),
      step: node.getAttribute("step"),
      label: node.getAttribute("aria-label"),
      value: node.value,
      focused:
        document.querySelector("opengrid-grid").shadowRoot.activeElement === node,
      options: [...node.querySelectorAll("option")].map((option) => option.value),
    };
  });
}

async function cellText(page, row, col) {
  return page.evaluate(
    ({ row, col }) =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`td[data-row="${row}"][data-col="${col}"]`).textContent,
    { row, col },
  );
}

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-edit.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

test("Enter opens a typed editor and Escape leaves the value alone", async ({ page }) => {
  await open(page);
  const before = await cellText(page, 0, 2);

  // `amount` is a decimal: a number field with the column's own step, or the
  // browser would round away what the reader typed.
  await focusCell(page, 0, 2);
  await page.keyboard.press("Enter");

  await expect.poll(() => editor(page)).toMatchObject({
    tag: "input",
    type: "number",
    step: "0.01",
    label: "amount",
    focused: true,
  });

  await page.keyboard.press("Escape");
  await expect.poll(() => editor(page)).toBeNull();
  expect(await cellText(page, 0, 2)).toBe(before);
  expect(await page.evaluate(() => window.__changes.length)).toBe(0);

  // The focus is back on the cell, not lost to the document.
  const active = await page.evaluate(() => {
    const node = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    return { tag: node?.tagName, row: node?.getAttribute("data-row") };
  });
  expect(active).toEqual({ tag: "TD", row: "0" });
});

test("a committed value shows, is marked unsaved, and reaches the page", async ({ page }) => {
  await open(page);
  const before = await cellText(page, 0, 2);

  await focusCell(page, 0, 2);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("99.50");
  await page.keyboard.press("Enter");

  await expect.poll(() => cellText(page, 0, 2)).toContain("99.50");
  // Marked: the grid shows what was typed and does not claim it is stored.
  const marked = await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('td[data-row="0"][data-col="2"]')
      .hasAttribute("data-changed"),
  );
  expect(marked).toBe(true);

  const changes = await page.evaluate(() => window.__changes);
  expect(changes).toHaveLength(1);
  expect(changes[0]).toEqual({
    row: 0,
    column: "amount",
    value: "99.50",
    previous: before.replace(" *", ""),
  });
});

test("an empty editor clears a nullable cell and is refused on a required one", async ({
  page,
}) => {
  await open(page);

  // A typed input sanitizes most nonsense away before it ever reaches the
  // component — that is the point of typing it. What is left, and what matters,
  // is the empty field: on a nullable column it **means NULL**, because that is
  // the only way to clear a value.
  await focusCell(page, 0, 3);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");

  await expect.poll(() => cellText(page, 0, 3)).toBe("");
  const changes = await page.evaluate(() => window.__changes);
  expect(changes.at(-1)).toMatchObject({ column: "qty", value: "" });

  // `id` is the one required column: clearing it is refused and said out loud.
  const before = await cellText(page, 1, 0);
  await focusCell(page, 1, 0);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
            .textContent,
      ),
    )
    .toContain("needs a value");
  expect(await cellText(page, 1, 0)).toBe(before);
});

test("a boolean edits as a checkbox and a listed column as a select", async ({ page }) => {
  await open(page);

  // `customer` is Utf8, but the page supplied choices — so it is a select.
  await focusCell(page, 0, 1);
  await page.keyboard.press("Enter");
  const select = await editor(page);
  expect(select.tag).toBe("select");
  expect(select.options).toEqual(["Alpha", "Beta", "Gamma"]);
  await page.keyboard.press("Escape");

  // `id` is an integer: a number field with a step of one.
  await focusCell(page, 1, 0);
  await page.keyboard.press("Enter");
  expect(await editor(page)).toMatchObject({ tag: "input", type: "number", step: "1" });
  await page.keyboard.press("Escape");
});

test("the edited row survives and the marks go when the source speaks", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 2);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("77.00");
  await page.keyboard.press("Enter");
  await expect.poll(() => cellText(page, 0, 2)).toContain("77.00");

  // Sorting fetches again: the source's word replaces the reader's.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="0"]').focus(),
  );
  await page.keyboard.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document
            .querySelector("opengrid-grid")
            .shadowRoot.querySelectorAll("td[data-changed]").length,
      ),
    )
    .toBe(0);
});

test("has no axe violations with an editor open", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 1);
  await page.keyboard.press("Enter");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
