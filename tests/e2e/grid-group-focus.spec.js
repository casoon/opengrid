import { test, expect } from "@playwright/test";

// Focus and the active cell after the grouping changes (issue #345).
//
// The grid acts on its active cell — the one that holds `tabindex="0"` — and
// the keys reach whatever has the DOM focus. They have to be the same element,
// or `Enter` does nothing until the focus moves (WCAG 2.4.3, 2.4.7). A change
// of `group-by` used to put the active cell on the first header and leave the
// focus on the old cell.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(
        document.querySelector("opengrid-grid").shadowRoot,
        arg,
      ),
    { fn: fn.toString(), arg },
  );

/** What has the focus and what the grid acts on, as `th2` or `td3.1` (row.col). */
const where = (page) =>
  shadow(page, (root) => {
    const name = (element) => {
      if (!element) return null;
      const col = element.getAttribute("data-col");
      return element.tagName === "TH" ? `th${col}` : `td${element.getAttribute("data-row")}.${col}`;
    };
    return { focus: name(root.activeElement), active: name(root.querySelector('[tabindex="0"]')) };
  });

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-group-select.html?menu");
  await page.waitForFunction(() => window.__ready === true);
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("group-by"));
  await expect.poll(() => where(page).then((state) => state.active)).toBe("th0");
}

test("grouping while a cell has the focus: the focus and the active cell go to its column's header", async ({
  page,
}) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="3"][data-col="1"]').focus());
  expect(await where(page)).toEqual({ focus: "td3.1", active: "td3.1" });

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country"),
  );
  await expect.poll(() => where(page)).toEqual({ focus: "th1", active: "th1" });

  // The next key acts on what has the focus: down from the header of column 1.
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => where(page)).toEqual({ focus: "td0.1", active: "td0.1" });
});

test("grouping from the column menu leaves the focus on the header it came from", async ({
  page,
}) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('th[data-col="2"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await expect
    .poll(() => shadow(page, (root) => root.activeElement?.closest('[part="column-menu"]') !== null))
    .toBe(true);
  await shadow(page, (root) =>
    root.querySelector('[part="column-menu"] [data-action^="group:"]').click(),
  );
  await expect.poll(() => where(page)).toEqual({ focus: "th2", active: "th2" });

  await page.keyboard.press("ArrowDown");
  await expect.poll(() => where(page)).toEqual({ focus: "td0.2", active: "td0.2" });
});

test("a header that has the focus keeps it when the grouping changes", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('th[data-col="3"]').focus());
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country"),
  );
  await page.waitForTimeout(250);
  expect(await where(page)).toEqual({ focus: "th3", active: "th3" });
});
