import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow } from "./helpers.js";

// Cell templates (issue #317): a node the page builds instead of a cell's text.

const cell = (page, row, col) =>
  shadow(
    page,
    (root, { row, col }) => {
      const td = root.querySelector(`td[data-row="${row}"][data-col="${col}"]`);
      return td && { html: td.innerHTML, text: td.textContent, template: td.hasAttribute("data-template") };
    },
    { row, col },
  );

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-template.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

const where = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active ? `${active.tagName.toLowerCase()}:${active.dataset.row ?? ""}:${active.dataset.col ?? ""}` : null;
  });

test("a link and a badge are drawn from the page's nodes, a string as text, and axe is content", async ({ page }) => {
  await open(page);
  expect((await cell(page, 1, 1)).html).toBe('<a href="#customer-2" data-id="2" tabindex="-1">Customer 2</a>');
  expect((await cell(page, 0, 2)).html).toBe('<span class="badge badge-open">open</span>');
  expect((await cell(page, 1, 3)).text).toBe("note 2!");
  expect((await cell(page, 1, 0)).template).toBe(false);
  // The grid's one tab stop stays one: no link is tabbable.
  expect(await shadow(page, (root) => [...root.querySelectorAll("td a")].every((a) => a.tabIndex === -1))).toBe(true);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("Enter goes into the link, Escape comes back, and the arrows move on", async ({ page }) => {
  await open(page);
  await focusCell(page, 1, 1);
  await page.keyboard.press("Enter");
  expect(await where(page)).toBe("a::");
  // Inside, the keys are the link's: Enter follows it.
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => location.hash)).toBe("#customer-2");
  await page.keyboard.press("Escape");
  expect(await where(page)).toBe("td:1:1");
  expect(await shadow(page, (root) => root.querySelector("td a").tabIndex)).toBe(-1);
  await page.keyboard.press("ArrowDown");
  expect(await where(page)).toBe("td:2:1");
});

test("a scroll draws the cells for the rows now under them, nothing stale", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 0);
  for (let i = 0; i < 30; i++) await page.keyboard.press("ArrowDown");
  await expect.poll(() => shadow(page, (root) => root.querySelector('td[data-row="30"][data-col="1"]') !== null)).toBe(true);
  const stale = await shadow(page, (root) =>
    [...root.querySelectorAll("tbody td[data-col=\"1\"]")].filter((td) => {
      const link = td.querySelector("a");
      return !link || link.dataset.id !== String(Number(td.dataset.row) + 1) || link.textContent !== `Customer ${Number(td.dataset.row) + 1}`;
    }).length,
  );
  expect(stale).toBe(0);
  // A cell that did not change is not drawn again: at rest, nothing is built.
  let built = -1;
  await expect
    .poll(async () => {
      const now = await page.evaluate(() => window.__built);
      const settled = now === built;
      built = now;
      return settled;
    }, { intervals: [250] })
    .toBe(true);
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.__built)).toBe(built);
});

test("search and sort still see the value, and taking the templates away gives the text back", async ({ page }) => {
  await open(page);
  await page.locator("opengrid-grid").locator('[part="search-input"]').fill("status = open");
  await page.keyboard.press("Enter");
  await expect.poll(() => shadow(page, (root) => [...root.querySelectorAll('td[data-col="2"]')].map((td) => td.textContent).join(","))).toMatch(/^open(,open)+$/);
  expect((await cell(page, 0, 2)).template).toBe(true);

  await page.evaluate(() => window.__module.set_cell_templates(document.querySelector("opengrid-grid"), {}));
  await expect.poll(async () => (await cell(page, 0, 1)).template).toBe(false);
  expect((await cell(page, 0, 1)).html).toBe("Customer 1");
  await page.evaluate(() => window.__module.set_cell_templates(document.querySelector("opengrid-grid"), window.__templates));
  await expect.poll(async () => (await cell(page, 0, 1)).template).toBe(true);
});
