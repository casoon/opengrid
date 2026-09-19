import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-pivot>` against a real `opengrid-server` (plan points 32 and 53).
//
// Table Mode: a native <table>, a two-level column header, a row header per row
// and subtotals that say so. The structure is asserted, not just the numbers —
// a pivot whose headers do not connect to its cells is unreadable to a screen
// reader even when every value is right.

/** The pivot's shadow root, as facts. */
async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-pivot").shadowRoot;
    const headerRows = [...root.querySelectorAll("thead tr")];
    return {
      caption: root.querySelector("caption")?.textContent ?? null,
      ariaLabel: root.querySelector("table")?.getAttribute("aria-label") ?? null,
      headerRows: headerRows.length,
      colgroups: headerRows[0]
        ? [...headerRows[0].querySelectorAll('th[scope="colgroup"]')].map((th) => ({
            text: th.textContent,
            colspan: th.getAttribute("colspan"),
          }))
        : [],
      measureHeaders: headerRows[1]
        ? [...headerRows[1].querySelectorAll('th[scope="col"]')].map((th) => th.textContent)
        : [],
      rowHeaders: [...root.querySelectorAll('tbody th[scope="row"]')].map((th) => th.textContent),
      levels: [...root.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-level")),
      totals: [...root.querySelectorAll("tbody tr[data-total]")].length,
      status: root.querySelector('[part="status"]')?.textContent ?? null,
      state: root.querySelector('[part="status"]')?.getAttribute("data-state") ?? null,
    };
  });
}

async function open(page) {
  await page.goto("/tests/e2e/fixtures/pivot.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-pivot tbody tr").first()).toBeVisible();
}

test.describe("pivot", () => {
  test("renders a native table with a two-level column header", async ({ page }) => {
    await open(page);
    const seen = await facts(page);

    expect(seen.caption).toBe("Orders by country and year");
    expect(seen.ariaLabel).toBe("Orders by country and year");
    expect(seen.headerRows).toBe(2);
    // One group per column value that occurs, each spanning its two measures.
    // The NULL year is a group like any other and it has a **name**: an empty
    // header cell is silence to a screen reader.
    expect(seen.colgroups.map((group) => group.text)).toEqual(["2025", "2026", "(no value)"]);
    expect(seen.colgroups.every((group) => group.colspan === "2")).toBe(true);
    expect(seen.measureHeaders).toEqual(["total", "n", "total", "n", "total", "n"]);
    expect(seen.state).toBe("ready");
  });

  test("every row has a header and the grand total says that it is one", async ({ page }) => {
    await open(page);
    const seen = await facts(page);

    // One header per row, and the last row is the grand total.
    expect(seen.rowHeaders.length).toBe(seen.levels.length);
    expect(seen.levels.at(-1)).toBe("0");
    expect(seen.rowHeaders.at(-1)).toBe("Total");
    expect(seen.totals).toBe(1);
    // A real NULL country group is a row of its own, *not* the total — that is
    // the whole reason levels exist (S10/P2) — and it is named, not blank.
    expect(seen.levels.filter((level) => level === "1").length).toBeGreaterThan(1);
    expect(seen.rowHeaders).toContain("(no value)");
    expect(seen.rowHeaders.every((text) => text.trim().length > 0)).toBe(true);
  });

  test("the numbers are the server's, and they add up", async ({ page }) => {
    await open(page);
    const rows = await page.evaluate(() => {
      const root = document.querySelector("opengrid-pivot").shadowRoot;
      return [...root.querySelectorAll("tbody tr")].map((tr) =>
        [...tr.querySelectorAll("td")].map((td) => td.textContent),
      );
    });

    // Column 1 is `n` for 2025, column 3 `n` for 2026, column 5 `n` for NULL.
    const counts = (row) => [1, 3, 5].map((index) => Number(row[index] || 0));
    const total = counts(rows.at(-1));
    const summed = rows
      .slice(0, -1)
      .reduce((acc, row) => counts(row).map((value, i) => value + acc[i]), [0, 0, 0]);
    expect(summed).toEqual(total);
  });

  test("a broken measure list is reported, not swallowed", async ({ page }) => {
    await open(page);
    await page.evaluate(() =>
      document.querySelector("opengrid-pivot").setAttribute("values", "sum(qty)"),
    );
    await expect
      .poll(async () => (await facts(page)).state)
      .toBe("error");
    const seen = await facts(page);
    expect(seen.status).toContain("values");
  });

  test("has no axe violations", async ({ page }) => {
    await open(page);
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });
});
