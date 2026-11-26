import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// Issue #525: where am I. The focused cell's row and column header take a
// light tint; the footer says "Row 3 of 5,000 · customer", or a range's size
// and sums; a fling shows the rows in sight beside the scroll bar; a shadow
// marks each edge with columns past it.

async function open(page, search = "") {
  await page.goto(`/tests/e2e/fixtures/grid-orientation.html${search}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  await expect.poll(() => shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="0"]')?.textContent)).toBe("1");
}

const position = (page) => shadow(page, (root) => root.querySelector('[part="position"]').textContent);

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), {
    row,
    col,
  });
}

/** The background image of a cell's row and of a column's header. */
const tints = (page, row, cols) =>
  shadow(
    page,
    (root, { row, cols }) => ({
      row: getComputedStyle(root.querySelector(`td[data-row="${row}"]`).parentElement).backgroundImage,
      headers: cols.map((col) => getComputedStyle(root.querySelector(`thead th[data-col="${col}"]`)).backgroundImage),
    }),
    { row, cols },
  );

/** Whether each overlay shows, and where its box is beside the viewport's. */
const overlays = (page) =>
  shadow(page, (root) => {
    const viewport = root.querySelector('[part="viewport"]');
    const box = viewport.getBoundingClientRect();
    const read = (part) => {
      const node = root.querySelector(`[part="${part}"]`);
      const rect = node.getBoundingClientRect();
      return {
        shown: node.hasAttribute("data-shown") && getComputedStyle(node).visibility === "visible",
        hidden: node.getAttribute("aria-hidden"),
        left: Math.round(rect.left - box.left - viewport.clientLeft),
        right: Math.round(rect.right - box.left - viewport.clientLeft),
        top: Math.round(rect.top - box.top),
        height: Math.round(rect.height),
      };
    };
    return {
      width: viewport.clientWidth,
      height: viewport.clientHeight,
      start: read("edge-start"),
      end: read("edge-end"),
      bubble: { ...read("scroll-position"), text: root.querySelector('[part="scroll-position"]').textContent },
    };
  });

/**
 * Scrolls the viewport to `top` and answers, at that scroll's event, whether
 * the bubble shows and what it says — a programmatic scroll ends at once.
 */
const scrollTo = (page, top) =>
  shadow(
    page,
    (root, top) =>
      new Promise((resolve) => {
        const viewport = root.querySelector('[part="viewport"]');
        viewport.addEventListener(
          "scroll",
          () => {
            const bubble = root.querySelector('[part="scroll-position"]');
            const box = viewport.getBoundingClientRect();
            const rect = bubble.getBoundingClientRect();
            resolve({
              shown: bubble.hasAttribute("data-shown"),
              text: bubble.textContent,
              right: Math.round(rect.right - box.left - viewport.clientLeft),
              width: viewport.clientWidth,
            });
          },
          { once: true },
        );
        viewport.scrollTop = top;
      }),
    top,
  );

test.describe("the crosshair and the position", () => {
  test("the focused cell's row and header are tinted, and the footer says where it is", async ({ page }) => {
    await open(page);
    expect(await position(page)).toBe("");
    await focusCell(page, 2, 1);
    await expect.poll(() => position(page)).toBe("Row 3 of 5,000 · customer");
    const tinted = await tints(page, 2, [1, 2]);
    expect(tinted.row).toContain("gradient");
    expect(tinted.headers[0]).toContain("gradient");
    expect(tinted.headers[1]).toBe("none");
    expect(await tints(page, 1, [])).toMatchObject({ row: "none" });

    // The crosshair follows the focus, and leaves with it.
    await page.keyboard.press("ArrowRight");
    await expect.poll(() => position(page)).toBe("Row 3 of 5,000 · city");
    expect((await tints(page, 2, [1, 2])).headers).toEqual(["none", expect.stringContaining("gradient")]);
    await page.keyboard.press("Tab");
    expect(await tints(page, 2, [2])).toEqual({ row: "none", headers: ["none"] });
  });

  test("the position is not a live region", async ({ page }) => {
    await open(page);
    const node = await shadow(page, (root) => {
      const node = root.querySelector('[part="position"]');
      return { live: node.getAttribute("aria-live"), role: node.getAttribute("role"), inStatus: !!node.closest('[role="status"]') };
    });
    expect(node).toEqual({ live: null, role: null, inStatus: false });
  });
});

test.describe("a range", () => {
  test("says its size and sums, in the column's format where the numbers share one", async ({ page }) => {
    await open(page);
    await focusCell(page, 0, 4);
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowDown");
    // Prices 2.50, 3.75 and 5.00, in the page's currency format.
    await expect.poll(() => position(page)).toBe("3 by 1 cells · Count: 3 · Sum: $11.25 · Average: $3.75");
    // With the seats beside them the numbers share no column: plain.
    await page.keyboard.press("Shift+ArrowLeft");
    await expect.poll(() => position(page)).toBe("3 by 2 cells · Count: 6 · Sum: 23.25 · Average: 3.88");
    // A text counts and adds nothing.
    await page.keyboard.press("Shift+ArrowLeft");
    await expect.poll(() => position(page)).toBe("3 by 3 cells · Count: 9 · Sum: 23.25 · Average: 3.88");
    // A NULL is no value: note is empty for id 4.
    await focusCell(page, 3, 5);
    await expect.poll(() => position(page)).toBe("Row 4 of 5,000 · note");
    await page.keyboard.press("Shift+ArrowDown");
    await expect.poll(() => position(page)).toBe("2 by 1 cells · Count: 1");
  });

  test("past the loaded rows it is summed by the source, in one query", async ({ page }) => {
    await open(page);
    await focusCell(page, 0, 3);
    const rowHeight = await shadow(page, (root) => root.querySelector('td[data-row="0"]').getBoundingClientRect().height);
    await scrollTo(page, Math.round(rowHeight * 2000));
    await expect.poll(() => shadow(page, (root) => !!root.querySelector('td[data-row="2005"][data-col="3"]'))).toBe(true);
    await page.evaluate(() => {
      window.__queries = [];
      window.__delay = 800;
    });
    const cell = await shadow(page, (root) => {
      const rect = root.querySelector('td[data-row="2005"][data-col="3"]').getBoundingClientRect();
      return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
    });
    await page.keyboard.down("Shift");
    await page.mouse.click(cell.x, cell.y);
    await page.keyboard.up("Shift");
    // Until the source answers, the size alone.
    await expect.poll(() => position(page)).toBe("2006 by 1 cells");
    const expected = await page.evaluate(() => {
      const seats = window.__rows.slice(0, 2006).map((row) => row.seats);
      const sum = seats.reduce((a, b) => a + b, 0);
      return { sum, average: (Math.round((sum / seats.length) * 100) / 100).toFixed(2) };
    });
    await expect
      .poll(() => position(page), { timeout: 10_000 })
      .toBe(`2006 by 1 cells · Count: 2,006 · Sum: ${expected.sum} · Average: ${expected.average}`);
    // The range's column, and the one the order needs.
    const asked = await page.evaluate(() => window.__queries.filter((query) => query.select[0] === "seats"));
    expect(asked).toHaveLength(1);
    expect(asked[0]).toMatchObject({ select: ["seats", "id"], offset: 0, limit: 2006 });
  });

  test("has no axe violations", async ({ page }) => {
    await open(page);
    await focusCell(page, 0, 3);
    await page.keyboard.press("Shift+ArrowDown");
    await expect.poll(() => position(page)).toContain("Count");
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });
});

test.describe("the bubble while scrolling", () => {
  test("a long scroll shows the rows in sight beside the scroll bar, and it goes", async ({ page }) => {
    await open(page);
    const before = await overlays(page);
    expect(before.bubble).toMatchObject({ shown: false, hidden: "true" });
    // Not for a step: half a viewport.
    expect((await scrollTo(page, Math.round(before.height / 2))).shown).toBe(false);
    await page.waitForTimeout(400);

    const rowHeight = await shadow(page, (root) => root.querySelector('td[data-row="0"]').getBoundingClientRect().height);
    const during = await scrollTo(page, Math.round(rowHeight * 1200));
    expect(during.shown).toBe(true);
    expect(during.text).toMatch(/^1,20\d–1,2\d\d$/);
    // Beside the scroll bar, at the end of the viewport.
    expect(during.right).toBeLessThanOrEqual(during.width);
    expect(during.right).toBeGreaterThan(during.width - 40);
    await expect.poll(async () => (await overlays(page)).bubble.shown, { timeout: 3000 }).toBe(false);
  });

  test("without motion it comes and goes without a fade", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await open(page);
    const duration = await shadow(
      page,
      (root) => getComputedStyle(root.querySelector('[part="scroll-position"]')).transitionDuration,
    );
    expect(duration.split(",").every((value) => parseFloat(value) <= 0.001)).toBe(true);
  });
});

test.describe("the edges", () => {
  // `?keynone`: on the narrow project the key column would otherwise be pinned.
  test("a shadow marks each edge with columns past it", async ({ page }) => {
    await open(page, "?keynone");
    const start = await overlays(page);
    expect(start.start.shown).toBe(false);
    expect(start.end).toMatchObject({ shown: true, hidden: "true", right: start.width, top: 0, height: start.height });

    await shadow(page, (root) => {
      const viewport = root.querySelector('[part="viewport"]');
      viewport.scrollLeft = viewport.scrollWidth;
    });
    await expect.poll(async () => (await overlays(page)).start.shown).toBe(true);
    const end = await overlays(page);
    expect(end.end.shown).toBe(false);
    expect(end.start.left).toBe(0);
  });

  test("right to left the start is the right edge", async ({ page }) => {
    await open(page, "?rtl&keynone");
    const start = await overlays(page);
    expect(start.start.shown).toBe(false);
    expect(start.end).toMatchObject({ shown: true, left: 0 });
    await shadow(page, (root) => {
      const viewport = root.querySelector('[part="viewport"]');
      viewport.scrollLeft = -viewport.scrollWidth;
    });
    await expect.poll(async () => (await overlays(page)).start.shown).toBe(true);
    const end = await overlays(page);
    expect(end.end.shown).toBe(false);
    expect(end.start.right).toBe(end.width);
  });

  test("beside pinned columns the shadow stands where they end", async ({ page }) => {
    await open(page, "?pin");
    await shadow(page, (root) => {
      root.querySelector('[part="viewport"]').scrollLeft = 200;
    });
    await expect.poll(async () => (await overlays(page)).start.shown).toBe(true);
    const pinned = await shadow(page, (root) => Math.round(root.querySelector('thead th[data-col="0"]').getBoundingClientRect().width));
    const { start } = await overlays(page);
    expect(Math.abs(start.left - pinned)).toBeLessThanOrEqual(1);
  });
});

test.describe("pinning on narrow grids", () => {
  /** Which header cells stay in place, by column name, in the order they stand. */
  const pinned = (page) =>
    shadow(page, (root) =>
      [...root.querySelectorAll("thead th[data-col]")]
        .filter((th) => getComputedStyle(th).position === "sticky")
        .map((th) => th.textContent.replace(/[^a-z]/g, "")),
    );
  const order = (page) =>
    shadow(page, (root) => [...root.querySelectorAll("thead th[data-col]")].map((th) => th.textContent.replace(/[^a-z]/g, "")));
  const setWidth = (page, width) =>
    page.evaluate((width) => {
      document.querySelector("opengrid-grid").style.width = `${width}px`;
    }, width);

  test("under 560 px the row-key column is pinned at the start, and lets go when wider", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?width=480");
    await expect.poll(() => pinned(page)).toEqual(["id"]);
    await setWidth(page, 640);
    await expect.poll(() => pinned(page)).toEqual([]);
    await setWidth(page, 500);
    await expect.poll(() => pinned(page)).toEqual(["id"]);
    // Nothing changed the order, so nothing was asked again.
    expect(await page.evaluate(() => window.__queries.length)).toBe(1);
  });

  test("pinNarrow: none keeps the key column from it", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?width=480&keynone");
    await page.waitForTimeout(300);
    expect(await pinned(page)).toEqual([]);
  });

  test("a page's pin at every width holds on narrow grids too, and the default steps back", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?width=480");
    await expect.poll(() => pinned(page)).toEqual(["id"]);
    const width = { width: 180 };
    await page.evaluate(
      (width) =>
        window.__module.set_columns(document.querySelector("opengrid-grid"), {
          id: width,
          customer: width,
          city: { ...width, pin: "end" },
          seats: width,
          price: width,
          note: width,
        }),
      width,
    );
    await expect.poll(() => pinned(page)).toEqual(["city"]);
  });

  test("a page's pinNarrow moves the column to its end while narrow, and the default steps back", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?width=480&customerend");
    await expect.poll(() => pinned(page)).toEqual(["customer"]);
    expect(await order(page)).toEqual(["id", "city", "seats", "price", "note", "customer"]);
    await setWidth(page, 640);
    await expect.poll(() => pinned(page)).toEqual([]);
    await expect.poll(() => order(page)).toEqual(["id", "customer", "city", "seats", "price", "note"]);
  });

  test("the reader pins a column for narrow screens only, and the view says so", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    // The key column keeps out of it: two 180 px pins would be too much for 480 px.
    await open(page, "?width=480&menu&keynone");
    await shadow(page, (root) => root.querySelector('thead th[data-col="3"]').focus());
    await page.keyboard.press("Alt+ArrowDown");
    const item = (label) =>
      shadow(
        page,
        (root, label) => {
          const node = [...root.querySelectorAll('[part="column-menu"] [role="menuitemradio"]')].find(
            (entry) => entry.textContent === label,
          );
          return node && { checked: node.getAttribute("aria-checked") };
        },
        label,
      );
    await expect.poll(() => item("Only on narrow screens")).toEqual({ checked: "false" });
    await shadow(page, (root) =>
      [...root.querySelectorAll('[part="column-menu"] [role="menuitemradio"]')]
        .find((entry) => entry.textContent === "Only on narrow screens")
        .click(),
    );
    await expect.poll(() => pinned(page)).toEqual(["seats"]);
    await expect.poll(() => page.evaluate(() => window.__views.at(-1)?.columns.pinnedNarrow)).toEqual({ seats: "start" });
    await setWidth(page, 640);
    await expect.poll(() => pinned(page)).toEqual([]);

    // The view brings it back, and "Not pinned" lets go at every width.
    const view = await page.evaluate(() => window.__module.get_view(document.querySelector("opengrid-grid")));
    expect(view.columns.pinnedNarrow).toEqual({ seats: "start" });
    await setWidth(page, 480);
    await expect.poll(() => pinned(page)).toEqual(["seats"]);
  });

  test("has no axe violations with a narrow pin and the column menu open", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?width=480&menu");
    await expect.poll(() => pinned(page)).toEqual(["id"]);
    await shadow(page, (root) => root.querySelector('thead th[data-col="3"]').focus());
    await page.keyboard.press("Alt+ArrowDown");
    await expect.poll(() => shadow(page, (root) => !!root.querySelector('[part="column-menu"] [role="menuitemradio"]'))).toBe(true);
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });

  test("too narrow for the pins, they let go as any pin does", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?width=320&customerend");
    // id is not pinned by default beside the page's narrow pin; customer is 180 of 320.
    await expect.poll(() => pinned(page)).toEqual([]);
    await expect.poll(() => status(page)).toMatch(/Too narrow for the pinned columns/);
  });
});

test.describe("scrolling on a phone", () => {
  test("a swipe past the left edge is not the back gesture, and the page scrolls on past the grid's end", async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 });
    await open(page, "?page");
    const overscroll = await shadow(page, (root) => {
      const style = getComputedStyle(root.querySelector('[part="viewport"]'));
      return [style.overscrollBehaviorX, style.overscrollBehaviorY];
    });
    expect(overscroll).toEqual(["contain", "auto"]);

    // At the end of the grid a wheel turn leaves it: the page scrolls, the grid does not hold it.
    await shadow(page, (root) => {
      const viewport = root.querySelector('[part="viewport"]');
      viewport.scrollTop = viewport.scrollHeight;
    });
    const box = await page.locator("opengrid-grid").boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await expect.poll(() => shadow(page, (root) => {
      const viewport = root.querySelector('[part="viewport"]');
      return viewport.scrollTop + viewport.clientHeight >= viewport.scrollHeight - 1;
    })).toBe(true);
    // The first turn may still be the grid's own (a browser latches a gesture to the scroller it began in).
    await expect
      .poll(async () => {
        await page.mouse.wheel(0, 300);
        return page.evaluate(() => window.scrollY);
      })
      .toBeGreaterThan(0);
  });
});
