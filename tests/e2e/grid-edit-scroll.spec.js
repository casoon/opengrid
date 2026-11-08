import { test, expect } from "@playwright/test";

// Issue #224: a scroll brings rows of the same result. It must not take the
// cell editor away from a reader who is typing: the editor sits in the slot the
// renderer keeps for the focused row, and Escape has to still close it.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-long.html");
  await page.evaluate(() => window.__opengridReady);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

test("an editor stays open while a scroll brings other rows, and Escape closes it", async ({ page }) => {
  await open(page);
  const grid = page.locator("opengrid-grid");
  const editor = grid.locator('[part="editor"]');

  await grid.locator('td[data-row="0"][data-col="2"]').focus();
  await page.keyboard.press("Enter");
  await expect(editor).toHaveCount(1);
  await page.keyboard.type("77");
  const typed = await editor.inputValue();

  // Far enough that the rows around row 0 are not the loaded ones any more:
  // the grid asks for a window and applies the answer.
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]').scrollTop = 960;
  });
  await expect(grid.locator('td[data-row="30"][data-col="0"]')).toHaveCount(1);

  await expect(editor).toHaveCount(1);
  await expect(editor).toHaveValue(typed);
  await page.keyboard.press("Escape");
  await expect(editor).toHaveCount(0);
});
