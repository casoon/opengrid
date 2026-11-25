import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// What can be edited, and what the reader sees of it (issue #523): the key
// column is read-only, a lock marks it; a column the
// page gives `editable: true` is tinted, with a pencil on a touch screen; the
// footer says how the focused cell opens, or that editing is locked.

const ID = 0;
const NAME = 1;
const SEATS = 2;
const OPENED = 3;

const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
const editorOpen = (page) => shadow(page, (root) => root.querySelector('[part="editor"]') !== null);
const cellAttr = (page, col, name) =>
  shadow(page, (root, { col, name }) => root.querySelector(`td[data-row="0"][data-col="${col}"]`).getAttribute(name), { col, name });
const headerAttr = (page, col, name) =>
  shadow(page, (root, { col, name }) => root.querySelector(`th[data-col="${col}"]`).getAttribute(name), { col, name });
const markShown = (page, col) =>
  shadow(page, (root, col) => getComputedStyle(root.querySelector(`th[data-col="${col}"] [part="edit-mark"]`)).display !== "none", col);
const background = (page, col) =>
  shadow(page, (root, col) => getComputedStyle(root.querySelector(`td[data-row="0"][data-col="${col}"]`)).backgroundColor, col);
const hint = (page) => shadow(page, (root) => root.querySelector('[part="edit-hint"]').textContent);
const events = (page) => page.evaluate(() => window.__events);
const setColumns = (page, columns) =>
  page.evaluate((columns) => window.__module.set_columns(document.querySelector("opengrid-grid"), columns), columns);
const TRANSPARENT = "rgba(0, 0, 0, 0)";

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-editable.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => cellAttr(page, NAME, "data-edit")).toBe("editable");
}

test("the key column opens no editor, and says so", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, ID);
  for (const press of ["Enter", "F2", "7"]) {
    await page.keyboard.press(press);
    expect(await editorOpen(page), press).toBe(false);
  }
  expect(await cellAttr(page, ID, "aria-readonly")).toBe("true");
  expect(await cellAttr(page, ID, "data-edit")).toBe("readonly");
  expect(await headerAttr(page, ID, "data-edit")).toBe("readonly");
  expect(await markShown(page, ID)).toBe(true);
  expect(await events(page)).toEqual([]);
  // The other columns stay as they were: editable, and nothing said about them.
  expect(await cellAttr(page, NAME, "aria-readonly")).toBeNull();
  await focusCell(page, 0, NAME);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(true);
  await page.keyboard.press("Escape");
});

test("a read-only column is drawn in the muted ink, an editable one is not", async ({ page }) => {
  await open(page);
  const colors = await shadow(page, (root) => {
    const host = root.host;
    const muted = getComputedStyle(host).getPropertyValue("--og-ink-muted").trim();
    const probe = document.createElement("span");
    probe.style.color = muted;
    root.append(probe);
    const expected = getComputedStyle(probe).color;
    probe.remove();
    const color = (col) => getComputedStyle(root.querySelector(`td[data-row="0"][data-col="${col}"]`)).color;
    return { expected, id: color(0), name: color(1) };
  });
  expect(colors.id).toBe(colors.expected);
  expect(colors.name).not.toBe(colors.expected);
});

test("readonly: false opens the key column", async ({ page }) => {
  await open(page);
  await setColumns(page, { id: { readonly: false }, name: { editable: true }, seats: { editable: true } });
  await expect.poll(() => cellAttr(page, ID, "aria-readonly")).toBeNull();
  expect(await headerAttr(page, ID, "data-edit")).toBeNull();
  await focusCell(page, 0, ID);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(true);
  await page.keyboard.press("Escape");
});

test("the new row does not require the read-only key and adds without it", async ({ page }) => {
  await open(page);
  const marks = await shadow(page, (root) =>
    [...root.querySelectorAll('tr[data-new-row] td[part~="cell"]')].map((td) => td.getAttribute("aria-required")));
  // id is not nullable, but read-only: the page or the source gives it.
  expect(marks).toEqual([null, "true", null, null]);
  await shadow(page, (root) => root.querySelector('tr[data-new-row] td[data-col="0"]').focus());
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(false);
  await shadow(page, (root) => root.querySelector('tr[data-new-row] td[data-col="1"]').focus());
  await page.keyboard.press("Enter");
  await page.keyboard.type("New");
  await page.keyboard.press("ControlOrMeta+Enter");
  await expect.poll(async () => (await events(page)).filter((event) => event.type === "opengrid-row-add").length).toBe(1);
  const added = (await events(page)).find((event) => event.type === "opengrid-row-add");
  expect(added.values.name).toBe("New");
  expect(added.values.id ?? null).toBeNull();
});

