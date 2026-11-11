import { test, expect } from "@playwright/test";
import { shadow } from "./helpers.js";

// Issue #223: what a new answer does to the row the focus stands on, and to a
// focus request whose query a newer one superseded.

const focusedPart = (page) =>
  shadow(page, (root) => {
    const node = root.activeElement;
    return node ? { tag: node.tagName.toLowerCase(), cell: !!node.closest("td"), col: node.dataset.col ?? null } : null;
  });

test("a filter from the filter row does not leave the focused row showing the old record", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid.html");
  await page.evaluate(() => window.__opengridReady);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();

  // Record 4 (Alpha) is in row 3; the focus stands on it.
  await shadow(page, (root) => root.querySelector('td[data-row="3"][data-col="1"]').focus());
  await shadow(page, (root) => root.querySelector('input[data-col="1"]').focus());
  await page.keyboard.type("Gamma");
  await page.keyboard.press("Enter");

  await expect.poll(() => shadow(page, (root) => root.querySelector('[part="status"]').textContent)).toBe("1 match");
  const cells = () =>
    shadow(page, (root) =>
      [...root.querySelectorAll('tbody td[data-col="1"]')]
        .filter((td) => td.closest("tr").getAttribute("aria-rowindex"))
        .map((td) => ({ row: td.dataset.row, text: td.textContent.trim(), tab: td.getAttribute("tabindex") })),
    );
  await expect.poll(async () => (await cells()).map((cell) => cell.text)).toEqual(["Gamma"]);
  // One match, one row: nothing is left at row 3, and the single tab stop is on row 0.
  const rows = await cells();
  expect(rows).toEqual([{ row: "0", text: "Gamma", tab: "0" }]);
});

async function openLate(page) {
  await page.goto("/tests/e2e/fixtures/grid-late.html");
  await page.evaluate(() => window.__opengridReady);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

// Ctrl+End moves the focus to the last row, which is outside the loaded rows:
// the grid asks for that window and wants the focus back with the answer. The
// queries are held back until the test has asked for what it wants, so no
// frame decides the order.
async function ctrlEndHeld(page) {
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').focus());
  await page.evaluate(() => (window.__hold = true));
  await page.keyboard.press("Control+End");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBeGreaterThan(0);
}

const release = (page) =>
  page.evaluate(() => {
    window.__hold = false;
    window.__releaseAll();
  });

test("a focus request survives the newer query that superseded its own", async ({ page }) => {
  await openLate(page);
  await ctrlEndHeld(page);
  const asked = await page.evaluate(() => window.__held.length);
  // A second window query, which asks for no focus: `add-row` turns the new row
  // on and asks for the window again. It supersedes the first one.
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("add-row", ""));
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBeGreaterThan(asked);
  await release(page);

  // The focus stands on the active cell — the last row, the one with the tab
  // stop — not on whatever cell the pool recycled under it.
  const stands = () =>
    shadow(page, (root) => ({
      focused: root.activeElement?.dataset.row ?? null,
      stop: root.querySelector('tbody td[tabindex="0"]')?.dataset.row ?? null,
    }));
  await expect.poll(stands).toEqual({ focused: "59", stop: "59" });
});

test("an answer does not pull the focus back from a filter field the reader went to", async ({ page }) => {
  await openLate(page);
  await ctrlEndHeld(page);
  await shadow(page, (root) => root.querySelector('input[data-col="1"]').focus());
  await release(page);
  await page.waitForTimeout(300);

  expect(await focusedPart(page)).toMatchObject({ tag: "input", cell: false });
});
