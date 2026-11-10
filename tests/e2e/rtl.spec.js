import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Right to left (issue #320): one page per element, each with `dir="rtl"` on
// the document and nothing on the element — the direction is the platform's.
//
// What is under test is what the elements do themselves: the sheets use the
// logical sides, `←` and `→` follow what the reader sees, and the grid's own
// arithmetic holds with the negative `scrollLeft` of a right-to-left scroller.
// How the browser orders the letters inside a cell is not.

/** Runs `fn(root, arg)` in the shadow root of the element `selector` names. */
const inside = (page, selector, fn, arg) =>
  page.evaluate(
    ({ selector, fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(document.querySelector(selector).shadowRoot, arg),
    { selector, fn: fn.toString(), arg },
  );

const focusIn = (page, selector, target) => inside(page, selector, (root, target) => root.querySelector(target).focus(), target);

/** The focused cell of a grid: its column, its row, and the first cell's text. */
const focused = (page, selector) =>
  inside(page, selector, (root) => {
    const cell = root.activeElement;
    return cell && { col: cell.dataset.col ?? "select", row: cell.dataset.row ?? null, text: cell.textContent.trim() };
  });

const status = (page, selector) => inside(page, selector, (root) => root.querySelector('[part="status"]').textContent);

const settled = (page, selector) =>
  page.waitForFunction((selector) => {
    const root = document.querySelector(selector)?.shadowRoot;
    const line = root?.querySelector('[part="status"]');
    return !!line && line.getAttribute("data-state") !== "loading" && !!root.querySelector("td[data-row]");
  }, selector);

test.describe("grid, right to left", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/e2e/fixtures/grid-rtl.html");
    await page.waitForFunction(() => window.__opengridReady);
    for (const selector of ["#flat", "#grouped", "#tree"]) await settled(page, selector);
  });

  /** Where the columns, their filter groups and the first row's cells stand. */
  const layout = (page) =>
    inside(page, "#flat", (root) => {
      const box = (element) => {
        const rect = element.getBoundingClientRect();
        return [Math.round(rect.left), Math.round(rect.right)];
      };
      const viewport = root.querySelector('[part~="viewport"]');
      const row = root.querySelector('td[data-row="0"]').closest("tr");
      return {
        viewport: box(viewport),
        scroll: viewport.scrollLeft,
        filterScroll: root.querySelector('[part="filter"]').scrollLeft,
        select: box(root.querySelector("th[data-select]")),
        columns: [...root.querySelectorAll("th[data-col]")].map((th) => ({
          header: box(th),
          cell: box(row.querySelector(`td[data-col="${th.dataset.col}"]`)),
          group: box(root.querySelector(`[part="filter-operator"][data-col="${th.dataset.col}"]`).parentElement),
        })),
      };
    });

  const near = (a, b) => Math.abs(a[0] - b[0]) <= 1 && Math.abs(a[1] - b[1]) <= 1;
  const aligned = ({ columns }) => columns.every(({ header, cell, group }) => near(header, cell) && near(header, group));

  test("the columns run from the right, and rows and filter fields stand under them", async ({ page }) => {
    await expect.poll(async () => aligned(await layout(page))).toBe(true);
    const { viewport, select, columns, scroll } = await layout(page);
    expect(scroll).toBe(0);
    // The row starts at the right edge: the selection column, then `id`, and
    // every further column to the left of the one before.
    expect(select[1]).toBe(viewport[1]);
    expect(columns[0].header[1]).toBe(select[0]);
    for (let col = 1; col < columns.length; col += 1) {
      expect(columns[col].header[1]).toBe(columns[col - 1].header[0]);
    }
    // Six columns of 150 px do not fit: the rest is out of sight on the left.
    expect(columns.at(-1).header[0]).toBeLessThan(viewport[0]);
  });

  test("start and end change sides", async ({ page }) => {
    const align = await inside(page, "#flat", (root) => {
      const of = (selector) => getComputedStyle(root.querySelector(selector)).textAlign;
      // `customer` is text, `amount` a number; a header stands as its column does.
      return { text: of('td[data-row="0"][data-col="1"]'), number: of('td[data-row="0"][data-col="3"]'), header: of('th[data-col="1"]') };
    });
    expect(align).toEqual({ text: "right", number: "left", header: "right" });
  });

  test("← goes to the next cell and → to the one before; Home and End keep the row's ends", async ({ page }) => {
    await focusIn(page, "#flat", 'td[data-row="0"][data-col="0"]');
    await page.keyboard.press("ArrowLeft");
    expect(await focused(page, "#flat")).toMatchObject({ col: "1", row: "0" });
    await page.keyboard.press("ArrowLeft");
    expect(await focused(page, "#flat")).toMatchObject({ col: "2", row: "0" });
    await page.keyboard.press("ArrowRight");
    expect(await focused(page, "#flat")).toMatchObject({ col: "1", row: "0" });
    // `Home` is the start of the row — the selection cell, on the right here —
    // and `End` its last cell, on the left: named by reading order, not by side.
    await page.keyboard.press("End");
    expect(await focused(page, "#flat")).toMatchObject({ col: "5", row: "0" });
    await page.keyboard.press("Home");
    expect(await focused(page, "#flat")).toMatchObject({ col: "select" });
    // From the start of the row `→` has nowhere to go, `←` has.
    await page.keyboard.press("ArrowRight");
    expect(await focused(page, "#flat")).toMatchObject({ col: "select" });
    await page.keyboard.press("ArrowLeft");
    expect(await focused(page, "#flat")).toMatchObject({ col: "0", row: "0" });
    // The vertical keys are the same in both directions.
    await page.keyboard.press("ArrowDown");
    expect(await focused(page, "#flat")).toMatchObject({ col: "0", row: "1" });
  });

  test("the focused cell is in sight at both ends of a row that scrolls sideways", async ({ page }) => {
    const sight = () =>
      inside(page, "#flat", (root) => {
        const viewport = root.querySelector('[part~="viewport"]');
        const frame = viewport.getBoundingClientRect();
        const cell = root.activeElement.getBoundingClientRect();
        return {
          inSight: cell.left >= frame.left - 1 && cell.right <= frame.right + 1,
          scroll: viewport.scrollLeft,
          filterScroll: root.querySelector('[part="filter"]').scrollLeft,
        };
      });
    await focusIn(page, "#flat", 'td[data-row="0"][data-col="0"]');
    await page.keyboard.press("End");
    // A right-to-left scroller counts from 0 into the negative.
    await expect.poll(async () => (await sight()).scroll).toBeLessThan(0);
    expect((await sight()).inSight).toBe(true);
    // The filter row keeps the rows' position, sign and all.
    await expect.poll(async () => (await sight()).filterScroll).toBe((await sight()).scroll);

    for (let step = 0; step < 5; step += 1) {
      await page.keyboard.press("ArrowRight");
      expect((await sight()).inSight).toBe(true);
    }
    expect(await focused(page, "#flat")).toMatchObject({ col: "0", row: "0" });
    await page.keyboard.press("Home");
    await expect.poll(async () => (await sight()).scroll).toBe(0);
    expect((await sight()).inSight).toBe(true);
  });

  test("the filter row is laid out again while scrolled, and still stands under the columns", async ({ page }) => {
    await expect.poll(async () => aligned(await layout(page))).toBe(true);
    // A header out of sight on the left: the focus scrolls to it.
    await focusIn(page, "#flat", 'th[data-col="4"]');
    await expect.poll(async () => (await layout(page)).scroll).toBeLessThan(0);
    const before = await layout(page);
    await expect.poll(async () => (await layout(page)).filterScroll).toBe(before.scroll);
    // Its width changes: the indent is measured anew, from a scrolled table.
    await page.keyboard.press("Control+Shift+ArrowLeft");
    await expect
      .poll(async () => {
        const { header } = (await layout(page)).columns[4];
        return header[1] - header[0];
      })
      .toBeGreaterThan(before.columns[4].header[1] - before.columns[4].header[0]);
    await expect.poll(async () => aligned(await layout(page))).toBe(true);
    expect((await layout(page)).scroll).toBeLessThan(0);
  });

  test("a column moves and is resized by the side the key points to", async ({ page }) => {
    const headers = () =>
      inside(page, "#flat", (root) => [...root.querySelectorAll("th[data-col]")].map((th) => th.querySelector("span").textContent));
    const width = (col) =>
      inside(page, "#flat", (root, col) => root.querySelector(`th[data-col="${col}"]`).getBoundingClientRect().width, col);
    const before = await headers();

    // `id` is the first column, on the right: there is nothing further right.
    await focusIn(page, "#flat", 'th[data-col="0"]');
    await page.keyboard.press("Control+ArrowRight");
    expect(await headers()).toEqual(before);
    expect(await status(page, "#flat")).toContain("already at the end");

    // To the left is the next place.
    await page.keyboard.press("Control+ArrowLeft");
    await expect.poll(headers).toEqual([before[1], before[0], ...before.slice(2)]);
    await expect.poll(() => status(page, "#flat")).toContain("position 2");
    expect(await focused(page, "#flat")).toMatchObject({ col: "1" });

    // A column's far edge is its left one: `←` pushes it out, `→` pulls it in.
    const start = await width(1);
    await page.keyboard.press("Control+Shift+ArrowLeft");
    await expect.poll(() => width(1)).toBeGreaterThan(start);
    const wide = await width(1);
    await page.keyboard.press("Control+Shift+ArrowRight");
    await expect.poll(() => width(1)).toBeLessThan(wide);
  });

  test("the column menu opens from its header's right edge", async ({ page }) => {
    await focusIn(page, "#flat", 'th[data-col="1"]');
    await page.keyboard.press("Alt+ArrowDown");
    const edges = await inside(page, "#flat", (root) => ({
      menu: Math.round(root.querySelector('[part="column-menu"]').getBoundingClientRect().right),
      header: Math.round(root.querySelector('th[data-col="1"]').getBoundingClientRect().right),
    }));
    expect(edges.menu).toBe(edges.header);
  });

  test("a selected row's bar stands at the row's start, on the right", async ({ page }) => {
    await focusIn(page, "#flat", 'td[data-row="0"][data-col="0"]');
    await page.keyboard.press("Space");
    const shadow = await inside(page, "#flat", (root) =>
      getComputedStyle(root.querySelector('tr[data-selected="true"]')).boxShadow,
    );
    expect(shadow).toContain("-3px 0px 0px 0px inset");
  });

  test("← opens a group and → closes it, and the closed mark points left", async ({ page }) => {
    const rowcount = () => inside(page, "#grouped", (root) => Number(root.querySelector("table").getAttribute("aria-rowcount")));
    const mark = () =>
      inside(page, "#grouped", (root) => getComputedStyle(root.querySelector('td[data-row="0"][data-col="0"]'), "::before").transform);
    // The header row, five groups and the total.
    await expect.poll(rowcount).toBe(7);
    expect(await mark()).toBe("matrix(-1, 0, 0, 1, 0, 0)");

    await focusIn(page, "#grouped", 'td[data-row="0"][data-col="0"]');
    // `→` on a closed group opens nothing: it is the way back, and there is none.
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(150);
    expect(await rowcount()).toBe(7);

    await page.keyboard.press("ArrowLeft");
    await expect.poll(rowcount).toBe(7 + 52);
    await page.keyboard.press("ArrowRight");
    await expect.poll(rowcount).toBe(7);
  });

  test("← opens a node and goes into it, → climbs and closes", async ({ page }) => {
    const names = () =>
      inside(page, "#tree", (root) =>
        [...root.querySelectorAll("tbody tr")]
          .filter((tr) => tr.style.transform && !tr.style.display)
          .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
          .map((tr) => tr.querySelector('td[data-col="0"]').textContent),
      );
    const row = async () => (await focused(page, "#tree")).text;
    await expect.poll(names).toEqual(["Orphan", "Partners", "Sales"]);

    await focusIn(page, "#tree", 'td[data-row="2"][data-col="0"]');
    await page.keyboard.press("ArrowLeft");
    await expect.poll(names).toEqual(["Orphan", "Partners", "Sales", "North", "South"]);
    expect(await row()).toBe("Sales");
    // `←` on the open node: its first child.
    await page.keyboard.press("ArrowLeft");
    await expect.poll(row).toBe("North");
    // `→` on a closed node: its parent. `→` on the open parent: it closes.
    await page.keyboard.press("ArrowRight");
    await expect.poll(row).toBe("Sales");
    await page.keyboard.press("ArrowRight");
    await expect.poll(names).toEqual(["Orphan", "Partners", "Sales"]);
  });

  test("a direction that changes is followed by the next key", async ({ page }) => {
    await focusIn(page, "#flat", 'td[data-row="0"][data-col="1"]');
    await page.keyboard.press("ArrowLeft");
    expect(await focused(page, "#flat")).toMatchObject({ col: "2" });
    await page.evaluate(() => document.documentElement.setAttribute("dir", "ltr"));
    await page.keyboard.press("ArrowLeft");
    expect(await focused(page, "#flat")).toMatchObject({ col: "1" });
    // On the element itself it counts as much as on the page.
    await page.evaluate(() => document.querySelector("#flat").setAttribute("dir", "rtl"));
    await page.keyboard.press("ArrowLeft");
    expect(await focused(page, "#flat")).toMatchObject({ col: "2" });
  });

  test("has no axe violations", async ({ page }) => {
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });

  test("matches the visual baseline", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "Screenshot baselines are Chromium's (tests/e2e/playwright.config.js)");
    await expect.poll(async () => aligned(await layout(page))).toBe(true);
    await expect(page).toHaveScreenshot("grid-rtl.png", { fullPage: true, maxDiffPixelRatio: 0.02 });
  });
});

