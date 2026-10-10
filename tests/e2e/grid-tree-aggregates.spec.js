import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Subtree aggregates in the tree (issue #165, rule T7), on the conformance org
// chart: Sales(0) → North(100) → Alice(30) → Eve(3), Bob(NULL); Sales →
// South(50) → Carol(40); Partners(10) → Dave(5); Orphan(7). Sorted by name,
// the roots read Orphan, Partners, Sales.

/** Name, the revenue cell's text and what it says, per drawn row. */
async function revenue(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.style.display)
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .map((tr) => {
        const cell = tr.querySelector('td[data-col="2"]');
        return [
          tr.querySelector('td[data-col="0"]').textContent,
          cell.textContent,
          cell.getAttribute("aria-label"),
        ];
      });
  });
}

async function settled(page) {
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    const line = root?.querySelector('[part="status"]');
    return !!line && line.getAttribute("data-state") !== "loading" && !!root.querySelector("td[data-row]");
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-tree.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await settled(page);
  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.querySelector("opengrid-grid"), {
      revenue: { aggregate: "sum" },
    }),
  );
});

test("a node shows its own value and its subtree's sum, and says both", async ({ page }) => {
  await expect.poll(() => revenue(page)).toEqual([
    // A leaf: its own value, nothing else.
    ["Orphan", "7", null],
    ["Partners", "10 · Σ 15", "10. Sum of the subtree: 15"],
    ["Sales", "0 · Σ 223", "0. Sum of the subtree: 223"],
  ]);
  // The level was asked with the summary over each subtree.
  const asked = await page.evaluate(() => window.__queries.map((query) => JSON.parse(query)));
  expect(asked.at(-1).tree.aggregate).toEqual([{ fn: "sum", field: "revenue", as: "__og_a0" }]);
});

test("an opened level brings its own sums; NULL is no value", async ({ page }) => {
  await expect.poll(() => revenue(page).then((rows) => rows.length)).toBe(3);
  // Sales → North → Alice.
  for (const [row, count] of [
    [2, 5],
    [3, 7],
    [4, 8],
  ]) {
    await page.evaluate((row) => {
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`td[data-row="${row}"][data-col="0"]`)
        .focus();
    }, row);
    await page.keyboard.press("ArrowRight");
    await expect.poll(() => revenue(page).then((rows) => rows.length)).toBe(count);
  }
  const rows = await revenue(page);
  expect(rows.slice(3)).toEqual([
    // North: 100 + Alice 30 + Eve 3; Bob's NULL is not a value (S11).
    ["North", "100 · Σ 133", "100. Sum of the subtree: 133"],
    ["Alice", "30 · Σ 33", "30. Sum of the subtree: 33"],
    ["Eve", "3", null],
    ["Bob", "", null],
    ["South", "50 · Σ 90", "50. Sum of the subtree: 90"],
  ]);
});

test("another summary from the view asks the tree again", async ({ page }) => {
  await expect.poll(() => revenue(page).then((rows) => rows[2]?.[1])).toBe("0 · Σ 223");
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      aggregates: { revenue: "max" },
    }),
  );
  await expect.poll(() => revenue(page).then((rows) => rows[2])).toEqual([
    "Sales",
    "0 · max 100",
    "0. Maximum of the subtree: 100",
  ]);
});

test("has no axe violations with subtree sums shown", async ({ page }) => {
  await expect.poll(() => revenue(page).then((rows) => rows[2]?.[1])).toBe("0 · Σ 223");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});
