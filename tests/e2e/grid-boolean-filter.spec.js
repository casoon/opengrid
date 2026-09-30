// A boolean column in the filter row (issue #60).
//
// It was a checkbox, whose `value` is `on` whether it is ticked or not: the
// grid showed a chip "flag is on" with nothing filtered, and the first filter
// on any other column silently brought `flag = true` along. Now it is a choice
// of three — any, yes, no — and "any" is no filter at all.
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const CSV = readFileSync(fileURLToPath(new URL("./fixtures/grid-virtual.csv", import.meta.url)), "utf8")
  .trim()
  .split("\n")
  .slice(1)
  .map((line) => line.split(","));
const count = (keep) => CSV.filter(keep).length;

const shadow = (page, fn, arg) =>
  page.evaluate(
    ([source, arg]) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      return new Function("root", "arg", `return (${source})(root, arg);`)(root, arg);
    },
    [fn.toString(), arg],
  );
const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const chips = (page) => shadow(page, (root) => [...root.querySelectorAll('[part~="chip"]')].map((chip) => chip.textContent));

/** Chooses `value` in the flag column's choice and applies it with Enter. */
async function choose(page, value) {
  await shadow(page, (root, value) => {
    const choice = root.querySelector('select[data-value-col="6"]');
    choice.value = value;
    choice.focus();
  }, value);
  await page.keyboard.press("Enter");
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("columns", "id,customer,country,amount,qty,ordered_on,flag"),
  );
  await expect.poll(() => status(page)).toBe("200 Treffer");
});

test("a boolean column offers any, yes and no — and nothing is filtered at first", async ({ page }) => {
  const controls = await shadow(page, (root) => ({
    input: root.querySelector('input[data-col="6"]').hidden,
    choice: root.querySelector('select[data-value-col="6"]').hidden,
    options: [...root.querySelector('select[data-value-col="6"]').options].map((option) => [option.value, option.textContent]),
    name: root.querySelector('select[data-value-col="6"]').getAttribute("aria-label"),
    other: root.querySelector('select[data-value-col="1"]').hidden,
  }));
  expect(controls).toEqual({
    input: true,
    choice: false,
    options: [["", "any"], ["true", "yes"], ["false", "no"]],
    name: controls.name,
    other: true,
  });
  expect(controls.name).toContain("flag");
  expect(await chips(page)).toEqual([]);
});

test("yes and no filter, any takes the filter away", async ({ page }) => {
  await choose(page, "true");
  await expect.poll(() => status(page)).toBe(`${count((row) => row[6] === "true")} Treffer`);
  expect((await chips(page)).length).toBe(1);

  await choose(page, "false");
  await expect.poll(() => status(page)).toBe(`${count((row) => row[6] === "false")} Treffer`);

  await choose(page, "");
  await expect.poll(() => status(page)).toBe("200 Treffer");
  expect(await chips(page)).toEqual([]);
});

test("a filter on another column does not bring a boolean filter with it", async ({ page }) => {
  await shadow(page, (root) => {
    // The comparison is a menu behind the button in the field (issue #96).
    root.querySelector(`[part="filter-operator"][data-col="1"]`).click();
    root.querySelector(`[part="operator-menu"] [data-op="contains"]`).click();
    const input = root.querySelector('input[data-col="1"]');
    input.value = "Alpha";
    input.focus();
  });
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe(`${count((row) => row[1].includes("Alpha"))} Treffer`);
  expect((await chips(page)).length).toBe(1);
});

test("has no axe violations with the choice shown", async ({ page }) => {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
