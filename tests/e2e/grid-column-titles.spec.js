// Column titles (issue #66): a column is named by the page wherever the grid
// names it to a reader, and by its field name wherever the name is an
// identifier — the query, the view, the search expression.
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
const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 Treffer");
  // The fixture's own configuration, with titles for two columns.
  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.querySelector("opengrid-grid"), {
      customer: { facet: "list", title: "Kunde" },
      country: { facet: "pills", title: "Land" },
      amount: { facet: "range", aggregate: "sum" },
      qty: { aggregate: "sum" },
      ordered_on: { facet: "period" },
    }),
  );
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('th[data-col="1"] span').textContent))
    .toBe("Kunde");
});

test("the header, the column list and the filter row name the column by its title", async ({ page }) => {
  const named = await shadow(page, (root) => ({
    headers: [...root.querySelectorAll("th[data-col] > span:first-child")].map((span) => span.textContent),
    list: [...root.querySelectorAll('[part="column-toggle"]')].map((label) => label.textContent.trim()),
    operator: root.querySelector('select[data-col="1"]').getAttribute("aria-label"),
    value: root.querySelector('input[data-col="1"]').getAttribute("aria-label"),
    menu: root.querySelector('th[data-col="2"]').textContent,
  }));
  expect(named.headers).toEqual(["id", "Kunde", "Land", "amount", "qty", "ordered_on"]);
  expect(named.list).toEqual(expect.arrayContaining(["Kunde", "Land", "amount"]));
  expect(named.operator).toContain("Kunde");
  expect(named.value).toContain("Kunde");
});

test("group rows, chips and facets say the title; the view keeps the field name", async ({ page }) => {
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      group: ["country"],
      filters: [{ column: "customer", op: "contains", value: "Al" }],
    }),
  );
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="group"]'),
  );
  const seen = await shadow(page, (root) => ({
    group: root.querySelector('tr[data-kind="group"]').textContent,
    chips: [...root.querySelectorAll('[part~="chip"]')].map((chip) => chip.textContent),
    legends: [...root.querySelectorAll('[part="facet"] legend')].map((legend) => legend.textContent),
  }));
  expect(seen.group).toMatch(/^\s*Land: /);
  expect(seen.chips.some((chip) => chip.includes("Land"))).toBe(true);
  expect(seen.chips.some((chip) => chip.startsWith("Kunde "))).toBe(true);
  expect(seen.legends).toEqual(expect.arrayContaining(["Kunde", "Land"]));

  const view = await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));
  expect(view.group).toEqual(["country"]);
  expect(view.filters[0].column).toBe("customer");
});

test("the + Filter dialog and the + Group menu offer titles, and take field names", async ({ page }) => {
  await shadow(page, (root) => root.querySelector('[part="add-filter"]').click());
  await expect.poll(() => shadow(page, (root) => !!root.querySelector('[part="filter-dialog"]'))).toBe(true);
  const options = await shadow(page, (root) =>
    [...root.querySelectorAll('[part="filter-dialog"] [data-dialog="column"] option')].map((option) => [option.value, option.textContent]),
  );
  expect(options).toEqual(expect.arrayContaining([["customer", "Kunde"], ["country", "Land"]]));
  await page.keyboard.press("Escape");

  await shadow(page, (root) => root.querySelector('[part="add-grouping"]').click());
  await expect.poll(() => shadow(page, (root) => !!root.querySelector('[part="grouping-menu"]'))).toBe(true);
  const items = await shadow(page, (root) =>
    [...root.querySelectorAll('[part="grouping-menu"] [role="menuitem"]')].map((item) => [item.dataset.groupColumn, item.textContent]),
  );
  expect(items).toEqual(expect.arrayContaining([["country", "Land"]]));
});

test("a suggestion shows the title and completes the field name", async ({ page }) => {
  const input = page.locator("opengrid-grid").locator('[part="search-input"]');
  await input.focus();
  await page.keyboard.type("cou");
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-list"] [data-column="country"]'),
  );
  const option = await shadow(page, (root) => root.querySelector('[part="search-list"] [data-column="country"]').textContent);
  expect(option).toContain("Land");
  expect(option).toContain("country");
});

test("an empty title is reported, and nothing of the call is applied", async ({ page }) => {
  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.querySelector("opengrid-grid"), { country: { title: "  " } }),
  );
  await expect.poll(() => status(page)).toContain("country");
  expect(await shadow(page, (root) => root.querySelector('th[data-col="2"] span').textContent)).toBe("Land");
});

test("has no axe violations with titles", async ({ page }) => {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
