import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` loading, empty and error states (plan point 41).
//
// The fixture is `grid.html`'s data behind a provider that can be switched into
// a slow or a failing mode (`window.__mode`), so all four states of the status
// line are reachable:
//
//   ready   the result count, "5 Treffer"
//   loading while a query runs, released with `window.__release()`
//   empty   a filter that matches nothing
//   error   the engine answering for a source that was never loaded
//
// What each test asserts is the same thing in different words: the state is
// **visible** in the grid and **announced** through one polite live region —
// and, for the error, that the grid is still there afterwards.

/** The status line: its text and the ARIA that makes it an announcement. */
async function status(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const line = root.querySelector('[part="status"]');
    if (!line) return null;
    return {
      text: line.textContent,
      role: line.getAttribute("role"),
      live: line.getAttribute("aria-live"),
      state: line.getAttribute("data-state"),
      // A live region inside the `role="grid"` table would be read as a cell.
      outsideGrid: !line.closest('table[role="grid"]'),
      visible: !!line.offsetParent && line.textContent.trim().length > 0,
      // Exactly one live region, or a state is announced twice.
      regions: root.querySelectorAll('[role="status"], [role="alert"], [aria-live]').length,
    };
  });
}

/** The status text alone, for polling. */
async function statusText(page) {
  return (await status(page))?.text ?? null;
}

/** Whether the `role="grid"` table is still rendered, and how many rows it has. */
async function gridFacts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const table = root.querySelector('table[role="grid"]');
    return {
      present: !!table,
      rows: root.querySelectorAll("tbody tr[aria-rowindex]").length,
      rowcount: table?.getAttribute("aria-rowcount") ?? null,
      focusedCell: root.activeElement?.getAttribute("data-row") ?? null,
    };
  });
}

/** Focuses an element inside the shadow root. */
async function focusIn(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

/** Replaces a filter value and applies it with Enter (column 1 is `customer`). */
async function applyFilter(page, value) {
  await focusIn(page, 'input[data-col="1"]');
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(value);
  await page.keyboard.press("Enter");
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-states.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("the status line is one visible, polite live region outside the grid", async ({
  page,
}) => {
  const line = await status(page);
  expect(line).toMatchObject({
    text: "5 Treffer",
    role: "status",
    live: "polite",
    state: "ready",
    outsideGrid: true,
    visible: true,
    regions: 1,
  });
});

test("a running query announces that the grid is loading", async ({ page }) => {
  await page.evaluate(() => {
    window.__mode = "slow";
  });
  await applyFilter(page, "Alpha");

  // The query is held by the fixture, so the loading state is observable.
  await expect.poll(() => statusText(page)).toBe("Wird geladen …");
  expect(await status(page)).toMatchObject({ state: "loading", visible: true });

  await page.evaluate(() => {
    window.__mode = "ok";
    window.__release();
  });
  await expect.poll(() => statusText(page)).toBe("2 Treffer");
});

test("a result without rows announces that nothing matched", async ({ page }) => {
  await applyFilter(page, "Delta");

  await expect.poll(() => statusText(page)).toBe("Keine Treffer");
  expect(await status(page)).toMatchObject({ state: "empty", visible: true });
  // The grid stays, with no data rows and only the header counted.
  expect(await gridFacts(page)).toMatchObject({ present: true, rows: 0, rowcount: "1" });
});

test("a failed query names its cause and leaves the grid standing", async ({ page }) => {
  await page.evaluate(() => {
    window.__mode = "error";
  });
  await applyFilter(page, "Alpha");

  await expect
    .poll(() => statusText(page))
    .toBe('Die Daten konnten nicht geladen werden: unknown source "missing"');
  const line = await status(page);
  expect(line).toMatchObject({ state: "error", role: "status", visible: true, regions: 1 });

  // The point of the state: the table, its rows and its row count survive the
  // failure instead of being replaced by an error page.
  const grid = await gridFacts(page);
  expect(grid.present).toBe(true);
  expect(grid.rows).toBe(5);

  // And the next successful query clears the error.
  await page.evaluate(() => {
    window.__mode = "ok";
  });
  await applyFilter(page, "Alpha");
  await expect.poll(() => statusText(page)).toBe("2 Treffer");
});

test("keyboard navigation still works after a failed query", async ({ page }) => {
  await page.evaluate(() => {
    window.__mode = "error";
  });
  await applyFilter(page, "Alpha");
  await expect.poll(() => status(page).then((line) => line.state)).toBe("error");

  // The roving tabindex is intact: a cell takes the focus and the arrow keys
  // move it. A cleared shadow root would have made this impossible.
  await focusIn(page, 'td[data-row="0"][data-col="0"]');
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => gridFacts(page).then((grid) => grid.focusedCell)).toBe("1");
});

test("has no axe violations in the error state", async ({ page }) => {
  await page.evaluate(() => {
    window.__mode = "error";
  });
  await applyFilter(page, "Alpha");
  await expect.poll(() => status(page).then((line) => line.state)).toBe("error");

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
