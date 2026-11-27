import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// The page's own items in the toolbar (issue #531): `set_toolbar` draws buttons,
// menu buttons and disabled items after the grid's own controls, keyed like
// them; the `toolbar-end` slot takes a control the grid cannot know and leaves
// it its own keys. `opengrid-toolbar-action` names the item and carries the
// selection.

async function open(page, width = 1280) {
  await page.setViewportSize({ width, height: 800 });
  await page.goto("/tests/e2e/fixtures/grid-toolbar-items.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

const item = (page, id) => page.locator(`opengrid-grid [data-toolbar-item="${id}"]`);
const events = (page) => page.evaluate(() => window.__toolbar);
const press = (page, key) => page.keyboard.press(key);
const focusItem = (page, id) =>
  shadow(page, (root, id) => root.querySelector(`[data-toolbar-item="${id}"]`).focus(), id);
const focused = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active?.getAttribute("data-toolbar-item") ?? active?.textContent.trim() ?? null;
  });
const menuOpen = (page) =>
  shadow(page, (root) => !!root.querySelector('[part="toolbar-menu"]:popover-open'));
const select = (page, ...rows) =>
  shadow(
    page,
    (root, rows) => {
      const cells = root.querySelectorAll('td[data-select="row"]');
      for (const row of rows) cells[row].click();
    },
    rows,
  );

test("the page's items stand after the grid's own controls, then the slot", async ({ page }) => {
  await open(page);
  const row = await shadow(page, (root) => {
    const row = root.querySelector("[data-toolbar-row]");
    const slot = row.querySelector("slot");
    return {
      children: [...row.children].map((node) => node.getAttribute("part") ?? node.tagName.toLowerCase()),
      slotName: slot.getAttribute("name"),
      slotted: slot.assignedElements().map((node) => node.querySelector("input").id),
      insideGrid: !!root.querySelector("table").contains(slot),
      toolbarInsideGrid: !!root.querySelector("table").contains(root.querySelector('[part="toolbar"]')),
    };
  });
  // The grid's own, the density last of them, the page's three, the slot.
  expect(row.children.slice(-5)).toEqual(["density", "toolbar-item", "toolbar-item", "toolbar-item", "slot"]);
  expect(row.slotName).toBe("toolbar-end");
  expect(row.slotted).toEqual(["reference"]);
  expect(row.insideGrid).toBe(false);
  expect(row.toolbarInsideGrid).toBe(false);

  const items = await shadow(page, (root) =>
    [...root.querySelectorAll('[part="toolbar-item"]')].map((button) => ({
      text: button.textContent,
      popup: button.getAttribute("aria-haspopup"),
      expanded: button.getAttribute("aria-expanded"),
      disabled: button.getAttribute("aria-disabled"),
      name: button.getAttribute("aria-label"),
    })),
  );
  expect(items).toEqual([
    { text: "Prepare price change", popup: null, expanded: null, disabled: null, name: null },
    { text: "Export", popup: "menu", expanded: "false", disabled: null, name: null },
    { text: "Bulk change", popup: null, expanded: null, disabled: "true", name: "Bulk change. Select rows first" },
  ]);
});

test("Tab goes from the grid's last control through the page's items to the slotted field", async ({ page, browserName }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('[data-density="comfortable"]').focus());
  const order = [];
  for (let step = 0; step < 4; step += 1) {
    await press(page, browserName === "webkit" ? "Alt+Tab" : "Tab");
    order.push(
      await page.evaluate(() => {
        const active = document.activeElement;
        return active.id || (active.shadowRoot?.activeElement?.getAttribute("data-toolbar-item") ?? null);
      }),
    );
  }
  expect(order).toEqual(["prepare", "export", "bulk", "reference"]);
});

