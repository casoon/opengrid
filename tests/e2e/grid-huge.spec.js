import { test, expect } from "@playwright/test";

// Issues #236 and #284: the sizer is `total * row_height`, and a browser caps the
// height of an element (Chromium and WebKit at 33 554 428px, Firefox at about
// 17.9 million). The grid asks for at most 10 000 000px and maps the scroll
// position onto the rows above that: 312 500 rows at 32px are the last result
// drawn as it always was. The fixture answers with the real engine's rows but
// claims a total of its own, so the scroll mechanics of a very long result are
// tested without a very long result.

const LIMIT_ROWS = 312_500; // 10 000 000 / 32

async function open(page, total) {
  await page.addInitScript((total) => (window.__total = total), total);
  await page.goto("/tests/e2e/fixtures/grid-huge.html");
  await page.evaluate(() => window.__opengridReady);
  await page.waitForSelector("opengrid-grid td[data-row]");
}

/** What is drawn and where, in the viewport's terms. */
function view(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const viewport = root.querySelector('[part="viewport"]');
    const top = viewport.getBoundingClientRect().top;
    const bottom = top + viewport.clientHeight;
    const headBottom = root.querySelector("thead").getBoundingClientRect().bottom;
    const rows = [...root.querySelectorAll("tbody tr[aria-rowindex]")].map((tr) => {
      const box = tr.getBoundingClientRect();
      return {
        row: Number(tr.querySelector("td[data-col='0']").dataset.row),
        index: Number(tr.getAttribute("aria-rowindex")),
        top: box.top,
        bottom: box.bottom,
        transform: tr.style.transform,
      };
    });
    const shown = rows.filter((row) => row.bottom > headBottom + 0.5 && row.top < bottom - 0.5);
    const focus = root.activeElement;
    return {
      rows,
      shown,
      headBottom,
      bottom,
      scrollTop: viewport.scrollTop,
      scrollHeight: viewport.scrollHeight,
      maxScroll: viewport.scrollHeight - viewport.clientHeight,
      count: Number(root.querySelector("table").getAttribute("aria-rowcount")) - 1,
      focusRow: focus && focus.dataset.row !== undefined ? Number(focus.dataset.row) : null,
      focusTag: focus ? focus.tagName.toLowerCase() : null,
    };
  });
}

// -1 while nothing is drawn in sight (a scroll whose window has not arrived yet).
const firstShown = async (page) => (await shownRows(page))[0];
const lastShown = async (page) => (await shownRows(page)).at(-1);
async function shownRows(page) {
  const rows = (await view(page)).shown.map((row) => row.row).sort((a, b) => a - b);
  return rows.length ? rows : [-1];
}

const setScrollTop = (page, scrollTop) =>
  page.evaluate((scrollTop) => {
    const viewport = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]');
    viewport.scrollTop = scrollTop;
  }, scrollTop);

const scrollToEnd = (page) =>
  page.evaluate(() => {
    const viewport = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]');
    viewport.scrollTop = viewport.scrollHeight;
  });

/** The last row is drawn, whole, at the bottom edge of the viewport. */
async function expectLastRowAtTheBottom(page, total) {
  await expect.poll(async () => lastShown(page)).toBe(total - 1);
  const state = await view(page);
  const last = state.shown.find((row) => row.row === total - 1);
  expect(last.index).toBe(total + 1);
  expect(state.count).toBe(total);
  // To the pixel; the viewport's own height is an integer, its box is not
  // (WebKit lays out 160.5 where Chromium says 160).
  expect(last.bottom).toBeLessThanOrEqual(state.bottom + 1.5);
  expect(last.bottom).toBeGreaterThan(state.bottom - 2);
}

/** The first row is drawn right below the header. */
async function expectFirstRowAtTheTop(page) {
  await expect.poll(async () => firstShown(page)).toBe(0);
  const state = await view(page);
  const first = state.shown.find((row) => row.row === 0);
  expect(first.index).toBe(2);
  expect(Math.abs(first.top - state.headBottom)).toBeLessThan(1.5);
}

