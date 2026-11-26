import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// Issue #524: editing and selecting by pointer and touch. A double-click, a
// double-tap and a tap on the focused cell open the editor; a long-press, a
// right click, `Shift`+`F10` and the context-menu key open the cell menu, which
// lists what applies to the cell and then the page's own items. A swipe
// scrolls; a long-press and a drag mark a range.

async function open(page, search = "") {
  await page.goto(`/tests/e2e/fixtures/grid-touch.html${search}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

/** The middle of a data cell, in page coordinates. */
const centre = (page, row, col) =>
  shadow(
    page,
    (root, { row, col }) => {
      const rect = root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).getBoundingClientRect();
      return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    },
    { row, col },
  );

/** What has the focus inside the grid: the editor, a cell, a menu entry. */
const focused = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    if (!active) return null;
    if (active.closest('[part="editor"]') || active.matches("input, textarea, select"))
      return { editor: true, row: active.closest("td")?.getAttribute("data-row") };
    if (active.matches("td"))
      return { cell: [Number(active.getAttribute("data-row")), Number(active.getAttribute("data-col"))] };
    return { other: active.getAttribute("role") ?? active.tagName, text: active.textContent };
  });

/** The cell menu, if one is open: its name and its entries' labels. */
const menu = (page) =>
  shadow(page, (root) => {
    const node = root.querySelector('[part="cell-menu"]');
    if (!node || !node.matches(":popover-open")) return null;
    return {
      label: node.getAttribute("aria-label"),
      items: [...node.querySelectorAll('[role^="menuitem"], [role="separator"]')].map((item) =>
        item.getAttribute("role") === "separator" ? "—" : item.textContent,
      ),
    };
  });

async function pick(page, label) {
  await shadow(
    page,
    (root, label) =>
      [...root.querySelectorAll('[part="cell-menu"] [role^="menuitem"]')]
        .find((item) => item.textContent === label)
        .click(),
    label,
  );
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), {
    row,
    col,
  });
}

test.describe("mouse and keyboard", () => {
  test("a double-click opens the editor, and selects no text", async ({ page }) => {
    await open(page);
    const { x, y } = await centre(page, 2, 1);
    await page.mouse.dblclick(x, y);
    await expect.poll(() => focused(page)).toMatchObject({ editor: true, row: "2" });
    expect(await page.evaluate(() => String(window.getSelection()))).toBe("");
  });

  test("a double-click on the read-only key opens nothing", async ({ page }) => {
    await open(page);
    const { x, y } = await centre(page, 2, 0);
    await page.mouse.dblclick(x, y);
    await expect.poll(() => focused(page)).toEqual({ cell: [2, 0] });
  });

  test("a single click on the focused cell does not edit with a mouse", async ({ page }) => {
    await open(page);
    const { x, y } = await centre(page, 1, 1);
    await page.mouse.click(x, y);
    await page.waitForTimeout(400);
    await page.mouse.click(x, y);
    await expect.poll(() => focused(page)).toEqual({ cell: [1, 1] });
  });

  test("a right click opens the cell menu: the grid's entries, then the page's for that column", async ({
    page,
  }) => {
    await open(page);
    const { x, y } = await centre(page, 3, 1);
    await page.mouse.click(x, y, { button: "right" });
    await expect.poll(() => menu(page)).toEqual({
      label: "name cell menu",
      items: ["Edit", "Copy", "Select row", "Open record", "Column menu", "—", "Book this room", "Suggest a name"],
    });
    expect(await focused(page)).toMatchObject({ other: "menuitem", text: "Edit" });

    // The key column: no Edit, and the item for `name` is not offered.
    await page.keyboard.press("Escape");
    await expect.poll(() => focused(page)).toEqual({ cell: [3, 1] });
    const key = await centre(page, 3, 0);
    await page.mouse.click(key.x, key.y, { button: "right" });
    await expect.poll(() => menu(page)).toEqual({
      label: "id cell menu",
      items: ["Copy", "Select row", "Open record", "Column menu", "—", "Book this room"],
    });
  });

  test("Shift+F10 and the context-menu key open it; Escape gives the focus back", async ({ page }) => {
    await open(page);
    await focusCell(page, 4, 2);
    await page.keyboard.press("Shift+F10");
    await expect.poll(() => menu(page)).toMatchObject({ label: "seats cell menu" });
    await page.keyboard.press("ArrowDown");
    expect(await focused(page)).toMatchObject({ text: "Copy" });
    await page.keyboard.press("Escape");
    await expect.poll(() => menu(page)).toBeNull();
    expect(await focused(page)).toEqual({ cell: [4, 2] });

    await page.keyboard.press("ContextMenu");
    await expect.poll(() => menu(page)).toMatchObject({ label: "seats cell menu" });
    await page.keyboard.press("Enter");
    await expect.poll(() => focused(page)).toMatchObject({ editor: true, row: "4" });
  });

  test("the entries do what they say", async ({ page }) => {
    await open(page);
    await focusCell(page, 5, 1);
    await page.keyboard.press("Shift+F10");
    await pick(page, "Select row");
    await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.keys)).toEqual([6]);
    await page.keyboard.press("Shift+F10");
    expect((await menu(page)).items).toContain("Deselect row");
    await pick(page, "Book this room");
    expect(await page.evaluate(() => window.__actions)).toEqual([{ id: "book", row: 5, column: "name", key: 6 }]);
    expect(await focused(page)).toEqual({ cell: [5, 1] });

    await page.keyboard.press("Shift+F10");
    await pick(page, "Copy");
    await expect.poll(() => status(page)).toMatch(/Copied/);

    await page.keyboard.press("Shift+F10");
    await pick(page, "Column menu");
    await expect
      .poll(() => shadow(page, (root) => root.querySelector('[part="column-menu"]')?.matches(":popover-open")))
      .toBe(true);
  });

  test("under edit-locked no way in edits", async ({ page }) => {
    await open(page, "?locked");
    const { x, y } = await centre(page, 2, 1);
    await page.mouse.dblclick(x, y);
    await expect.poll(() => focused(page)).toEqual({ cell: [2, 1] });
    await page.keyboard.press("Shift+F10");
    await expect.poll(() => menu(page)).toMatchObject({
      items: ["Copy", "Select row", "Column menu", "—", "Book this room", "Suggest a name"],
    });
  });

  test("has no axe violations with the cell menu open", async ({ page }) => {
    await open(page);
    await focusCell(page, 1, 1);
    await page.keyboard.press("Shift+F10");
    await expect.poll(() => menu(page)).not.toBeNull();
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });
});

test.describe("touch", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 760 } });

  /** Touches a point for `ms` milliseconds, then lifts — through the DevTools protocol. */
  async function press(page, { x, y }, ms, moveTo) {
    const cdp = await page.context().newCDPSession(page);
    const point = (x, y) => [{ x, y, id: 1, radiusX: 1, radiusY: 1, force: 1 }];
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: point(x, y) });
    await page.waitForTimeout(ms);
    if (moveTo) {
      const steps = 8;
      for (let step = 1; step <= steps; step += 1) {
        await cdp.send("Input.dispatchTouchEvent", {
          type: "touchMove",
          touchPoints: point(x + ((moveTo.x - x) * step) / steps, y + ((moveTo.y - y) * step) / steps),
        });
        await page.waitForTimeout(16);
      }
    }
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await cdp.detach();
  }

  test("rows are 44 px tall for a finger", async ({ page }) => {
    await open(page);
    const height = await shadow(page, (root) => root.querySelector("tbody tr[aria-rowindex]").getBoundingClientRect().height);
    expect(height).toBeGreaterThanOrEqual(44);
  });

  test("a tap focuses; a tap on the focused cell opens the editor with the focus in it", async ({ page }) => {
    await open(page);
    const point = await centre(page, 2, 1);
    await page.touchscreen.tap(point.x, point.y);
    await expect.poll(() => focused(page)).toEqual({ cell: [2, 1] });
    await page.waitForTimeout(400);
    await page.touchscreen.tap(point.x, point.y);
    await expect.poll(() => focused(page)).toMatchObject({ editor: true, row: "2" });
  });

  test("a tap on another cell only moves the focus", async ({ page }) => {
    await open(page);
    const first = await centre(page, 2, 1);
    const second = await centre(page, 3, 1);
    await page.touchscreen.tap(first.x, first.y);
    await page.waitForTimeout(400);
    await page.touchscreen.tap(second.x, second.y);
    await expect.poll(() => focused(page)).toEqual({ cell: [3, 1] });
  });

  test("a double-tap opens the editor", async ({ page }) => {
    await open(page);
    const point = await centre(page, 4, 1);
    await page.touchscreen.tap(point.x, point.y);
    await page.touchscreen.tap(point.x, point.y);
    await expect.poll(() => focused(page)).toMatchObject({ editor: true, row: "4" });
  });

  test("a long-press opens the cell menu on that cell", async ({ page }) => {
    await open(page);
    await press(page, await centre(page, 3, 1), 800);
    await expect.poll(() => menu(page)).toMatchObject({ label: "name cell menu" });
    expect(await page.evaluate(() => window.__changes)).toEqual([]);
    await pick(page, "Edit");
    await expect.poll(() => focused(page)).toMatchObject({ editor: true, row: "3" });
  });

  test("on a short screen the menu stays on it, and scrolls", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 560 });
    await open(page);
    await press(page, await centre(page, 2, 1), 800);
    await expect.poll(() => menu(page)).not.toBeNull();
    const box = await shadow(page, (root) => {
      const node = root.querySelector('[part="cell-menu"]');
      const rect = node.getBoundingClientRect();
      return { top: rect.top, bottom: rect.bottom, scrolls: node.scrollHeight > node.clientHeight };
    });
    expect(box.top).toBeGreaterThanOrEqual(0);
    expect(box.bottom).toBeLessThanOrEqual(560);
    expect(box.scrolls).toBe(true);
  });

  test("a swipe scrolls, and marks nothing and opens nothing", async ({ page }) => {
    await open(page);
    const from = await centre(page, 6, 1);
    await press(page, from, 0, { x: from.x, y: from.y - 220 });
    await expect
      .poll(() => shadow(page, (root) => root.querySelector('[part="viewport"]').scrollTop))
      .toBeGreaterThan(50);
    expect(await menu(page)).toBeNull();
    expect(await shadow(page, (root) => root.querySelectorAll("td[data-range]").length)).toBe(0);
  });

  test("a long-press and a drag mark a range instead of scrolling", async ({ page }) => {
    await open(page);
    const from = await centre(page, 1, 1);
    const to = await centre(page, 3, 2);
    await press(page, from, 800, to);
    await expect
      .poll(() => shadow(page, (root) => root.querySelectorAll("td[data-range]").length))
      .toBe(6);
    expect(await menu(page)).toBeNull();
    expect(await shadow(page, (root) => root.querySelector('[part="viewport"]').scrollTop)).toBe(0);
  });
});