test.describe("table, right to left", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/e2e/fixtures/table-rtl.html");
    await page.evaluate(() => window.__opengridReady);
    await expect(page.locator("opengrid-table tbody tr").first()).toBeVisible();
  });

  test("the columns run from the right, and start and end change sides", async ({ page }) => {
    const facts = await inside(page, "opengrid-table", (root) => {
      const first = root.querySelector("tbody tr");
      return {
        // Left edges, in the order of the columns: each further left.
        lefts: [...root.querySelectorAll("thead th")].map((th) => Math.round(th.getBoundingClientRect().left)),
        customer: getComputedStyle(first.children[0]).textAlign,
        amount: getComputedStyle(first.children[1]).textAlign,
      };
    });
    expect(facts.lefts[0]).toBeGreaterThan(facts.lefts[1]);
    expect(facts.lefts[1]).toBeGreaterThan(facts.lefts[2]);
    expect(facts).toMatchObject({ customer: "right", amount: "left" });
  });

  test("has no axe violations", async ({ page }) => {
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });

  test("matches the visual baseline", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "Screenshot baselines are Chromium's (tests/e2e/playwright.config.js)");
    await expect(page).toHaveScreenshot("table-rtl.png", { maxDiffPixelRatio: 0.02 });
  });
});

test.describe("pivot, right to left", () => {
  const state = (page) => inside(page, "opengrid-pivot", (root) => root.querySelector('[part~="status"]')?.dataset.state);
  const chips = (page) =>
    inside(page, "opengrid-pivot", (root) =>
      [...root.querySelectorAll('[part="chip"][data-field]')]
        .filter((chip) => chip.querySelector("[data-move]")?.dataset.move.startsWith("rows|"))
        .map((chip) => chip.dataset.field),
    );

  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/e2e/fixtures/pivot-rtl.html");
    await page.waitForFunction(() => window.__ready);
    await expect(page.locator("opengrid-pivot tbody tr").first()).toBeVisible();
  });

  test("the row headers stand on the right and the values to their left", async ({ page }) => {
    const facts = await inside(page, "opengrid-pivot", (root) => {
      const row = root.querySelector("tbody tr");
      const header = row.querySelector("th").getBoundingClientRect();
      const value = row.querySelector("td").getBoundingClientRect();
      return { header: Math.round(header.left), value: Math.round(value.right) };
    });
    expect(facts.value).toBeLessThanOrEqual(facts.header);
  });

  test("what points sideways points the other way: a fold mark and the move buttons", async ({ page }) => {
    const flipped = "matrix(-1, 0, 0, 1, 0, 0)";
    const facts = await inside(page, "opengrid-pivot", (root) => {
      const later = root.querySelector('button[aria-label="Move zone later"]');
      const name = later.previousElementSibling;
      return {
        mark: getComputedStyle(root.querySelector('[part~="group-mark"]')).transform,
        move: getComputedStyle(later).transform,
        // "Later" is the next place in reading order: left of the field's name.
        laterIsLeft: later.getBoundingClientRect().right <= name.getBoundingClientRect().left + 1,
      };
    });
    expect(facts).toEqual({ mark: flipped, move: flipped, laterIsLeft: true });
  });

  test("a field still moves by its button, by reading order", async ({ page }) => {
    expect(await chips(page)).toEqual(["zone", "kind"]);
    await focusIn(page, "opengrid-pivot", 'button[aria-label="Move zone later"]');
    await page.keyboard.press("Enter");
    await expect.poll(() => chips(page)).toEqual(["kind", "zone"]);
  });

  test("has no axe violations", async ({ page }) => {
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });

  test("matches the visual baseline", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "Screenshot baselines are Chromium's (tests/e2e/playwright.config.js)");
    await expect.poll(() => state(page)).toBe("ready");
    await expect(page).toHaveScreenshot("pivot-rtl.png", { maxDiffPixelRatio: 0.02 });
  });
});
