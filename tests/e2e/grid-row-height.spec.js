import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` configurable row height (plan point 17 review).
//
// The fixture sets `--og-row-height: 48px` on the host (overriding the shadow
// default of 32px). This spec proves that the resolved value inherits into the
// shadow tree and drives the sizer, the row offsets and the scroll→window math.

/** Facts about the rendered shadow root. */
async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const viewport = root.querySelector('[part="viewport"]');
    const tbody = root.querySelector("tbody");
    return {
      rowHeight: getComputedStyle(viewport)
        .getPropertyValue("--og-row-height")
        .trim(),
      tbodyStyle: tbody.getAttribute("style"),
      scrollHeight: viewport.scrollHeight,
      clientHeight: viewport.clientHeight,
    };
  });
}

/** The inline style of the `<tr>` rendering logical row `row`. */
async function rowStyle(page, row) {
  return page.evaluate((row) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return root
      .querySelector(`td[data-row="${row}"]`)
      ?.closest("tr")
      ?.getAttribute("style");
  }, row);
}

/** The `aria-rowindex` of the `<tr>` rendering logical row `row`. */
async function rowindexFor(page, row) {
  return page.evaluate((row) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return (
      root
        .querySelector(`td[data-row="${row}"]`)
        ?.closest("tr")
        ?.getAttribute("aria-rowindex") ?? null
    );
  }, row);
}

/** The inner focused element, or null. */
async function activeCell(page) {
  return page.evaluate(() => {
    const element = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!element) return null;
    return { tag: element.tagName.toLowerCase(), row: element.getAttribute("data-row") };
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-row-height.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("the host override inherits and sizes the sizer and rows", async ({
  page,
}) => {
  const rendered = await facts(page);
  // The configured value wins over the shadow default and inherits.
  expect(rendered.rowHeight).toBe("48px");
  // 200 rows × 48px, plus the one sticky header row at 48px.
  expect(rendered.tbodyStyle).toContain("height: 9600px;");
  expect(rendered.scrollHeight).toBe(9600 + 48);
  // Row 2 sits at 2 × 48px.
  expect(await rowStyle(page, 2)).toContain("translateY(96px)");
  expect(await rowindexFor(page, 2)).toBe("4");
});

test("the scroll→window math follows the configured row height", async ({
  page,
}) => {
  // 960px = 20 rows × 48px; with the 6-row overscan the window starts at 14.
  await page.evaluate(() => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="viewport"]').scrollTop = 960;
  });
  await expect.poll(() => rowindexFor(page, 20)).toBe("22");
  expect(await rowStyle(page, 20)).toContain("translateY(960px)");
});

test("PageDown steps by a viewport of 48px rows", async ({ page }) => {
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot
      .querySelector('th[data-col="0"]')
      .focus();
  });
  // The 160px viewport shows three 48px rows, so the first page lands on row 2.
  await page.keyboard.press("PageDown");
  await expect.poll(() => activeCell(page)).toMatchObject({ row: "2" });
});

test("has no axe violations", async ({ page }) => {
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
test("a grid nobody has themed wears the system colours", async ({ page }) => {
  // The defaults are `Canvas`, `CanvasText` and `Highlight` — not a palette of
  // our own. Two reasons, and both are accessibility rather than taste: a grid
  // with no page CSS stays legible and in the right light or dark, and
  // `forced-colors` keeps winning. The fixture sets only --og-row-height, so
  // every colour below is a default.
  const system = await page.evaluate(() => {
    const probe = document.createElement("div");
    probe.style.cssText = "background: Canvas; color: CanvasText";
    document.body.append(probe);
    const wanted = getComputedStyle(probe);
    const canvas = wanted.backgroundColor;
    const canvasText = wanted.color;
    probe.remove();

    const root = document.querySelector("opengrid-grid").shadowRoot;
    const row = getComputedStyle(root.querySelector("tbody tr"));
    const layout = getComputedStyle(root.querySelector('[part="layout"]'));
    return {
      rowBackground: row.backgroundColor,
      canvas,
      ink: layout.color,
      canvasText,
    };
  });

  expect(system.rowBackground).toBe(system.canvas);
  expect(system.ink).toBe(system.canvasText);
});
