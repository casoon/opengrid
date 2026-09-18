import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Plan point 28: `mode` decides where a query runs, and the answer must not
// depend on that decision. The source here is a real `opengrid-server` (started
// by the Playwright config), the client half is the engine in the tab.
//
// The fixture records every plan the provider made in `window.__plans`, so the
// split is read rather than guessed from the rows.

const READY = () => window.__ready === true;

async function open(page, mode) {
  await page.goto("/tests/e2e/fixtures/hybrid.html");
  await page.waitForFunction(READY);
  if (mode) {
    await page.evaluate((value) => {
      window.__plans.length = 0;
      document.querySelector("opengrid-grid").setAttribute("mode", value);
    }, mode);
  }
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

/** The text of the first column of the rendered rows. */
function firstColumn(page) {
  return page.locator("opengrid-grid tbody tr td:first-child").allTextContents();
}

/**
 * Focuses a header cell and toggles its sort, the way a keyboard user does,
 * then waits for the query that follows — the plans are the signal that the
 * round trip through the server and back is over.
 */
async function toggleSort(page, column) {
  const planned = await page.evaluate(() => window.__plans.length);
  await page.evaluate((column) => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector(`th[data-col="${column}"]`)
      .focus();
  }, column);
  await page.keyboard.press("Enter");
  // A plan is made *before* the request goes out, so it says the query started,
  // not that its rows are on screen. The caller waits for the rows.
  await page.waitForFunction((seen) => window.__plans.length > seen, planned);
}

/** The last plan the provider made. */
function lastPlan(page) {
  return page.evaluate(() => window.__plans.at(-1));
}

test.describe("hybrid execution against a real server", () => {
  test("the server answers what it can, and says what that is", async ({ page }) => {
    await open(page);

    // The capability declaration is what the split is decided from.
    const capabilities = await page.evaluate(() => window.__capabilities);
    expect(capabilities.filter).toBe(true);
    expect(capabilities.paging).toBe(true);

    const plan = await lastPlan(page);
    expect(plan.mode).toBe("auto");
    // The grid sorts by its first column from the start (rule S6: paging needs
    // a total order), so the query that travels is a sorted, paged one.
    expect(plan.describe).toBe("source: sort · page | client: —");
    expect(plan.client).toBeNull();
    expect(await firstColumn(page)).toHaveLength(10);
  });

  test("mode=local runs the whole query in the tab, with the same rows", async ({ page }) => {
    await open(page);
    const remoteRows = await firstColumn(page);

    await open(page, "local");
    const plan = await lastPlan(page);
    expect(plan.mode).toBe("local");
    expect(plan.describe).toBe("source: scan | client: sort · page");
    expect(plan.steps).toEqual(["sort", "page"]);
    // The source query asks for rows, nothing else.
    expect(plan.source.limit ?? null).toBeNull();

    expect(await firstColumn(page)).toEqual(remoteRows);
  });

  test("a sort the client does holds the paging back", async ({ page }) => {
    await open(page, "local");
    const before = await firstColumn(page);
    await toggleSort(page, 1);

    // The rows really were reordered here, by the engine, over rows the server
    // handed over unsorted.
    await expect.poll(() => firstColumn(page)).not.toEqual(before);

    const plan = await lastPlan(page);
    expect(plan.client.sort[0].field).toBe("customer");
    // **The rule of 05-planner.md.** The client sorts, so the client pages:
    // a `limit` sent ahead of the sort would page the wrong rows.
    expect(plan.describe).toBe("source: scan | client: sort · page");
    expect(plan.source.sort).toEqual([]);
    expect(plan.source.limit ?? null).toBeNull();
  });

  test("the three explicit modes are enforceable and agree", async ({ page }) => {
    await open(page);
    const expected = await firstColumn(page);

    for (const mode of ["remote", "hybrid", "local"]) {
      await open(page, mode);
      const plan = await lastPlan(page);
      expect(plan.mode).toBe(mode);
      expect(await firstColumn(page), `mode=${mode}`).toEqual(expected);
    }
  });

  test("the hybrid page has no accessibility violations", async ({ page }) => {
    await open(page);
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });
});