test("paste skips the read-only key and names it", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, ID);
  await page.evaluate(() => {
    const data = new DataTransfer();
    data.setData("text/plain", "9\tZ\r\n");
    const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
  });
  await expect.poll(() => shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent ?? "")).toContain("id is read-only");
});

test("an editable column is tinted, the tint is a token, and the others are not", async ({ page }) => {
  await open(page);
  expect(await background(page, NAME)).not.toBe(TRANSPARENT);
  expect(await background(page, SEATS)).not.toBe(TRANSPARENT);
  expect(await background(page, ID)).toBe(TRANSPARENT);
  // `opened` is neither read-only nor declared: drawn as before.
  expect(await cellAttr(page, OPENED, "data-edit")).toBeNull();
  expect(await background(page, OPENED)).toBe(TRANSPARENT);
  await page.evaluate(() => document.querySelector("opengrid-grid").style.setProperty("--og-editable", "transparent"));
  expect(await background(page, NAME)).toBe(TRANSPARENT);
  // With a mouse, no pencil: the cursor and the hint say it.
  expect(await markShown(page, NAME)).toBe(false);
});

test("the footer says how the focused cell opens, and that editing is locked", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, NAME);
  await expect.poll(() => hint(page)).toBe("Enter to edit");
  await focusCell(page, 0, ID);
  await expect.poll(() => hint(page)).toBe("");
  await focusCell(page, 0, OPENED);
  await expect.poll(() => hint(page)).toBe("");
  // Not the live region: moving the focus says nothing new there.
  expect(await shadow(page, (root) => root.querySelector('[part="edit-hint"]').closest("[aria-live]"))).toBeNull();

  await page.evaluate(() => document.querySelector("opengrid-grid").toggleAttribute("edit-locked", true));
  await expect.poll(() => hint(page)).toBe("Editing locked");
  // Locked, the grid draws no marks: the footer says it once.
  expect(await cellAttr(page, NAME, "data-edit")).toBeNull();
  expect(await headerAttr(page, ID, "data-edit")).toBeNull();
  expect(await markShown(page, ID)).toBe(false);
  expect(await background(page, NAME)).toBe(TRANSPARENT);
  await page.evaluate(() => document.querySelector("opengrid-grid").toggleAttribute("edit-locked", false));
  await expect.poll(() => hint(page)).toBe("");
  await expect.poll(() => cellAttr(page, NAME, "data-edit")).toBe("editable");
});

test("editable together with readonly is reported, and none of the call applied", async ({ page }) => {
  await open(page);
  await setColumns(page, { name: { editable: true, readonly: true }, seats: { editable: true } });
  await expect.poll(() => status(page)).toContain("name: is readonly and editable at once");
  // The call was refused whole: the configuration before it still draws.
  expect(await cellAttr(page, NAME, "data-edit")).toBe("editable");
  expect(await cellAttr(page, NAME, "aria-readonly")).toBeNull();
});

test("no axe violations, and under a forced palette the lock stays", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, NAME);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
  await page.emulateMedia({ forcedColors: "active" });
  expect(await markShown(page, ID)).toBe(true);
  // The tint is gone: whatever colour the palette names, nothing is painted.
  expect(await background(page, NAME)).toMatch(/^rgba\(\d+, \d+, \d+, 0\)$/);
});

test.describe("on a touch screen", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 800 } });

  test("an editable column shows a pencil, a read-only one the lock", async ({ page }) => {
    await open(page);
    expect(await shadow(page, () => matchMedia("(pointer: coarse)").matches)).toBe(true);
    expect(await markShown(page, NAME)).toBe(true);
    expect(await markShown(page, SEATS)).toBe(true);
    expect(await markShown(page, ID)).toBe(true);
    expect(await markShown(page, OPENED)).toBe(false);
    // Turning the tint off keeps the pencil.
    await page.evaluate(() => document.querySelector("opengrid-grid").style.setProperty("--og-editable", "transparent"));
    expect(await markShown(page, NAME)).toBe(true);
  });
});