test("a button fires its id and the selection — by pointer, Enter and Space", async ({ page }) => {
  await open(page);
  // Nothing selected: the keys are empty, not missing.
  await item(page, "prepare").click();
  expect(await events(page)).toEqual([{ id: "prepare", selection: { count: 0, keys: [] } }]);

  // Rows 1 and 2 of the grid are the records 2 and 3: the selection is by key.
  await select(page, 1, 2);
  await expect.poll(() => shadow(page, (root) => root.querySelectorAll('tr[aria-selected="true"]').length)).toBe(2);
  await focusItem(page, "prepare");
  await press(page, "Enter");
  await press(page, " ");
  const fired = await events(page);
  expect(fired).toHaveLength(3);
  for (const detail of fired.slice(1)) {
    expect(detail.id).toBe("prepare");
    expect(detail.selection.count).toBe(2);
    expect([...detail.selection.keys].sort()).toEqual([2, 3]);
    expect("rows" in detail.selection).toBe(false);
  }
});

test("a menu button opens with the arrows and has the column menu's keys", async ({ page }) => {
  await open(page);
  await focusItem(page, "export");
  await press(page, "ArrowDown");
  expect(await menuOpen(page)).toBe(true);
  expect(
    await shadow(page, (root) => {
      const menu = root.querySelector('[part="toolbar-menu"]');
      return {
        role: menu.getAttribute("role"),
        name: menu.getAttribute("aria-label"),
        entries: [...menu.querySelectorAll('[role="menuitem"]')].map((entry) => entry.textContent),
        expanded: root.querySelector('[data-toolbar-item="export"]').getAttribute("aria-expanded"),
        outside: !root.querySelector("table").contains(menu),
      };
    }),
  ).toEqual({ role: "menu", name: "Export", entries: ["PDF", "Excel", "CSV"], expanded: "true", outside: true });
  expect(await focused(page)).toBe("PDF");

  await press(page, "ArrowDown");
  expect(await focused(page)).toBe("Excel");
  await press(page, "End");
  expect(await focused(page)).toBe("CSV");
  await press(page, "Home");
  expect(await focused(page)).toBe("PDF");

  // Escape: back to the button, nothing chosen.
  await press(page, "Escape");
  expect(await menuOpen(page)).toBe(false);
  expect(await focused(page)).toBe("export");
  expect(
    await shadow(page, (root) => root.querySelector('[data-toolbar-item="export"]').getAttribute("aria-expanded")),
  ).toBe("false");
  expect(await events(page)).toEqual([]);

  // ArrowUp opens on the last entry; Enter on an entry fires its own id.
  await press(page, "ArrowUp");
  expect(await focused(page)).toBe("CSV");
  await press(page, "ArrowUp");
  await press(page, "Enter");
  expect(await menuOpen(page)).toBe(false);
  expect(await focused(page)).toBe("export");
  expect(await events(page)).toEqual([{ id: "export-xlsx", selection: { count: 0, keys: [] } }]);

  // Enter on the button opens it, Space on an entry chooses; Tab closes.
  await press(page, "Enter");
  expect(await menuOpen(page)).toBe(true);
  await press(page, " ");
  expect((await events(page)).at(-1).id).toBe("export-pdf");
  await press(page, "Enter");
  await press(page, "Tab");
  expect(await menuOpen(page)).toBe(false);
});

test("a menu button and its entries work by pointer", async ({ page }) => {
  await open(page);
  await item(page, "export").click();
  expect(await menuOpen(page)).toBe(true);
  await page.locator('opengrid-grid [part="toolbar-menu"] [role="menuitem"]', { hasText: "CSV" }).click();
  expect(await events(page)).toEqual([{ id: "export-csv", selection: { count: 0, keys: [] } }]);
  expect(await menuOpen(page)).toBe(false);

  // A click outside closes it, and the button says so.
  await item(page, "export").click();
  expect(await menuOpen(page)).toBe(true);
  await page.locator("h1").click();
  await expect.poll(() => menuOpen(page)).toBe(false);
  await expect
    .poll(() =>
      shadow(page, (root) => root.querySelector('[data-toolbar-item="export"]').getAttribute("aria-expanded")),
    )
    .toBe("false");
  expect(await events(page)).toHaveLength(1);
});

