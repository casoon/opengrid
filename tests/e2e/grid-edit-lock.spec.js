import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// Who may edit (issue #527): the grid's lock and the toolbar's switch for it,
// the columns the page marked, and the columns the reader locked.

const ID = 0;
const NAME = 1;
const SEATS = 2;
const CITY = 3;

const open = async (page, query = "") => {
  await page.goto(`/tests/e2e/fixtures/grid-edit-lock.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => shadow(page, (root) => root.querySelectorAll("tbody tr[aria-rowindex]").length)).toBe(6);
};
const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
const editorOpens = async (page, col) => {
  await focusCell(page, 0, col);
  await page.keyboard.press("Enter");
  const opened = await shadow(page, (root) => root.querySelector('[part="editor"]') !== null);
  await page.keyboard.press("Escape");
  return opened;
};
const toggle = (page) => shadow(page, (root) => root.querySelector('[part="edit-toggle"]')?.getAttribute("aria-pressed"));
const clickToggle = (page) => shadow(page, (root) => root.querySelector('[part="edit-toggle"]').click());
const locked = (page) => page.evaluate(() => document.querySelector("opengrid-grid").hasAttribute("edit-locked"));
const view = (page) => page.evaluate(() => window.__module.get_view(document.querySelector("opengrid-grid")));
const mark = (page, col) =>
  shadow(page, (root, col) => root.querySelector(`thead th[data-col="${col}"]`).querySelector('[part="edit-mark"]') !== null, col);
const menuActions = async (page, col) => {
  await shadow(page, (root, col) => root.querySelector(`thead th[data-col="${col}"]`).focus(), col);
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(() => shadow(page, (root) => root.querySelector('[part="column-menu"] [role="menuitem"]') !== null)).toBe(true);
  return shadow(page, (root) => [...root.querySelectorAll('[part="column-menu"] [data-action]')].map((item) => item.dataset.action));
};
const choose = (page, action) =>
  shadow(page, (root, action) => root.querySelector(`[part="column-menu"] [data-action="${action}"]`).click(), action);

test.describe("the toolbar's switch", () => {
  test("is pressed while editing is on, and sets and removes edit-locked", async ({ page }) => {
    await open(page);
    expect(await toggle(page)).toBe("true");
    expect(await editorOpens(page, NAME)).toBe(true);

    // Sort first, to see that the switch costs the grid nothing.
    await shadow(page, (root) => root.querySelector('thead th[data-col="1"]').click());
    await expect.poll(() => view(page).then((view) => view.sort.length)).toBe(1);
    const queries = await page.evaluate(() => window.__queries);

    await clickToggle(page);
    expect(await locked(page)).toBe(true);
    expect(await toggle(page)).toBe("false");
    await expect.poll(() => status(page)).toContain("Editing is locked");
    expect(await editorOpens(page, NAME)).toBe(false);

    await clickToggle(page);
    expect(await locked(page)).toBe(false);
    expect(await toggle(page)).toBe("true");
    await expect.poll(() => status(page)).toContain("Editing is unlocked");
    expect(await editorOpens(page, NAME)).toBe(true);
    // No query, and the sort stays.
    expect(await page.evaluate(() => window.__queries)).toBe(queries);
    expect((await view(page)).sort.length).toBe(1);
  });

  test("a page that reads first sets edit-locked and the switch unlocks it", async ({ page }) => {
    await open(page, "?locked");
    expect(await toggle(page)).toBe("false");
    expect(await editorOpens(page, NAME)).toBe(false);
    await clickToggle(page);
    expect(await toggle(page)).toBe("true");
    expect(await editorOpens(page, NAME)).toBe(true);
  });

  test("the page's own attribute change moves the switch, and without toolbar there is none", async ({ page }) => {
    await open(page);
    await page.evaluate(() => document.querySelector("opengrid-grid").toggleAttribute("edit-locked", true));
    await expect.poll(() => toggle(page)).toBe("false");
    await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("toolbar"));
    await expect.poll(() => toggle(page)).toBeUndefined();
  });

  test("has no axe violations", async ({ page }) => {
    await open(page);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

test.describe("the columns the page marked", () => {
  test("without a marked column every column edits, as it always did", async ({ page }) => {
    await open(page);
    expect(await editorOpens(page, NAME)).toBe(true);
    expect(await editorOpens(page, SEATS)).toBe(true);
    expect(await editorOpens(page, CITY)).toBe(true);
  });

  test("with one marked, only the marked columns edit", async ({ page }) => {
    await open(page, "?marked");
    expect(await editorOpens(page, NAME)).toBe(true);
    expect(await editorOpens(page, SEATS)).toBe(true);
    expect(await editorOpens(page, CITY)).toBe(false);
    expect(await mark(page, CITY)).toBe(true);
    expect(await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="3"]').getAttribute("aria-readonly"))).toBe("true");
  });

  test("readonly: false counts as marked", async ({ page }) => {
    await open(page, "?marked&opened");
    expect(await editorOpens(page, CITY)).toBe(true);
  });

  test("a locked grid is locked whatever the columns say, and the switch gives the columns back", async ({ page }) => {
    await open(page, "?marked&locked");
    expect(await editorOpens(page, NAME)).toBe(false);
    await clickToggle(page);
    expect(await editorOpens(page, NAME)).toBe(true);
    expect(await editorOpens(page, CITY)).toBe(false);
  });
});

test.describe("the columns the reader locked", () => {
  test("a column locks and unlocks from its menu, and the view holds it", async ({ page }) => {
    await open(page, "?marked");
    expect(await menuActions(page, NAME)).toContain("lock");
    expect(await menuActions(page, NAME)).not.toContain("unlock");
    await choose(page, "lock");
    await expect.poll(() => status(page)).toContain("name locked against editing");
    expect(await editorOpens(page, NAME)).toBe(false);
    expect(await editorOpens(page, SEATS)).toBe(true);
    expect(await mark(page, NAME)).toBe(true);
    expect((await view(page)).locked).toEqual(["name"]);
    expect(await page.evaluate(() => window.__views.at(-1).locked)).toEqual(["name"]);

    expect(await menuActions(page, NAME)).toContain("unlock");
    await choose(page, "unlock");
    await expect.poll(() => status(page)).toContain("name unlocked for editing");
    expect(await editorOpens(page, NAME)).toBe(true);
    expect((await view(page)).locked).toEqual([]);
  });

  test("never beyond what the page allows: a read-only column offers neither", async ({ page }) => {
    await open(page, "?marked");
    for (const col of [ID, CITY]) {
      const actions = await menuActions(page, col);
      expect(actions).not.toContain("lock");
      expect(actions).not.toContain("unlock");
      await page.keyboard.press("Escape");
    }
    await open(page, "?readonlyseats");
    const actions = await menuActions(page, SEATS);
    expect(actions).not.toContain("lock");
  });

  test("a locked grid offers no lock; a column's lock stays when it is unlocked again", async ({ page }) => {
    await open(page, "?locked");
    expect(await menuActions(page, NAME)).not.toContain("lock");
    await page.keyboard.press("Escape");
    await clickToggle(page);
    await menuActions(page, NAME);
    await choose(page, "lock");
    await expect.poll(() => locked(page).then((locked) => !locked && true)).toBe(true);
    await clickToggle(page);
    await clickToggle(page);
    expect(await editorOpens(page, NAME)).toBe(false);
  });

  test("set_view restores the locks and refuses a column the grid does not have", async ({ page }) => {
    await open(page);
    await page.evaluate(() =>
      window.__module.set_view(document.querySelector("opengrid-grid"), { ...window.__module.get_view(document.querySelector("opengrid-grid")), locked: ["seats"] }),
    );
    await expect.poll(() => view(page).then((view) => view.locked)).toEqual(["seats"]);
    expect(await editorOpens(page, SEATS)).toBe(false);
    expect(await editorOpens(page, NAME)).toBe(true);

    await page.evaluate(() =>
      window.__module.set_view(document.querySelector("opengrid-grid"), { ...window.__module.get_view(document.querySelector("opengrid-grid")), locked: ["nope"] }),
    );
    await expect.poll(() => status(page)).toContain("nope");
    expect((await view(page)).locked).toEqual(["seats"]);
  });

  test("has no axe violations with the menu open on a locked column", async ({ page }) => {
    await open(page, "?marked");
    await menuActions(page, NAME);
    await choose(page, "lock");
    await menuActions(page, NAME);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

test("a menu with no room below or above its header scrolls inside the window", async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 400 });
  await open(page, "?marked");
  await menuActions(page, NAME);
  const box = await shadow(page, (root) => {
    const rect = root.querySelector('[part="column-menu"]').getBoundingClientRect();
    return { bottom: rect.bottom, scrolls: root.querySelector('[part="column-menu"]').scrollHeight > rect.height };
  });
  expect(box.bottom).toBeLessThanOrEqual(400);
  expect(box.scrolls).toBe(true);
});
