import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The column menu (plan point 64).
//
// Every line of the keyboard protocol that point 64 wrote down before building
// has a test here, and so has the claim that makes the menu acceptable at all:
// each entry is a second way to something that already has a first.

// id, customer, country, amount, qty
const AMOUNT = 3;
const CUSTOMER = 1;

async function focusHeader(page, col) {
  await page.evaluate((col) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(`th[data-col="${col}"]`).focus();
  }, col);
}

/** What has the focus: a header, a menu entry, a field. */
async function focused(page) {
  return page.evaluate(() => {
    const element = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!element) return null;
    return {
      tag: element.tagName.toLowerCase(),
      col: element.getAttribute("data-col"),
      role: element.getAttribute("role"),
      action: element.getAttribute("data-action"),
    };
  });
}

async function menu(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const menu = root.querySelector('[part="column-menu"]');
    if (!menu) return null;
    return {
      open: menu.matches(":popover-open"),
      label: menu.getAttribute("aria-label"),
      actions: [...menu.querySelectorAll("[data-action]")].map((item) => item.dataset.action),
      checked: [...menu.querySelectorAll('[aria-checked="true"]')].map((item) => item.dataset.action),
    };
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-menu.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
  // The typed schema has arrived (the menu's entries depend on the types).
  await page.waitForFunction(
    () =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`th[data-col="3"]`)
        ?.getAttribute("data-align") === "end",
  );
});

test("Alt+Down opens the menu, focus on its first entry", async ({ page }) => {
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  expect(await menu(page)).toMatchObject({ open: true, label: "amount column menu" });
  expect(await focused(page)).toMatchObject({ role: "menuitemradio", action: "sort:asc" });
});

test("Shift+F10 opens it too", async ({ page }) => {
  await focusHeader(page, CUSTOMER);
  await page.keyboard.press("Shift+F10");
  expect((await menu(page))?.open).toBe(true);
});

test("the arrows move and wrap, Home and End jump", async ({ page }) => {
  await focusHeader(page, CUSTOMER);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("ArrowDown");
  expect((await focused(page)).action).toBe("sort:desc");
  await page.keyboard.press("End");
  expect((await focused(page)).action).toBe("hide");
  await page.keyboard.press("ArrowDown");
  expect((await focused(page)).action).toBe("sort:asc");
  await page.keyboard.press("ArrowUp");
  expect((await focused(page)).action).toBe("hide");
  await page.keyboard.press("Home");
  expect((await focused(page)).action).toBe("sort:asc");
});

test("Escape closes and gives the focus back to the header", async ({ page }) => {
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("Escape");
  expect(await menu(page)).toBeNull();
  expect(await focused(page)).toMatchObject({ tag: "th", col: String(AMOUNT) });
});

test("Tab closes and moves on from the header, no trap", async ({ page }) => {
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("Tab");
  expect(await menu(page)).toBeNull();
  // The focus left the menu; it is not held inside anything.
  const inMenu = await page.evaluate(
    () => !!document.querySelector("opengrid-grid").shadowRoot.activeElement?.closest('[part="column-menu"]'),
  );
  expect(inMenu).toBe(false);
});

test("the entries fit the column's type", async ({ page }) => {
  // A number offers aggregates and no grouping; text offers grouping and no
  // aggregate — "count" in every text header would be noise.
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  const amount = await menu(page);
  expect(amount.actions).toContain("aggregate:sum");
  expect(amount.actions).not.toContain("group:first");
  await page.keyboard.press("Escape");

  await focusHeader(page, CUSTOMER);
  await page.keyboard.press("Alt+ArrowDown");
  const customer = await menu(page);
  expect(customer.actions).toContain("group:first");
  expect(customer.actions.some((action) => action.startsWith("aggregate:"))).toBe(false);
});

test("sorting from the menu sorts, and the active direction is checked", async ({ page }) => {
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");

  await expect
    .poll(() =>
      page.evaluate(() =>
        document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="3"]').getAttribute("aria-sort"),
      ),
    )
    .toBe("descending");
  expect(await focused(page)).toMatchObject({ tag: "th", col: String(AMOUNT) });

  await page.keyboard.press("Alt+ArrowDown");
  expect((await menu(page)).checked).toEqual(["sort:desc", "aggregate:none"]);
});

test("Filter … goes to the column's field in the filter row", async ({ page }) => {
  // The menu holds no form: a `<select>` inside `role="menu"` breaks the role.
  await focusHeader(page, CUSTOMER);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  expect((await focused(page)).action).toBe("filter");
  await page.keyboard.press("Enter");
  expect(await focused(page)).toMatchObject({ tag: "input", col: String(CUSTOMER) });
});

