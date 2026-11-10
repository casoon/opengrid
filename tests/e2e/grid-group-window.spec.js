import { test, expect } from "@playwright/test";

// A grouped grid's window (issue #342).
//
// The grouped path took the overscan off an offset that already was a window
// start, so the window sat six rows short of where the keys asked for it:
// `Ctrl`+`End` stopped before the last position, and `ArrowDown` at the
// window's edge needed two presses. A tree is windowed by the same line.
//
// The fixture has a pool of twelve rows; DE opened under a single level gives
// 58 display positions, far more than one window.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(
        document.querySelector("opengrid-grid").shadowRoot,
        arg,
      ),
    { fn: fn.toString(), arg },
  );

const rowcount = (page) =>
  shadow(page, (root) => Number(root.querySelector("table").getAttribute("aria-rowcount")));

/** The display position of the cell that has the focus, or null. */
const focused = (page) =>
  shadow(page, (root) => {
    const row = root.activeElement?.getAttribute("data-row");
    return row == null ? null : Number(row);
  });

/** One level, DE open: the header row and 58 positions, 0 to 57. */
async function openDe(page) {
  await page.goto("/tests/e2e/fixtures/grid-group-select.html");
  await page.waitForFunction(() => window.__ready === true);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country"),
  );
  const first = () =>
    shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]')?.textContent);
  await expect.poll(first).toBe("country: DE (52 rows)");
  await shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 52);
}

test("Ctrl+End reaches the last position of a grouped grid", async ({ page }) => {
  await openDe(page);
  await page.keyboard.press("Control+End");
  await expect.poll(() => focused(page)).toBe(57);
});

test("ArrowDown moves one position with every press, across the window's edge", async ({
  page,
}) => {
  await openDe(page);
  // Short enough that the pool of twelve holds what is visible and the
  // overscan above it: at the fixture's height it does not, grouped or not.
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").style.height = "220px";
  });
  // From the group's header through two windows of twelve. The window moves
  // with the scrolling the focus causes, and the focus stays on its row.
  for (let position = 1; position <= 30; position += 1) {
    await page.keyboard.press("ArrowDown");
    await expect.poll(() => focused(page), `press ${position}`).toBe(position);
  }
});
