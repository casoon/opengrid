import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Pagination (plan point 38): the *other* mode, not an addition to
// virtualization. With `page-size` the grid shows exactly one page, nothing
// scrolls, and `aria-rowcount` counts what is there — a screen reader reads the
// page, not a promise about the rest.

async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const pager = root.querySelector('[part="pager"]');
    const viewport = root.querySelector('[part="viewport"]');
    return {
      rows: root.querySelectorAll("tbody tr[aria-rowindex]").length,
      rowcount: root.querySelector("table").getAttribute("aria-rowcount"),
      rowindexes: [...root.querySelectorAll("tbody tr[aria-rowindex]")].map((tr) =>
        tr.getAttribute("aria-rowindex"),
      ),
      ids: [...root.querySelectorAll('tbody td[data-col="0"]')].map((td) => td.textContent),
      pagerHidden: pager.hasAttribute("hidden"),
      label: root.querySelector('[part="page-label"]')?.textContent ?? null,
      disabled: ["page-first", "page-previous", "page-next", "page-last"].map((part) =>
        root.querySelector(`[part="${part}"]`).hasAttribute("disabled"),
      ),
      status: root.querySelector('[part="status"]').textContent,
      // The sizer is what virtualization inflates to the whole result; while
      // paging it is the page.
      sizer: root.querySelector("tbody").getBoundingClientRect().height,
      viewportHeight: viewport.clientHeight,
    };
  });
}

async function click(page, part) {
  await page.evaluate((part) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(`[part="${part}"]`).click();
  }, part);
}

async function open(page, url = "/tests/e2e/fixtures/grid-paged.html") {
  await page.goto(url);
  // The fixtures signal readiness differently; the rows are the honest signal.
  await page.waitForFunction(() => document.querySelector("opengrid-grid")?.shadowRoot);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

test("a page is the whole table, and it says so", async ({ page }) => {
  await open(page);
  const seen = await facts(page);

  expect(seen.rows).toBe(5);
  // `aria-rowcount` counts the page plus the header — what is in the DOM.
  expect(seen.rowcount).toBe("6");
  expect(seen.rowindexes).toEqual(["2", "3", "4", "5", "6"]);
  expect(seen.pagerHidden).toBe(false);
  expect(seen.label).toBe("Page 1 of 12");
  // The total belongs in the status line, where the whole result is reported.
  expect(seen.status).toContain("60 matches");
  // The sizer is the **page**, not the result: 5 rows at 32px, not 60.
  expect(seen.sizer).toBeLessThan(200);
  // On the first page there is no way back, and the button says so.
  expect(seen.disabled).toEqual([true, true, false, false]);
});

test("paging moves through the result and back", async ({ page }) => {
  await open(page);
  const first = (await facts(page)).ids;

  await click(page, "page-next");
  await expect.poll(async () => (await facts(page)).label).toBe("Page 2 of 12");
  const second = (await facts(page)).ids;
  expect(second).not.toEqual(first);
  expect((await facts(page)).rowindexes).toEqual(["2", "3", "4", "5", "6"]);

  await click(page, "page-last");
  await expect.poll(async () => (await facts(page)).label).toBe("Page 12 of 12");
  expect((await facts(page)).disabled).toEqual([false, false, true, true]);

  await click(page, "page-first");
  await expect.poll(async () => (await facts(page)).ids).toEqual(first);
});

test("the focus lands on the new page, not in the void", async ({ page }) => {
  await open(page);
  await click(page, "page-next");
  await expect.poll(async () => (await facts(page)).label).toBe("Page 2 of 12");

  const focused = await page.evaluate(() => {
    const active = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    return active ? { tag: active.tagName, row: active.getAttribute("data-row") } : null;
  });
  expect(focused).toEqual({ tag: "TD", row: "5" });
});

test("paging is reachable with the keyboard alone", async ({ page }) => {
  await open(page);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="page-next"]').focus(),
  );
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await facts(page)).label).toBe("Page 2 of 12");
});

test("without page-size nothing is paged", async ({ page }) => {
  await open(page, "/tests/e2e/fixtures/grid-long.html");
  const seen = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      pagerHidden: root.querySelector('[part="pager"]').hasAttribute("hidden"),
      rowcount: root.querySelector("table").getAttribute("aria-rowcount"),
    };
  });
  expect(seen.pagerHidden).toBe(true);
  // Virtualizing: the count is the whole result, because the window promises
  // to reach all of it.
  expect(Number(seen.rowcount)).toBeGreaterThan(11);
});

test("has no axe violations", async ({ page }) => {
  await open(page);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
