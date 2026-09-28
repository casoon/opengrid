// A narrow column keeps its name (issue #61).
//
// With the column menu, a sorted 80 px `id` column showed only "▲ ⋯": the
// header reserved a fixed 2.75em for the sort marks on top of the menu
// button, which left the name a negative width. Now what stands beside the
// name is reserved as it is, and a column is never drawn narrower than its
// header needs.
import { test, expect } from "@playwright/test";

const header = (page) =>
  page.evaluate(() => {
    const th = document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="0"]');
    const name = th.querySelector("span:first-child");
    return {
      column: th.getBoundingClientRect().width,
      name: name.getBoundingClientRect().width,
      // Cut, the name is wider than the room it has.
      cut: name.scrollWidth > name.clientWidth,
      text: name.textContent,
      sorted: th.getAttribute("aria-sort"),
      menu: !!th.querySelector('[part="column-menu-button"]'),
    };
  });

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
});

test("a narrow sorted column with a menu still shows its name", async ({ page }) => {
  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    window.__opengridModule.set_columns(grid, { id: { width: 60 } });
    window.__opengridModule.set_view(grid, { sort: [{ field: "id", direction: "asc" }] });
  });
  await expect.poll(async () => (await header(page)).sorted).toBe("ascending");
  const { column, name, text, menu, cut } = await header(page);
  expect(menu).toBe(true);
  expect(text).toBe("id");
  // The page asked for 60 px; the header needs more, and gets it.
  expect(column).toBeGreaterThanOrEqual(96);
  // Two characters fit whole at the narrowest width.
  expect(name).toBeGreaterThan(0);
  expect(cut).toBe(false);
});

test("without the menu the narrowest column is smaller, and the name still shows", async ({ page }) => {
  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    grid.removeAttribute("column-menu");
    window.__opengridModule.set_columns(grid, { id: { width: 40 } });
    window.__opengridModule.set_view(grid, { sort: [{ field: "id", direction: "desc" }] });
  });
  await expect.poll(async () => (await header(page)).sorted).toBe("descending");
  const { column, name, menu, cut } = await header(page);
  expect(menu).toBe(false);
  expect(column).toBeGreaterThanOrEqual(64);
  expect(column).toBeLessThan(96);
  expect(name).toBeGreaterThan(0);
  expect(cut).toBe(false);
});

/// Beside a longer name only what is there is reserved: at 110 px a sorted
/// `customer` with a menu keeps about 45 px of its name, not the 25 px a fixed
/// worst-case reserve left it.
test("a sorted column reserves only what stands beside its name", async ({ page }) => {
  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    window.__opengridModule.set_columns(grid, { customer: { width: 110 } });
    window.__opengridModule.set_view(grid, { sort: [{ field: "customer", direction: "asc" }] });
  });
  const name = () =>
    page.evaluate(() => {
      const th = document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="1"]');
      return { sorted: th.getAttribute("aria-sort"), width: th.querySelector("span:first-child").getBoundingClientRect().width };
    });
  await expect.poll(async () => (await name()).sorted).toBe("ascending");
  expect((await name()).width).toBeGreaterThanOrEqual(40);
});
