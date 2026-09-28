import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The toolbar on a narrow grid (issue #77). Wrapped, it took four rows of a
// phone's grid — 178 of 460 px — and left room for four rows of data. Below a
// grid width of 560 px, the search field takes the full width and the buttons
// stay in one row that scrolls sideways. Not at a 320 px viewport, the reflow
// case (1.4.10): there the toolbar wraps as before.

async function open(page, width) {
  await page.setViewportSize({ width, height: 800 });
  await page.goto("/tests/e2e/fixtures/grid-search.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
      ),
    )
    .toBe("200 matches");
}

/** The toolbar's shape: how tall, how many lines its buttons take, whether the row scrolls. */
async function shape(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const toolbar = root.querySelector('[part="toolbar"]');
    const buttons = [...toolbar.querySelectorAll("button")].filter((button) => !button.closest('[part="columns"]'));
    const row = toolbar.querySelector("[data-toolbar-row]");
    return {
      height: Math.round(toolbar.getBoundingClientRect().height),
      lines: new Set(buttons.map((button) => { const box = button.getBoundingClientRect(); return Math.round((box.top + box.bottom) / 2); })).size,
      display: getComputedStyle(row).display,
      scrolls: row.scrollWidth > row.clientWidth,
      pageScrolls: document.documentElement.scrollWidth > window.innerWidth,
    };
  });
}

test("on a phone the buttons keep to one row under the search field", async ({ page }) => {
  await open(page, 375);
  const narrow = await shape(page);
  expect(narrow.display).toBe("flex");
  expect(narrow.lines).toBe(1);
  expect(narrow.scrolls).toBe(true);
  expect(narrow.pageScrolls).toBe(false);
  // Search and one row of buttons: two rows, not four.
  expect(narrow.height).toBeLessThanOrEqual(110);
});

test("at a 320 px viewport the toolbar wraps, as reflow asks", async ({ page }) => {
  await open(page, 320);
  const reflow = await shape(page);
  expect(reflow.display).toBe("contents");
  expect(reflow.lines).toBeGreaterThan(1);
  expect(reflow.pageScrolls).toBe(false);
});

test("a wide grid keeps its one toolbar row", async ({ page }) => {
  await open(page, 1280);
  const wide = await shape(page);
  expect(wide.display).toBe("contents");
  expect(wide.lines).toBe(1);
  expect(wide.height).toBeLessThanOrEqual(64);
});

test("Tab reaches every button of the row, and each comes into view", async ({ page, browserName }) => {
  await open(page, 375);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-input"]').focus(),
  );
  const seen = [];
  for (let step = 0; step < 12; step += 1) {
    // WebKit on macOS moves Tab between text fields only; Option+Tab is Tab.
    await page.keyboard.press(browserName === "webkit" ? "Alt+Tab" : "Tab");
    const focus = await page.evaluate(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const active = root.activeElement;
      const row = root.querySelector("[data-toolbar-row]");
      if (!active || !row.contains(active)) return null;
      const box = active.getBoundingClientRect();
      const frame = row.getBoundingClientRect();
      return {
        name: active.getAttribute("part") ?? active.dataset.density,
        visible: box.left >= frame.left - 1 && box.right <= frame.right + 1,
      };
    });
    if (!focus) break;
    seen.push(focus);
  }
  expect(seen.map((focus) => focus.name)).toEqual([
    "add-filter",
    "add-grouping",
    "filter-row-toggle",
    "columns-toggle",
    "compact",
    "normal",
    "comfortable",
  ]);
  for (const focus of seen) expect(focus.visible, focus.name).toBe(true);
});

test("the column list opens on a line of its own, below the row", async ({ page }) => {
  await open(page, 375);
  const place = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('[part="columns-toggle"]').click();
    const panel = root.querySelector('[part="columns"]');
    const row = root.querySelector("[data-toolbar-row]");
    return {
      open: !panel.hidden,
      inRow: row.contains(panel),
      below: panel.getBoundingClientRect().top >= row.getBoundingClientRect().bottom - 4,
    };
  });
  expect(place).toEqual({ open: true, inRow: false, below: true });
});

test("has no axe violations on a phone", async ({ page }) => {
  await open(page, 375);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
