import { test, expect } from "@playwright/test";

// Issue #223: what a new answer does to the row the focus stands on, and to a
// focus request whose query a newer one superseded.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

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

// PageDown moves the focus past the loaded rows, which asks for a window and
// wants the focus back with the answer.
async function pageDownFar(page) {
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').focus());
  await page.evaluate(() => (window.__hold = true));
  for (let i = 0; i < 4; i++) await page.keyboard.press("PageDown");
}

test("a focus request survives the newer query that superseded its own", async ({ page }) => {
  await openLate(page);
  await pageDownFar(page);
  // A scroll asks again, and asks for no focus.
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]').scrollTop += 32;
  });
  await page.evaluate(() => {
    window.__hold = false;
    window.__releaseAll();
  });

  // The focus stands on the active cell — the one with the tab stop, far down
  // the list — not on whatever cell the pool recycled under it.
  const stands = () =>
    shadow(page, (root) => {
      const focused = root.activeElement;
      const stop = root.querySelector('tbody td[tabindex="0"]');
      return { focused: focused?.dataset.row ?? null, stop: stop?.dataset.row ?? null };
    });
  await expect.poll(async () => {
    const { focused, stop } = await stands();
    return focused !== null && focused === stop && Number(stop) > 10;
  }).toBe(true);
});

test("an answer does not pull the focus back from a filter field the reader went to", async ({ page }) => {
  await openLate(page);
  await pageDownFar(page);
  await shadow(page, (root) => root.querySelector('input[data-col="1"]').focus());
  await page.evaluate(() => {
    window.__hold = false;
    window.__releaseAll();
  });
  await page.waitForTimeout(300);

  expect(await focusedPart(page)).toMatchObject({ tag: "input", cell: false });
});