test("a result below the browser's element height is scrolled to its last row", async ({ page }) => {
  // 1 000 000 rows of 32px are 32 000 000px: past the grid's own limit of
  // 10 000 000px, below the browser's.
  await open(page, 1_000_000);
  await scrollToEnd(page);
  await expectLastRowAtTheBottom(page, 1_000_000);
});

test("a result above the browser's element height is scrolled to its last row too (#284)", async ({ page }) => {
  // 3 000 000 rows of 32px ask for 96 000 000px; Chromium stops at 33 554 428.
  await open(page, 3_000_000);
  await scrollToEnd(page);
  await expectLastRowAtTheBottom(page, 3_000_000);
  // The sizer never was more than the limit, and the rows after the viewport's
  // last position did not stretch it.
  const state = await view(page);
  expect(state.scrollHeight).toBeLessThan(10_001_000);
  // And back: the first row is the first row.
  await setScrollTop(page, 0);
  await expectFirstRowAtTheTop(page);
});

test("a result of exactly the limit and one row more both reach their ends (#284)", async ({ page }) => {
  await open(page, LIMIT_ROWS);
  // Exactly the limit: the sizer is the rows' height and nothing is shifted, as before.
  const exact = await view(page);
  expect(exact.scrollHeight - exact.maxScroll).toBeGreaterThan(0);
  await expectFirstRowAtTheTop(page);
  await scrollToEnd(page);
  await expectLastRowAtTheBottom(page, LIMIT_ROWS);
  // The first window is where it always was: row r at r * 32px.
  await setScrollTop(page, 0);
  await expectFirstRowAtTheTop(page);
  const rowsDrawn = (await view(page)).rows;
  expect(rowsDrawn.length).toBe(20);
  for (const row of rowsDrawn) {
    expect(row.transform).toBe(`translateY(${row.row * 32}px)`);
  }
});

test("a result one row past the limit is scaled and reaches its ends (#284)", async ({ page }) => {
  await open(page, LIMIT_ROWS + 1);
  await scrollToEnd(page);
  await expectLastRowAtTheBottom(page, LIMIT_ROWS + 1);
  await setScrollTop(page, 0);
  await expectFirstRowAtTheTop(page);
});

test("Ctrl+End, Ctrl+Home and the page keys go the whole way (#284)", async ({ page }) => {
  await open(page, 3_000_000);
  await page.locator("opengrid-grid td[data-row='0'][data-col='0']").click();

  await page.keyboard.press("Control+End");
  await expect.poll(async () => (await view(page)).focusRow).toBe(2_999_999);
  await expectLastRowAtTheBottom(page, 3_000_000);

  // PageUp from the end moves the focus by a page and keeps it in view.
  await page.keyboard.press("PageUp");
  await expect.poll(async () => (await view(page)).focusRow).toBeLessThan(2_999_999);
  const afterPageUp = (await view(page)).focusRow;
  expect(afterPageUp).toBeGreaterThan(2_999_990);
  await expect
    .poll(async () => {
      const state = await view(page);
      return state.shown.some((row) => row.row === afterPageUp);
    })
    .toBe(true);

  await page.keyboard.press("PageDown");
  await expect.poll(async () => (await view(page)).focusRow).toBe(2_999_999);

  // Ctrl+Home is the first cell of the whole grid, the header's.
  await page.keyboard.press("Control+Home");
  await expect.poll(async () => (await view(page)).focusTag).toBe("th");
  await expectFirstRowAtTheTop(page);

  // From there the arrow goes to row 0 and PageDown walks down by a page, the
  // focus staying in sight.
  await page.keyboard.press("ArrowDown");
  await expect.poll(async () => (await view(page)).focusRow).toBe(0);
  await page.keyboard.press("PageDown");
  await expect.poll(async () => (await view(page)).focusRow).toBeGreaterThan(0);
  const down = (await view(page)).focusRow;
  expect(down).toBeLessThan(10);
  await expect.poll(async () => (await view(page)).shown.some((row) => row.row === down)).toBe(true);
});