test("a disabled item says why on focus and on activation, and fires nothing", async ({ page }) => {
  await open(page);
  await focusItem(page, "bulk");
  expect(
    await shadow(page, (root) => root.activeElement.getAttribute("aria-label")),
  ).toBe("Bulk change. Select rows first");
  await press(page, "Enter");
  await expect.poll(() => status(page)).toContain("Select rows first");
  // Playwright will not click what says it is disabled; a reader can.
  await item(page, "bulk").click({ force: true });
  expect(await events(page)).toEqual([]);
});

test("a call with a problem applies none of it, and says every problem", async ({ page }) => {
  await open(page);
  await page.evaluate(() =>
    window.__module.set_toolbar(document.querySelector("opengrid-grid"), [
      { id: "a", label: "A" },
      { id: "a", label: "" },
      { id: "b", label: "B", position: "end" },
      { id: "c", label: "C", disabled: true },
      { id: "d", label: "D", items: [{ id: "a", label: "Again" }] },
    ]),
  );
  const said = await status(page);
  expect(said).toContain("Toolbar items not applied.");
  for (const problem of [
    "a: id is used twice",
    "a: has an empty label",
    'b: unknown key "position"',
    "c: is disabled without a reason",
  ]) {
    expect(said).toContain(problem);
  }
  // The items of the call before stand.
  expect(
    await shadow(page, (root) => [...root.querySelectorAll('[part="toolbar-item"]')].map((button) => button.textContent)),
  ).toEqual(["Prepare price change", "Export", "Bulk change"]);
});

test("a valid call replaces the items, keeps the focus on an item it still has, and [] takes them away", async ({ page }) => {
  await open(page);
  await focusItem(page, "prepare");
  await page.evaluate(() =>
    window.__module.set_toolbar(document.querySelector("opengrid-grid"), [
      { id: "prepare", label: "Prepare", disabled: true, reason: "Nothing to prepare" },
      { id: "again", label: "Again" },
    ]),
  );
  expect(
    await shadow(page, (root) => [...root.querySelectorAll('[part="toolbar-item"]')].map((button) => button.textContent)),
  ).toEqual(["Prepare", "Again"]);
  expect(await focused(page)).toBe("prepare");

  await page.evaluate(() => window.__module.set_toolbar(document.querySelector("opengrid-grid"), []));
  expect(await shadow(page, (root) => root.querySelectorAll('[part="toolbar-item"]').length)).toBe(0);
  // The slot stays; it is the page's.
  expect(await shadow(page, (root) => !!root.querySelector('slot[name="toolbar-end"]'))).toBe(true);
});

test("the page's items and the slot show only with the toolbar attribute", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("toolbar"));
  await expect
    .poll(() => shadow(page, (root) => root.querySelectorAll('[part="toolbar-item"], slot').length))
    .toBe(0);
  // Nothing is drawn for the slotted field either: it has no slot to stand in.
  expect(await page.locator("#reference").isVisible()).toBe(false);
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("toolbar", ""));
  await expect(item(page, "prepare")).toBeVisible();
  await expect(page.locator("#reference")).toBeVisible();
});