test("grouping from the menu groups", async ({ page }) => {
  await focusHeader(page, 2);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowUp");
  expect((await focused(page)).action).toBe("group:first");
  await page.keyboard.press("Enter");
  await expect
    .poll(() => page.evaluate(() => document.querySelector("opengrid-grid").getAttribute("group-by")))
    .toBe("country");
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.querySelector("opengrid-grid").shadowRoot.querySelector("table").getAttribute("role"),
      ),
    )
    .toBe("treegrid");
});

test("an aggregate chosen in the menu shows in the groups and travels in the view", async ({
  page,
}) => {
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("group-by", "country"));
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="group"]'),
  );

  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[data-action="aggregate:sum"]')
      .focus(),
  );
  await page.keyboard.press("Enter");

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document
            .querySelector("opengrid-grid")
            .shadowRoot.querySelector('tr[data-kind="group"] td[data-col="3"]')
            ?.getAttribute("aria-label") ?? "",
      ),
    )
    .toMatch(/^Sum: /);
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  expect(view.aggregates).toEqual({ amount: "sum" });
});

test("hiding from the menu hides", async ({ page }) => {
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("End");
  expect((await focused(page)).action).toBe("hide");
  await page.keyboard.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("thead th[data-col]")].map(
          (th) => th.querySelector("span").textContent,
        ),
      ),
    )
    .not.toContain("amount");
});

test("every entry has a way in without the menu", async ({ page }) => {
  // The 2.5.7-shaped argument: the menu is a second door. Each action it offers
  // maps onto a path that exists with the menu switched off.
  const without = {
    "sort:asc": "Enter on the header",
    "sort:desc": "Enter on the header",
    filter: "the filter row",
    hide: "the column list",
    "group:first": "group-by / set_view",
    "group:second": "group-by / set_view",
    "group:remove": "group-by / set_view",
  };
  for (const col of [AMOUNT, CUSTOMER]) {
    await focusHeader(page, col);
    await page.keyboard.press("Alt+ArrowDown");
    for (const action of (await menu(page)).actions) {
      const known = without[action] ?? (action.startsWith("aggregate:") ? "set_columns / set_view" : null);
      expect(known, `${action} has no way in without the menu`).not.toBeNull();
    }
    await page.keyboard.press("Escape");
  }
});

test("the menu sits below its header and inside the window", async ({ page }) => {
  // WCAG 2.2 §2.4.11: it must not cover the header the focus goes back to.
  for (const col of [0, 4]) {
    await focusHeader(page, col);
    await page.keyboard.press("Alt+ArrowDown");
    const box = await page.evaluate((col) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const menu = root.querySelector('[part="column-menu"]').getBoundingClientRect();
      const header = root.querySelector(`th[data-col="${col}"]`).getBoundingClientRect();
      return {
        below: menu.top >= header.bottom - 0.5,
        overlaps:
          menu.left < header.right && menu.right > header.left && menu.top < header.bottom && menu.bottom > header.top,
        inside: menu.left >= 0 && menu.right <= window.innerWidth,
      };
    }, col);
    expect(box).toEqual({ below: true, overlaps: false, inside: true });
    await page.keyboard.press("Escape");
  }
});

test("the pointer path: a 24px trigger in the header", async ({ page }) => {
  const trigger = await page.evaluate(() => {
    const button = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('th[data-col="3"] [part="column-menu-button"]');
    const box = button.getBoundingClientRect();
    return {
      width: Math.round(box.width),
      height: Math.round(box.height),
      hidden: button.getAttribute("aria-hidden"),
      tabbable: button.hasAttribute("tabindex"),
      shortcut: button.closest("th").getAttribute("aria-keyshortcuts"),
    };
  });
  // Not a second tab stop inside the grid; the cell says how to get there.
  expect(trigger).toEqual({
    width: 24,
    height: 24,
    hidden: "true",
    tabbable: false,
    shortcut: "Alt+ArrowDown",
  });

  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('th[data-col="3"] [part="column-menu-button"]')
      .click(),
  );
  expect((await menu(page))?.open).toBe(true);
});

test("without the attribute there is no menu", async ({ page }) => {
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("column-menu"));
  await page.waitForFunction(
    () => !document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="column-menu-button"]'),
  );
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  expect(await menu(page)).toBeNull();
});

test("has no axe violations with the menu open", async ({ page }) => {
  await focusHeader(page, AMOUNT);
  await page.keyboard.press("Alt+ArrowDown");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
