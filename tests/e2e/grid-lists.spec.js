import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// List columns (issue #185): chips in the cell, the ARIA combobox over the page's
// choices, a text that is no choice refused, the filter operators.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const chips = (page, row, col) =>
  shadow(page, (root, { row, col }) =>
    [...root.querySelectorAll(`td[data-row="${row}"][data-col="${col}"] [part~="list-item"]`)].map((chip) => chip.textContent),
  { row, col });
const column = (page, col) =>
  shadow(page, (root, col) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => tr.querySelector(`td[data-col="${col}"]`)?.textContent),
    col);

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-lists.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(async () => (await column(page, 1)).filter(Boolean).length).toBe(5);
}

async function startEdit(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
  await page.keyboard.press("Enter");
}

const comboboxState = (page) =>
  shadow(page, (root) => {
    const input = root.querySelector('[part="editor"] input');
    const options = [...root.querySelectorAll('[part~="listbox"] [role="option"]:not([hidden])')].map((o) => o.textContent);
    return {
      expanded: input?.getAttribute("aria-expanded"),
      options,
      active: root.getElementById?.("x") ?? input?.getAttribute("aria-activedescendant"),
      focused: root.activeElement === input,
      value: input?.value,
    };
  });

test("a list shows as chips, with the label of a choice, and NULL and the empty list as nothing", async ({ page }) => {
  await open(page);
  expect(await chips(page, 0, 2)).toEqual(["Maths", "Physics"]);
  expect(await chips(page, 1, 3)).toEqual([]);
  expect(await chips(page, 3, 2)).toEqual([]);
  expect(await chips(page, 0, 3)).toEqual(["101", "102"]);
  // The cell's accessible content reads as a list.
  expect(await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="2"] [part~="list"]').getAttribute("role"))).toBe("list");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("the editor is a combobox: type to filter, arrows and Enter pick, Backspace takes a chip back, Enter commits", async ({ page }) => {
  await open(page);
  await startEdit(page, 1, 2);
  // The cell's own pick is a chip already; the focus is in the text field.
  expect(await comboboxState(page)).toMatchObject({ expanded: "false", focused: true, value: "" });
  expect(await shadow(page, (root) => [...root.querySelectorAll('[part="editor"] [part~="list-item"]')].map((c) => c.textContent.replace("×", "")))).toEqual(["Maths"]);

  await page.keyboard.type("ph");
  await expect.poll(() => comboboxState(page)).toMatchObject({ expanded: "true", options: ["Physics"] });
  expect(await status(page)).toContain("1 choice");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  await page.keyboard.press("ArrowDown");
  expect((await comboboxState(page)).active).toBeTruthy();
  await page.keyboard.press("Enter");
  expect(await status(page)).toContain("Physics added");
  expect(await comboboxState(page)).toMatchObject({ expanded: "false", value: "" });

  // Backspace on the empty field takes the last chip back.
  await page.keyboard.press("Backspace");
  expect(await status(page)).toContain("Physics removed");

  await page.keyboard.type("che");
  await page.keyboard.press("Enter");
  // Enter with an empty field commits.
  await page.keyboard.press("Enter");
  const changes = await page.evaluate(() => window.__changes);
  expect(changes).toHaveLength(1);
  expect(changes[0]).toMatchObject({ key: 2, column: "subjects", value: '["m","c"]', previous: '["m"]' });
  expect(await chips(page, 1, 2)).toEqual(["Maths", "Chemistry"]);
});

test("a text that is none of the choices is refused with the reason, not dropped", async ({ page }) => {
  await open(page);
  await startEdit(page, 1, 2);
  await page.keyboard.type("zzz");
  await page.keyboard.press("Enter");
  expect(await shadow(page, (root) => root.querySelector('[part="validation"]')?.textContent)).toBe("“zzz” is not one of the choices");
  expect(await status(page)).toContain("“zzz” is not one of the choices");
  expect(await page.evaluate(() => window.__changes)).toHaveLength(0);
  // The editor is still there, with the focus in its field.
  expect((await comboboxState(page)).focused).toBe(true);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  expect(await shadow(page, (root) => !!root.querySelector('[part="editor"]'))).toBe(false);
});

test("a list of integers picks numbers, and a NULL cell stays NULL when nothing is picked", async ({ page }) => {
  await open(page);
  await startEdit(page, 2, 3);
  await page.keyboard.type("103");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__changes.at(-1))).toMatchObject({ key: 3, column: "rooms", value: "[101,103]" });

  await startEdit(page, 3, 3);
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__changes)).toHaveLength(1);
  expect(await chips(page, 3, 3)).toEqual([]);
});

test("a column with choices that is no list picks one value, committed with the pick", async ({ page }) => {
  await open(page);
  await startEdit(page, 0, 1);
  await page.keyboard.type("gra");
  await page.keyboard.press("Enter");
  const changes = await page.evaluate(() => window.__changes);
  expect(changes).toEqual([expect.objectContaining({ key: 1, column: "name", value: "Grace", previous: "Ada" })]);
  expect(await shadow(page, (root) => !!root.querySelector('[part="editor"]'))).toBe(false);
});

test("contains, contains all and is empty filter a list column", async ({ page }) => {
  await open(page);
  const setFilter = async (op, text) => {
    await page.evaluate(
      ({ op, text }) => {
        const root = document.querySelector("opengrid-grid").shadowRoot;
        const operator = root.querySelector('[data-filter-field]:nth-of-type(3) [part="filter-operator"]');
        operator.setAttribute("data-op", op);
        const value = root.querySelector('[data-filter-field]:nth-of-type(3) input');
        value.value = text;
        value.dispatchEvent(new Event("change", { bubbles: true }));
        value.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      },
      { op, text },
    );
  };
  await setFilter("has", "m");
  await expect.poll(async () => (await column(page, 1)).filter(Boolean)).toEqual(["Ada", "Grace", "Linus"]);
  await setFilter("has_all", "m, p");
  await expect.poll(async () => (await column(page, 1)).filter(Boolean)).toEqual(["Ada", "Linus"]);
  await setFilter("is_empty", "");
  await expect.poll(async () => (await column(page, 1)).filter(Boolean)).toEqual(["Edsger"]);
});