test("a slotted date field keeps its own keys; the grid's never reach it", async ({ page }) => {
  await open(page);
  const before = await shadow(page, (root) => ({
    stop: root.querySelector('[tabindex="0"]')?.outerHTML ?? null,
    sort: root.querySelector("thead th[aria-sort]")?.dataset.col ?? null,
    selected: root.querySelectorAll('tr[aria-selected="true"]').length,
  }));
  await page.locator("#reference").focus();
  const start = await page.locator("#reference").inputValue();
  for (const key of ["ArrowUp", "ArrowDown", "ArrowUp", "ArrowRight", "ArrowUp", "Home", "End", "Enter", "ArrowLeft", "Delete"]) {
    await press(page, key);
  }
  expect(await page.evaluate(() => window.__refKeys)).toHaveLength(10);
  // The field took the arrows: its value moved, and it kept the focus.
  expect(await page.locator("#reference").inputValue()).not.toBe(start);
  expect(await page.evaluate(() => document.activeElement.id)).toBe("reference");
  expect(
    await shadow(page, (root) => ({
      stop: root.querySelector('[tabindex="0"]')?.outerHTML ?? null,
      sort: root.querySelector("thead th[aria-sort]")?.dataset.col ?? null,
      selected: root.querySelectorAll('tr[aria-selected="true"]').length,
    })),
  ).toEqual(before);
  expect(await events(page)).toEqual([]);

  // A call that redraws the toolbar leaves the field its focus.
  await page.evaluate(() =>
    window.__module.set_toolbar(document.querySelector("opengrid-grid"), [{ id: "x", label: "X" }]),
  );
  expect(await page.evaluate(() => document.activeElement.id)).toBe("reference");
});

/** The toolbar's shape, the page's items and the slot included. */
const shape = (page) =>
  page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const row = root.querySelector("[data-toolbar-row]");
    const mid = (box) => Math.round((box.top + box.bottom) / 2);
    const controls = [
      ...[...row.querySelectorAll("button")].filter((button) => !button.closest('[part="columns"]')),
      document.querySelector("#reference"),
    ];
    return {
      display: getComputedStyle(row).display,
      lines: new Set(controls.map((control) => mid(control.getBoundingClientRect()))).size,
      scrolls: row.scrollWidth > row.clientWidth,
      pageScrolls: document.documentElement.scrollWidth > window.innerWidth,
      slottedShrink: getComputedStyle(document.querySelector("label[slot]")).flexShrink,
    };
  });

test("on a phone the page's items and the slot stay in the one row that scrolls", async ({ page }) => {
  await open(page, 375);
  const narrow = await shape(page);
  expect(narrow).toEqual({ display: "flex", lines: 1, scrolls: true, pageScrolls: false, slottedShrink: "0" });

  // Each of the page's items and the field come into view when they take the focus.
  for (const id of ["prepare", "export", "bulk"]) {
    await focusItem(page, id);
    expect(
      await shadow(page, (root) => {
        const box = root.activeElement.getBoundingClientRect();
        const frame = root.querySelector("[data-toolbar-row]").getBoundingClientRect();
        return box.left >= frame.left - 1 && box.right <= frame.right + 1;
      }),
      id,
    ).toBe(true);
  }
  await page.locator("#reference").focus();
  expect(
    await page.evaluate(() => {
      const box = document.querySelector("#reference").getBoundingClientRect();
      const frame = document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector("[data-toolbar-row]")
        .getBoundingClientRect();
      return box.left >= frame.left - 1 && box.right <= frame.right + 1;
    }),
  ).toBe(true);
});

test("at a 320 px viewport the toolbar wraps, the page's items with it", async ({ page }) => {
  await open(page, 320);
  const reflow = await shape(page);
  expect(reflow.display).toBe("contents");
  expect(reflow.lines).toBeGreaterThan(1);
  expect(reflow.pageScrolls).toBe(false);
});

test("a wide grid keeps one toolbar row", async ({ page }) => {
  await open(page, 1600);
  const wide = await shape(page);
  expect(wide.display).toBe("contents");
  expect(wide.lines).toBe(1);
});

test("has no axe violations: the toolbar, an open menu, on a phone", async ({ page }) => {
  await open(page);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await item(page, "export").click();
  expect(await menuOpen(page)).toBe(true);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await press(page, "Escape");
  await page.setViewportSize({ width: 375, height: 800 });
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
