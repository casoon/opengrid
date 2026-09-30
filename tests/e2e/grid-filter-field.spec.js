import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The filter row as one field per column (issue #96). It had two controls per
// column — a comparison `<select>` and a value field — and at ordinary widths
// both were cut off ("contai…", "30.0…"). Now the comparison is a small button
// inside the field that opens a menu; typing filters with the type's default.
//
// The fixture of `grid.spec.js`: five rows, `id,customer,amount,qty`.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

/** The operator button of `col`, as a reader meets it. */
const operator = (page, col) =>
  shadow(
    page,
    (root, col) => {
      const button = root.querySelector(`[part="filter-operator"][data-col="${col}"]`);
      return {
        tag: button.tagName.toLowerCase(),
        name: button.getAttribute("aria-label"),
        popup: button.getAttribute("aria-haspopup"),
        expanded: button.getAttribute("aria-expanded"),
        op: button.dataset.op,
        sign: button.textContent,
      };
    },
    col,
  );

const focused = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active
      ? { part: active.getAttribute("part"), role: active.getAttribute("role"), op: active.dataset.op ?? null }
      : null;
  });

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.waitForFunction(() => window.__opengridReady);
  await expect.poll(() => status(page)).toBe("5 matches");
});

test("each column has one field, with its comparison as a button inside it", async ({ page }) => {
  const fields = await shadow(page, (root) =>
    [...root.querySelectorAll("[data-filter-field]")].map((group) => {
      const button = group.querySelector('[part="filter-operator"]');
      const input = group.querySelector('input[part="filter-value"]');
      const b = button.getBoundingClientRect();
      const i = input.getBoundingClientRect();
      return {
        selects: group.querySelectorAll("select:not([hidden])").length,
        inside: b.left >= i.left && b.right <= i.right && b.top >= i.top - 1 && b.bottom <= i.bottom + 1,
        // The value field is the group's width, not half of it.
        share: Math.round((i.width / group.getBoundingClientRect().width) * 100),
      };
    }),
  );
  expect(fields).toHaveLength(4);
  for (const field of fields) {
    expect(field.selects).toBe(0);
    expect(field.inside).toBe(true);
    expect(field.share).toBeGreaterThanOrEqual(90);
  }
});

test("the button says the column and the comparison, and the type's default is chosen", async ({ page }) => {
  expect(await operator(page, 1)).toEqual({
    tag: "button",
    name: "customer operator: contains",
    popup: "menu",
    expanded: "false",
    op: "contains",
    sign: "∗",
  });
  // A number starts at "is", not at a text comparison it cannot take.
  expect(await operator(page, 0)).toMatchObject({ name: "id operator: is", op: "eq", sign: "=" });
  // The empty field says the comparison in words, next to the sign.
  expect(
    await shadow(page, (root) =>
      [0, 1].map((col) => root.querySelector(`input[data-col="${col}"]`).placeholder),
    ),
  ).toEqual(["is", "contains"]);
});

test("typing filters with the default comparison", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('input[data-col="1"]').focus());
  await page.keyboard.type("lph");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("2 matches");
});

test("the menu is operable from the keyboard and gives the focus back", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="filter-operator"][data-col="1"]').focus());
  await page.keyboard.press("ArrowDown");
  const menu = await shadow(page, (root) => {
    const menu = root.querySelector('[part="operator-menu"]');
    return {
      role: menu.getAttribute("role"),
      name: menu.getAttribute("aria-label"),
      items: [...menu.querySelectorAll('[role="menuitemradio"]')].map((item) => [
        item.dataset.op,
        item.getAttribute("aria-checked"),
      ]),
    };
  });
  expect(menu.role).toBe("menu");
  expect(menu.name).toBe("customer operator");
  expect(menu.items[0]).toEqual(["contains", "true"]);
  expect(menu.items.map(([op]) => op)).toEqual(
    expect.arrayContaining(["contains", "starts_with", "eq", "ne"]),
  );
  // The checked one has the focus, and the button says the menu is open.
  expect(await focused(page)).toEqual({ part: null, role: "menuitemradio", op: "contains" });
  expect((await operator(page, 1)).expanded).toBe("true");

  // Escape closes and returns to the button.
  await page.keyboard.press("Escape");
  expect(await focused(page)).toMatchObject({ part: "filter-operator" });
  expect((await operator(page, 1)).expanded).toBe("false");

  // Pick "starts with": the button says so, the focus goes to the value.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  expect(await operator(page, 1)).toMatchObject({
    name: "customer operator: starts with",
    op: "starts_with",
    expanded: "false",
  });
  expect(await focused(page)).toMatchObject({ part: "filter-value" });
  await page.keyboard.type("Al");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("2 matches");
});

test("a comparison picked while the field has a value filters at once", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('input[data-col="1"]').focus());
  await page.keyboard.type("Beta");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("2 matches");
  await shadow(page, (root) => {
    root.querySelector('[part="filter-operator"][data-col="1"]').click();
    root.querySelector('[part="operator-menu"] [data-op="ne"]').click();
  });
  await expect.poll(() => status(page)).toBe("3 matches");
});

test("has no axe violations with the menu open", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="filter-operator"][data-col="1"]').click());
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
