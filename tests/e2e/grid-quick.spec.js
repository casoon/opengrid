// "+ Filter" and "+ Group" (issue #34): the prototype's two quick doors, with
// the keyboard protocol decided before building — a non-modal dialog and a
// menu with the column menu's keys. Both write what the other doors write: the
// filter row's entry, and `group-by`.
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

const shadow = (page, fn, arg) =>
  page.evaluate(
    ([source, arg]) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      return new Function("root", "arg", `return (${source})(root, arg);`)(root, arg);
    },
    [fn.toString(), arg],
  );
const focused = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active?.getAttribute("data-dialog") ?? active?.getAttribute("data-dialog-action") ??
      active?.getAttribute("data-group-column") ?? active?.getAttribute("part") ?? null;
  });
const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 Treffer");
});

test("+ Filter opens a dialog, Tab stays inside, Enter in the value applies", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="add-filter"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => focused(page)).toBe("column");
  expect(
    await shadow(page, (root) => ({
      role: root.querySelector('[part="filter-dialog"]').getAttribute("role"),
      name: root.getElementById(root.querySelector('[part="filter-dialog"]').getAttribute("aria-labelledby")).textContent,
      expanded: root.querySelector('[part="add-filter"]').getAttribute("aria-expanded"),
    })),
  ).toEqual({ role: "dialog", name: "Add filter", expanded: "true" });

  // Tab from the last control comes back to the first, Shift+Tab the other way.
  await shadow(page, (root) => root.querySelector('[data-dialog-action="apply"]').focus());
  await page.keyboard.press("Tab");
  expect(await focused(page)).toBe("column");
  await page.keyboard.press("Shift+Tab");
  expect(await focused(page)).toBe("apply");

  await shadow(page, (root) => {
    const dialog = root.querySelector('[part="filter-dialog"]');
    const column = dialog.querySelector('[data-dialog="column"]');
    column.value = "country";
    column.dispatchEvent(new Event("change", { bubbles: true }));
    dialog.querySelector('[data-dialog="op"]').value = "eq";
    dialog.querySelector('[data-dialog="value"]').focus();
  });
  await page.keyboard.type("DE");
  await page.keyboard.press("Enter");

  // The filter row's own entry, the focus back on the button, the result said.
  await expect.poll(() => status(page)).not.toBe("200 Treffer");
  expect(
    await shadow(page, (root) => ({
      open: !!root.querySelector('[part="filter-dialog"]'),
      expanded: root.querySelector('[part="add-filter"]').getAttribute("aria-expanded"),
      entry: root.querySelector('input[data-col="2"]').value,
    })),
  ).toEqual({ open: false, expanded: "false", entry: "DE" });
  expect(await focused(page)).toBe("add-filter");
});

test("Escape closes the dialog without a change, and the focus goes back", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="add-filter"]').click());
  await expect.poll(() => focused(page)).toBe("column");
  await page.keyboard.press("Escape");
  expect(await focused(page)).toBe("add-filter");
  expect(await shadow(page, (root) => !!root.querySelector('[part="filter-dialog"]'))).toBe(false);
  expect(await status(page)).toBe("200 Treffer");
});

test("a value the column cannot take is named in the dialog, which stays open", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="add-filter"]').click());
  await shadow(page, (root) => {
    const dialog = root.querySelector('[part="filter-dialog"]');
    const column = dialog.querySelector('[data-dialog="column"]');
    column.value = "qty";
    column.dispatchEvent(new Event("change", { bubbles: true }));
    dialog.querySelector('[data-dialog="value"]').value = "1.5";
    dialog.querySelector('[data-dialog-action="apply"]').click();
  });
  const problem = await shadow(page, (root) => {
    const line = root.querySelector('[part="filter-dialog"] [data-dialog-problem]');
    return { text: line.textContent, hidden: line.hidden, role: line.getAttribute("role") };
  });
  expect(problem).toMatchObject({ hidden: false, role: "alert" });
  expect(problem.text).toContain("1.5");
  expect(await status(page)).toBe("200 Treffer");
});

test("+ Group is a menu with the column menu's keys, and a pick adds a level", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="add-grouping"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => focused(page)).toBe("id");
  const offered = await shadow(page, (root) =>
    [...root.querySelectorAll('[part="grouping-menu"] [role="menuitem"]')].map((item) => item.textContent),
  );
  // A decimal repeats too rarely to group by, so `amount` is not offered.
  expect(offered).toEqual(["id", "customer", "country", "qty", "ordered_on"]);
  await page.keyboard.press("End");
  expect(await focused(page)).toBe("ordered_on");
  await page.keyboard.press("ArrowDown");
  expect(await focused(page)).toBe("id");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => document.querySelector("opengrid-grid").getAttribute("group-by"))).toBe("country");
  expect(await focused(page)).toBe("add-grouping");

  // Escape closes without a pick.
  await page.keyboard.press("Enter");
  await expect.poll(() => focused(page)).toBe("id");
  await page.keyboard.press("Escape");
  expect(await focused(page)).toBe("add-grouping");

  // A second level, then the button says there is no third.
  await page.keyboard.press("Enter");
  await expect.poll(() => focused(page)).toBe("id");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => document.querySelector("opengrid-grid").getAttribute("group-by"))).toBe("country,customer");
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('[part="add-grouping"]').getAttribute("aria-disabled")))
    .toBe("true");
});

test("has no axe violations with the dialog open, nor with the menu open", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="add-filter"]').click());
  await expect.poll(() => focused(page)).toBe("column");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await page.keyboard.press("Escape");
  await shadow(page, (root) => root.querySelector('[part="add-grouping"]').click());
  await expect.poll(() => focused(page)).toBe("id");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