test("the arrow keys walk past the end of a window far from the top (#284)", async ({ page }) => {
  await open(page, 3_000_000);
  await page.locator("opengrid-grid td[data-row='0'][data-col='0']").click();
  await page.keyboard.press("Control+End");
  await expect.poll(async () => (await view(page)).focusRow).toBe(2_999_999);
  for (let step = 0; step < 40; step++) {
    await page.keyboard.press("ArrowUp");
  }
  await expect.poll(async () => (await view(page)).focusRow).toBe(2_999_959);
  const state = await view(page);
  expect(state.shown.some((row) => row.row === 2_999_959)).toBe(true);
  // The scroll area was not stretched by the rows far away.
  expect(state.scrollHeight).toBeLessThan(10_001_000);
});

test("the scroll bar maps onto the whole result and a wheel notch moves a few rows (#284)", async ({ page }) => {
  await open(page, 3_000_000);
  const { maxScroll } = await view(page);
  // The middle of the scroll bar is the middle of the result.
  await setScrollTop(page, Math.floor(maxScroll / 2));
  await expect.poll(async () => firstShown(page)).toBeGreaterThan(1_400_000);
  expect(await firstShown(page)).toBeLessThan(1_600_000);
  const start = await firstShown(page);

  // One notch of the wheel is 100px: three rows, not the thirty a proportional
  // map would make of it.
  await page.locator("opengrid-grid").hover();
  await page.mouse.wheel(0, 100);
  await expect.poll(async () => (await firstShown(page)) - start).toBeGreaterThanOrEqual(2);
  await expect.poll(async () => (await firstShown(page)) - start).toBeLessThanOrEqual(5);
});

test("dragging the scroll bar asks once per frame and keeps the focus (#284)", async ({ page }) => {
  await open(page, 3_000_000);
  // The focus stands on row 3.
  await page.locator("opengrid-grid td[data-row='3'][data-col='0']").click();
  expect((await view(page)).focusRow).toBe(3);
  const { maxScroll } = await view(page);
  const before = await page.evaluate(() => window.__queries);

  // A drag: many positions, one every 20ms.
  const steps = 30;
  for (let step = 1; step <= steps; step++) {
    await setScrollTop(page, Math.floor((maxScroll * step) / steps));
    await page.waitForTimeout(20);
  }
  // The focused row holds one of the pool's slots, so a window at the very end
  // is a row short (as it is below the limit); the drag is asked about, not that.
  await expect.poll(async () => lastShown(page)).toBeGreaterThan(2_999_990);
  const asked = (await page.evaluate(() => window.__queries)) - before;
  // At most one window per position, never a loop.
  expect(asked).toBeLessThanOrEqual(steps + 2);

  // Row 3 is far away and out of sight, yet it still holds the focus: its place
  // is parked outside the sizer, so it does not stretch the scroll area either.
  const state = await view(page);
  expect(state.focusRow).toBe(3);
  expect(state.shown.some((row) => row.row === 3)).toBe(false);
  expect(state.scrollHeight).toBeLessThan(10_001_000);

  // Close to the top again (by a jump, so the rows move with it), row 3's old
  // place is in sight but row 3 is not what the result shows there.
  await setScrollTop(page, 60_000);
  await setScrollTop(page, 100);
  await expect.poll(async () => firstShown(page)).toBeGreaterThan(10);
  expect((await view(page)).shown.some((row) => row.row === 3)).toBe(false);
  expect((await view(page)).focusRow).toBe(3);

  // Back to the top, the focused row is where it was.
  await setScrollTop(page, 0);
  await expectFirstRowAtTheTop(page);
  await expect.poll(async () => (await view(page)).shown.some((row) => row.row === 3)).toBe(true);
  expect((await view(page)).focusRow).toBe(3);
});
