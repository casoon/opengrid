import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Truncated cell values (plan point 47).
//
// The grid is one line per row with an ellipsis, because the virtualization
// needs a fixed row height. The full value is in the DOM — a screen reader reads
// it whole — but a sighted user would lose it (WCAG 1.4.4, 1.4.10). The focused
// cell therefore unfolds: it wraps and grows over the rows below.
//
// The fixture is 60 rows in a 420px-wide grid with a pool of 10, so scrolling
// and row recycling really happen. Three values do not fit: row 0 (with
// spaces), row 2 (one long word, which only `overflow-wrap` breaks) and row 59
// (the last row, the worst case for the layout).

/** Geometry and text of one cell in the shadow root. */
async function cell(page, row, col) {
  return page.evaluate(
    ({ row, col }) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const node = root.querySelector(`td[data-row="${row}"][data-col="${col}"]`);
      if (!node) return null;
      const box = node.getBoundingClientRect();
      return {
        text: node.textContent,
        height: Math.round(box.height),
        // Is anything clipped? The scroll size of the cell exceeds its box when
        // the value does not fit into it.
        clipped:
          node.scrollWidth > node.clientWidth + 1 || node.scrollHeight > node.clientHeight + 1,
        focused: root.activeElement === node,
      };
    },
    { row, col },
  );
}

/**
 * What the virtualization owns. The sizer is `total_count * row_height` and the
 * pool is a fixed number of `<tr>`; neither may move because a cell unfolded.
 */
async function virtualization(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      domRows: root.querySelectorAll("tbody tr").length,
      sizer: Math.round(root.querySelector("tbody").getBoundingClientRect().height),
      rowcount: root.querySelector('table[role="grid"]').getAttribute("aria-rowcount"),
    };
  });
}

/** The scroll state of the viewport. */
async function scroll(page) {
  return page.evaluate(() => {
    const viewport = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="viewport"]');
    return { height: viewport.scrollHeight, top: viewport.scrollTop };
  });
}

/** Focuses a cell of the loaded window directly. */
async function focusCell(page, row, col) {
  await page.evaluate(
    ({ row, col }) => {
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`td[data-row="${row}"][data-col="${col}"]`)
        .focus();
    },
    { row, col },
  );
}

/** Focuses a cell through the component's keyboard path (header, then down). */
async function focusByKeyboard(page, row, col) {
  await page.evaluate((col) => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector(`th[data-col="${col}"]`)
      .focus();
  }, col);
  for (let step = 0; step <= row; step += 1) {
    await page.keyboard.press("ArrowDown");
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-long.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("an unfocused cell is one line and the full value is still in the DOM", async ({
  page,
}) => {
  const unfocused = await cell(page, 0, 1);
  expect(unfocused.height).toBe(32);
  expect(unfocused.clipped).toBe(true);
  // What the screen reader reads is complete; only CSS shortens it.
  expect(unfocused.text).toBe(
    "Kundenname der weit breiter ist als seine Spalte und deshalb abgeschnitten wird",
  );
});

test("the focused cell unfolds until the whole value fits", async ({ page }) => {
  await focusByKeyboard(page, 0, 1);

  const focused = await cell(page, 0, 1);
  expect(focused.focused).toBe(true);
  expect(focused.height).toBeGreaterThan(32);
  expect(focused.clipped).toBe(false);

  // Moving on folds it back: exactly one cell is ever unfolded.
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => cell(page, 0, 1).then((c) => c.height)).toBe(32);
});

test("a value without spaces unfolds too", async ({ page }) => {
  // `white-space: normal` alone cannot break this one — `overflow-wrap` can.
  await focusByKeyboard(page, 2, 1);

  const focused = await cell(page, 2, 1);
  expect(focused.focused).toBe(true);
  expect(focused.height).toBeGreaterThan(32);
  expect(focused.clipped).toBe(false);
});

test("unfolding leaves the sizer, the pool and the scroll area untouched", async ({
  page,
}) => {
  const before = await virtualization(page);
  const resting = await scroll(page);
  await focusCell(page, 2, 1);
  expect(await cell(page, 2, 1).then((c) => c.height)).toBeGreaterThan(32);

  // The row is absolutely positioned, so it grows *over* the rows below instead
  // of moving them: the sizer (total_count * row_height), the pool and
  // aria-rowcount are exactly what they were, and the grid scrolls as far as it
  // did — the unfolded row ends long before the last one.
  expect(await virtualization(page)).toEqual(before);
  expect(before.domRows).toBe(10);
  expect(before.sizer).toBe(60 * 32);
  expect((await scroll(page)).height).toBe(resting.height);
});

test("at the end of the data the scroll area grows with the unfolded cell", async ({
  page,
}) => {
  // The one case where the unfolded row reaches past the sizer: the last row has
  // nothing below it to grow over.
  await page.evaluate(() => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="viewport"]').scrollTop = 1920;
  });
  await expect.poll(() => cell(page, 59, 1).then((c) => c?.height)).toBe(32);

  const sizer = (await virtualization(page)).sizer;
  const resting = await scroll(page);
  await focusCell(page, 59, 1);
  const unfolded = await scroll(page);
  const cellHeight = (await cell(page, 59, 1)).height;

  // Deliberate, not an accident: the value has to stay scrollable, or its last
  // lines would be unreachable. The grid grows by exactly the overshoot, and the
  // sizer — which the window arithmetic reads — does not move at all.
  expect(unfolded.height).toBe(resting.height + cellHeight - 32);
  expect((await virtualization(page)).sizer).toBe(sizer);

  // Folding it back restores the scroll area of the plain grid.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.activeElement.blur(),
  );
  await expect
    .poll(() => scroll(page).then((s) => s.height))
    .toBeLessThanOrEqual(resting.height);
});

