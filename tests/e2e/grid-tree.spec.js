import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The tree in the grid (issue #135, plan point 123, E38).
//
// The fixture is the conformance org chart: Sales(1) → North(2), South(3);
// North → Alice(4) → Eve(10), Bob(5); South → Carol(6); Partners(7) → Dave(8);
// and Orphan(9), whose parent 99 does not exist (T2). The grid sorts by its
// first column, `name`, so the roots read Orphan, Partners, Sales.

/** The rows currently drawn, in display order. */
async function drawn(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.style.display)
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .map((tr) => ({
        name: tr.querySelector('td[data-col="0"]')?.textContent ?? "",
        level: tr.getAttribute("aria-level"),
        posinset: tr.getAttribute("aria-posinset"),
        setsize: tr.getAttribute("aria-setsize"),
        expanded: tr.getAttribute("aria-expanded"),
        context: tr.hasAttribute("data-context"),
      }));
  });
}

const names = async (page) => (await drawn(page)).map((row) => row.name);

async function rowcount(page) {
  return page.evaluate(() =>
    Number(
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector("table")
        .getAttribute("aria-rowcount"),
    ),
  );
}

async function status(page) {
  return page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="status"]')
      .textContent.trim(),
  );
}

/** Focuses a cell and presses a key on it, the way the keyboard would. */
async function press(page, selector, key) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
  await page.keyboard.press(key);
}

/** The name in the cell that has the focus. */
const focused = (page) =>
  page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return root.activeElement?.closest("tr")?.querySelector('td[data-col="0"]')?.textContent;
  });

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
  // The three roots, and the header row.
  await expect.poll(() => rowcount(page)).toBe(4);
});

test("a tree is a treegrid of its roots, a leaf offers nothing to open", async ({ page }) => {
  expect(
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector("table").getAttribute("role"),
    ),
  ).toBe("treegrid");
  expect(await drawn(page)).toEqual([
    { name: "Orphan", level: "1", posinset: "1", setsize: "3", expanded: null, context: false },
    { name: "Partners", level: "1", posinset: "2", setsize: "3", expanded: "false", context: false },
    { name: "Sales", level: "1", posinset: "3", setsize: "3", expanded: "false", context: false },
  ]);
  // Every row of the tree counts, not the three shown; the orphan is said.
  const line = await status(page);
  expect(line).toContain("10");
  expect(line).toContain("Without a parent, shown at the top: 1");
  // One query: the roots, with their child counts.
  expect(await page.evaluate(() => window.__queries.length)).toBe(1);
});

test("→ opens a node, ← closes it and climbs to the parent", async ({ page }) => {
  await press(page, 'td[data-row="2"][data-col="0"]', "ArrowRight");
  await expect.poll(() => names(page)).toEqual(["Orphan", "Partners", "Sales", "North", "South"]);
  const rows = await drawn(page);
  expect(rows[2].expanded).toBe("true");
  expect(rows.slice(3).map((row) => [row.level, row.posinset, row.setsize])).toEqual([
    ["2", "1", "2"],
    ["2", "2", "2"],
  ]);
  expect(await status(page)).toContain("Sales expanded");
  expect(await focused(page)).toBe("Sales");

  // North, opened by Enter: Alice and Bob at level 3.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect
    .poll(() => names(page))
    .toEqual(["Orphan", "Partners", "Sales", "North", "Alice", "Bob", "South"]);
  // A level is one query each: the roots, Sales' children, North's.
  expect(await page.evaluate(() => window.__queries.length)).toBe(3);

  // ← on Bob, a leaf: up to North. ← on North, open: it closes.
  await press(page, 'td[data-row="5"][data-col="0"]', "ArrowLeft");
  await expect.poll(() => focused(page)).toBe("North");
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => names(page)).toEqual(["Orphan", "Partners", "Sales", "North", "South"]);
  expect(await status(page)).toContain("North collapsed");

  // Opening it again asks nothing: the level is loaded.
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => names(page)).toContain("Alice");
  expect(await page.evaluate(() => window.__queries.length)).toBe(3);
});

test("a click on a node's first cell opens it", async ({ page }) => {
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('td[data-row="1"][data-col="0"]')
      .click(),
  );
  await expect.poll(() => names(page)).toEqual(["Orphan", "Partners", "Dave", "Sales"]);
});

test("a filter keeps the path to a match, as context", async ({ page }) => {
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  view.filters = [{ column: "name", op: "eq", value: "Eve" }];
  // Open down to Eve: Sales, North, Alice — each `[key]`.
  view.expanded = [[1], [2], [4]];
  await page.evaluate(
    (view) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), view),
    view,
  );
  await expect.poll(() => names(page)).toEqual(["Sales", "North", "Alice", "Eve"]);
  const rows = await drawn(page);
  expect(rows.map((row) => row.context)).toEqual([true, true, true, false]);
  // The context is named for the ear, on its first cell.
  expect(
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="0"][data-col="0"]')
        .getAttribute("aria-description"),
    ),
  ).toBe("context");
  // One match, not four rows.
  expect(await status(page)).toMatch(/\b1\b/);
});

test("what is open travels in the view", async ({ page }) => {
  await press(page, 'td[data-row="2"][data-col="0"]', "Enter");
  await expect.poll(() => names(page)).toContain("North");
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  expect(view.expanded).toEqual([[1]]);

  await page.reload();
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await settled(page);
  await page.evaluate(
    (view) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), view),
    view,
  );
  await expect.poll(() => names(page)).toEqual(["Orphan", "Partners", "Sales", "North", "South"]);
});

test("page-size refuses a tree, group-by is not used in one", async ({ page }) => {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "region"),
  );
  await settled(page);
  // `group-by` is a regroup: it says why it is not used, and the tree stays.
  await expect.poll(() => status(page)).toContain("group-by is not used in a tree");
  expect(await names(page)).toEqual(["Orphan", "Partners", "Sales"]);

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("page-size", "5"),
  );
  await expect.poll(() => status(page)).toContain("no tree with page-size");
});

test("the tree passes axe, open and closed", async ({ page }) => {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await press(page, 'td[data-row="2"][data-col="0"]', "ArrowRight");
  await expect.poll(() => names(page)).toContain("North");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
