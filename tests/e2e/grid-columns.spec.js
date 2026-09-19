import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Columns: width, order, visibility (plan point 36).
//
// The point of this file is **WCAG 2.2 § 2.5.7 Dragging Movements**: everything
// here is done with the keyboard alone. A column handle that only a mouse can
// grab fails that criterion, so the keyboard is the primary way and not a
// fallback.

async function focusHeader(page, col) {
  await page.evaluate((col) => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector(`th[data-col="${col}"]`)
      .focus();
  }, col);
}

async function headers(page) {
  return page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("thead th")].map(
      (th) => th.querySelector("span")?.textContent ?? th.textContent,
    ),
  );
}

async function status(page) {
  return page.evaluate(
    () =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
        .textContent,
  );
}

async function widthOf(page, col) {
  return page.evaluate(
    (col) =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`th[data-col="${col}"]`)
        .getBoundingClientRect().width,
    col,
  );
}

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.waitForFunction(() => document.querySelector("opengrid-grid")?.shadowRoot);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

test("a column moves with the keyboard alone", async ({ page }) => {
  await open(page);
  const before = await headers(page);

  await focusHeader(page, 0);
  await page.keyboard.press("Control+ArrowRight");

  await expect.poll(() => headers(page)).toEqual([before[1], before[0], ...before.slice(2)]);
  // The announcement rides along with the result the move triggered, so it
  // lands a moment after the headers do.
  await expect.poll(() => status(page)).toContain("position 2");

  // The focus follows the column, not the place it left.
  const focused = await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.activeElement?.getAttribute("data-col"),
  );
  expect(focused).toBe("1");
});

test("a column at the end says so instead of wrapping", async ({ page }) => {
  await open(page);
  const before = await headers(page);

  await focusHeader(page, 0);
  await page.keyboard.press("Control+ArrowLeft");

  expect(await headers(page)).toEqual(before);
  expect(await status(page)).toContain("already at the end");
});

test("a column is resized with the keyboard alone, and never into nothing", async ({ page }) => {
  await open(page);
  await focusHeader(page, 0);
  const before = await widthOf(page, 0);

  await page.keyboard.press("Control+Shift+ArrowRight");
  await expect.poll(() => widthOf(page, 0)).toBeGreaterThan(before);
  expect(await status(page)).toContain("pixels wide");

  // Far past the minimum: the clamp holds, and the header stays big enough to
  // hit (WCAG 2.5.8 asks for 24px).
  for (let i = 0; i < 40; i += 1) {
    await page.keyboard.press("Control+Shift+ArrowLeft");
  }
  expect(await widthOf(page, 0)).toBeGreaterThanOrEqual(24);
});

test("a column is hidden and brought back from the checkbox list", async ({ page }) => {
  await open(page);
  const before = await headers(page);

  // The list is a disclosure: open it first, the way a reader would.
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="columns-toggle"]')
      .click(),
  );
  expect(
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('[part="columns"]')
        .hasAttribute("hidden"),
    ),
  ).toBe(false);

  const toggle = (name) =>
    page.evaluate((name) => {
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`input[data-column="${name}"]`)
        .click();
    }, name);

  await toggle(before[1]);
  await expect.poll(() => headers(page)).not.toContain(before[1]);
  await expect.poll(() => status(page)).toContain("hidden");

  // The checkbox is still there — that is the way back.
  await toggle(before[1]);
  await expect.poll(() => headers(page)).toContain(before[1]);
});

test("a hidden column is out of the DOM, not just invisible", async ({ page }) => {
  await open(page);
  const before = await headers(page);
  const columns = () =>
    page.evaluate(() =>
      Number(
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector("table")
          .getAttribute("aria-colcount"),
      ),
    );
  const first = await columns();

  await page.evaluate((name) => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector(`input[data-column="${name}"]`)
      .click();
  }, before[1]);

  // A screen reader must not keep reading a column nobody can see.
  await expect.poll(columns).toBe(first - 1);
  const cells = await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr:first-child td")
      .length,
  );
  expect(cells).toBe(first - 1);
});

test("has no axe violations", async ({ page }) => {
  await open(page);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