/** The stacking level of the row holding logical row `row`, and of the header. */
async function layering(page, row) {
  return page.evaluate((row) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    // The logical row lives on the cells; the <tr> is a recycled pool slot.
    const tr = root.querySelector(`td[data-row="${row}"]`).closest("tr");
    return {
      row: getComputedStyle(tr).zIndex,
      thead: getComputedStyle(root.querySelector("thead")).zIndex,
    };
  }, row);
}

test("the unfolded row stays under the sticky header", async ({ page }) => {
  // Rows default to `auto`; the sticky header sits above whatever a row is
  // raised to while it holds the focus.
  const resting = await layering(page, 0);
  expect(resting.row).toBe("auto");
  expect(Number.parseInt(resting.thead, 10)).toBeGreaterThan(1);

  await focusByKeyboard(page, 0, 1);
  const focused = await layering(page, 0);
  expect(Number.parseInt(focused.row, 10)).toBeGreaterThan(0);
  expect(Number.parseInt(focused.row, 10)).toBeLessThan(
    Number.parseInt(focused.thead, 10),
  );
});

test("scrolling past an unfolded cell keeps the pool and the row indices intact", async ({
  page,
}) => {
  await focusCell(page, 2, 1);
  const before = await virtualization(page);

  // Scroll far away from the unfolded — and therefore pinned — row.
  await page.evaluate(() => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="viewport"]').scrollTop = 1200;
  });
  await expect.poll(() => cell(page, 39, 1).then((c) => c?.text)).toBe("Kunde 40");

  // No node was created or removed and the sizer did not move; the rows still
  // carry the 1-based index including the header (row r -> r + 2).
  expect(await virtualization(page)).toEqual(before);
  expect(
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="39"]')
        .closest("tr")
        .getAttribute("aria-rowindex"),
    ),
  ).toBe("41");

  // The focused row keeps its slot even though it scrolled out of the window
  // (the focus pinning of point 17) — one slot of the pool is spent on it, which
  // is why the window below is one row shorter.
  expect(await cell(page, 2, 1).then((c) => c.focused)).toBe(true);
});

test("the last row unfolds too", async ({ page }) => {
  // The worst case for the layout: there is nothing below it to grow over.
  await page.evaluate(() => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="viewport"]').scrollTop = 1920;
  });
  await expect.poll(() => cell(page, 59, 1).then((c) => c?.height)).toBe(32);

  await focusCell(page, 59, 1);
  const focused = await cell(page, 59, 1);
  expect(focused.height).toBeGreaterThan(32);
  expect(focused.clipped).toBe(false);
});

test("an over-tall value starts at its first line, below the sticky header", async ({
  page,
  browserName,
}) => {
  // 400% zoom of a 1280x1024 window is a 320x256 CSS-pixel viewport (WCAG 1.4.10).
  await page.setViewportSize({ width: 320, height: 256 });
  await page.locator("opengrid-grid").scrollIntoViewIfNeeded();
  await focusByKeyboard(page, 0, 1);

  const reading = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const box = root.activeElement.getBoundingClientRect();
    const viewport = root.querySelector('[part="viewport"]').getBoundingClientRect();
    const header = root.querySelector("thead").getBoundingClientRect();
    return {
      tallerThanViewport: box.height > viewport.height,
      // `scroll-margin-top` keeps the first line out from under the sticky
      // header; without it the value starts above the visible area and its
      // first words cannot be read.
      startsBelowHeader: box.top >= header.bottom - 1,
      documentFits: document.documentElement.scrollWidth <= window.innerWidth,
    };
  });

  // At this size the value does not fit on one screen — it is read by scrolling,
  // which is exactly why it has to start at the top.
  expect(reading).toEqual({
    tallerThanViewport: true,
    startsBelowHeader: true,
    documentFits: true,
  });
  if (browserName === "chromium") {
    await expect(page.locator("opengrid-grid")).toHaveScreenshot("grid-unfolded-zoom.png");
  }
});

test("matches the unfolded baseline", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "Screenshot baselines are Chromium's (tests/e2e/playwright.config.js)");
  await focusByKeyboard(page, 0, 1);
  await expect(page.locator("opengrid-grid")).toHaveScreenshot("grid-unfolded.png");
});

test("a narrow column truncates its name, not its sort direction", async ({ page }) => {
  // Point 49: the sort marks are the *last* content of the header, so a column
  // name wider than its column used to push them out of the clipped box — the
  // direction disappeared exactly where the column is too narrow to read it
  // anyway. The name gives way instead. At 320px the four columns are 76px
  // wide and "customer" no longer fits, so this is the real case, not a
  // constructed one.
  await page.setViewportSize({ width: 320, height: 600 });
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('th[data-col="1"]')
      .focus(),
  );
  await page.keyboard.press("Enter");

  await expect
    .poll(() =>
      page.evaluate(() =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector('th[data-col="1"] [part="sort-direction"]').textContent,
      ),
    )
    .toBe("▲");

  const header = await page.evaluate(() => {
    const th = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('th[data-col="1"]');
    const name = th.querySelector("span");
    const glyph = th.querySelector('[part="sort-direction"]');
    const thBox = th.getBoundingClientRect();
    const glyphBox = glyph.getBoundingClientRect();
    return {
      nameTruncated: name.scrollWidth > name.clientWidth + 1,
      glyphVisible: glyphBox.width > 0 && glyphBox.right <= thBox.right + 0.5,
    };
  });
  expect(header).toEqual({ nameTruncated: true, glyphVisible: true });
});

test("has no axe violations with a cell unfolded", async ({ page }) => {
  await focusByKeyboard(page, 0, 1);
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
