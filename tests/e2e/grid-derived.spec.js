import { test, expect } from "@playwright/test";
import { shadow, status } from "./helpers.js";

// A derived field is read-only (issue #528): the year of a date has nothing to write back to. A
// result's schema never says which field is derived, so the grid asks the provider what the
// source is — `describe(source)` — once per `datasource`.

const NAME = 1;
const YEAR = 3;

const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
const editorOpen = (page) => shadow(page, (root) => root.querySelector('[part="editor"]') !== null);
// `data-edit` says "readonly" for a column that cannot be edited; a column nobody marked says nothing.
const cellEdit = (page, col) =>
  shadow(page, (root, col) => root.querySelector(`td[data-row="0"][data-col="${col}"]`)?.getAttribute("data-edit") ?? "", col);
const headerEdit = (page, col) =>
  shadow(page, (root, col) => root.querySelector(`th[data-col="${col}"]`)?.getAttribute("data-edit") ?? "", col);
const cellText = (page, col) =>
  shadow(page, (root, col) => root.querySelector(`td[data-row="0"][data-col="${col}"]`)?.textContent ?? "", col);

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-derived.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => cellText(page, NAME)).toBe("R1");
}

for (const provider of ["local", "worker"]) {
  test(`the ${provider} provider tells the grid, and a derived field opens no editor`, async ({ page }) => {
    await open(page, `?provider=${provider}`);
    await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
    expect(await headerEdit(page, YEAR)).toBe("readonly");
    await focusCell(page, 0, YEAR);
    for (const press of ["Enter", "F2", "7"]) {
      await page.keyboard.press(press);
      expect(await editorOpen(page), press).toBe(false);
    }
    // The other columns stay as they were.
    expect(await cellEdit(page, NAME)).toBe("");
    await focusCell(page, 0, NAME);
    await page.keyboard.press("Enter");
    expect(await editorOpen(page)).toBe(true);
  });
}

test("the new row opens no editor on a derived field, and a paste names it", async ({ page }) => {
  await open(page);
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
  await shadow(page, (root, col) => root.querySelector(`tr[data-new-row] td[data-col="${col}"]`).focus(), YEAR);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(false);
  await focusCell(page, 0, YEAR);
  await page.evaluate(() => {
    const data = new DataTransfer();
    data.setData("text/plain", "2030\r\n");
    const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
  });
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent ?? ""))
    .toContain("opened_year is read-only");
});

test("readonly: false does not open a derived field", async ({ page }) => {
  await open(page, "?open");
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
  await focusCell(page, 0, YEAR);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(false);
});

test("a provider that cannot describe keeps every field as the result says", async ({ page }) => {
  await open(page, "?provider=plain");
  expect(await cellEdit(page, YEAR)).toBe("");
  await focusCell(page, 0, YEAR);
  await page.keyboard.press("Enter");
  expect(await editorOpen(page)).toBe(true);
});

test("a derived field the page marked editable is reported, and none of the call applied", async ({ page }) => {
  await open(page, "?editable");
  await expect.poll(() => status(page)).toContain("opened_year: is derived and editable at once");
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
});

test("a source replaced under its name is described again", async ({ page }) => {
  await open(page);
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
  await page.evaluate(() => window.__replace(false));
  await expect.poll(() => cellEdit(page, YEAR)).toBe("");
  await page.evaluate(() => window.__replace(true));
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
});

test("a hybrid provider describes through its remote half, and only if that can", async ({ page }) => {
  await open(page);
  const answers = await page.evaluate(async () => {
    const { createHybridProvider } = await import("/packages/opengrid/loader.js");
    const calls = [];
    const remote = { execute: async () => "", describe: async (source) => (calls.push(source), { name: source, schema: { fields: [] } }) };
    const described = await createHybridProvider({ remote, planner: {} }).describe("rooms");
    const without = createHybridProvider({ remote: { execute: async () => "" }, planner: {} });
    return { calls, described, without: typeof without.describe };
  });
  expect(answers).toEqual({ calls: ["rooms"], described: { name: "rooms", schema: { fields: [] } }, without: "undefined" });
});

test("a description that comes after the first rows still makes the field read-only, and is reported", async ({ page }) => {
  await open(page, "?provider=slow&editable");
  // Rows first, the schema later: until it comes, nothing says the field is derived.
  expect(await cellEdit(page, YEAR)).toBe("editable");
  await expect.poll(() => status(page)).toContain("opened_year: is derived and editable at once");
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
});

test("a column nobody marked stays as it was while the description is on its way", async ({ page }) => {
  await open(page, "?provider=slow");
  expect(await cellEdit(page, YEAR)).toBe("");
  await expect.poll(() => cellEdit(page, YEAR)).toBe("readonly");
  expect(await cellEdit(page, NAME)).toBe("");
});
