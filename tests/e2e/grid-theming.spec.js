import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` theming (plan point 20, spezifikation/08-rendering.md
// §CSS-Architektur).
//
// The fixture themes the grid the way a page is meant to: custom properties on
// the host and `::part` rules — never a reach into the shadow DOM structure. The
// tests check that this actually arrives, and that the three guarantees the
// theme must not be able to break hold: the focus ring stays visible (also at
// 400% zoom), `forced-colors` keeps the grid legible, and
// `prefers-reduced-motion` wins over a theme that animates a part.

/** Computed style values of one element inside the shadow root. */
async function computed(page, selector, properties) {
  return page.evaluate(
    ({ selector, properties }) => {
      const node = document.querySelector("opengrid-grid").shadowRoot.querySelector(selector);
      const style = getComputedStyle(node);
      return Object.fromEntries(properties.map((name) => [name, style.getPropertyValue(name)]));
    },
    { selector, properties },
  );
}

/** Focuses an element inside the shadow root. */
async function focusIn(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-theming.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("custom properties on the host reach the shadow tree", async ({ page }) => {
  // --og-header-height only moves the header: it sits outside the <tbody>
  // sizer, so the row height (and the window math) is untouched.
  expect(await computed(page, "thead th", ["height"])).toEqual({ height: "48px" });
  expect(await computed(page, "tbody td", ["height"])).toEqual({ height: "32px" });

  // Two rule colours, because they separate two different things: --og-line
  // draws between rows, --og-line-strong between regions.
  expect(
    await computed(page, "tbody td", ["border-bottom-color", "border-bottom-width"]),
  ).toEqual({ "border-bottom-color": "rgb(0, 0, 255)", "border-bottom-width": "1px" });
  expect(await computed(page, '[part="filter"]', ["border-bottom-color"])).toEqual({
    "border-bottom-color": "rgb(0, 128, 0)",
  });
});

test("the grid computes the accented colours from the accent it is given", async ({
  page,
}) => {
  // The point of the split (point 57): a page picks one accent and the grid
  // works out what a selected row and a hover look like against *its* surface.
  // The fixture sets --og-accent and nothing else accented.
  const tokens = await page.evaluate(() => {
    const style = getComputedStyle(document.querySelector("opengrid-grid"));
    return Object.fromEntries(
      ["--og-accent", "--og-accent-ink", "--og-selected", "--og-hover"].map((name) => [
        name,
        style.getPropertyValue(name).trim(),
      ]),
    );
  });
  expect(tokens["--og-accent"]).toBe("rgb(204, 0, 102)");
  // Computed, not empty, and not simply the accent repeated.
  for (const name of ["--og-accent-ink", "--og-selected", "--og-hover"]) {
    expect(tokens[name]).not.toBe("");
    expect(tokens[name]).not.toBe(tokens["--og-accent"]);
  }
});

test("every token a page may set arrives in the shadow tree", async ({ page }) => {
  // The promise of point 57 is the whole list, not a sample of it: a page sets
  // these and the grid uses them. A token that is declared and then ignored is
  // a promise the element does not keep — the Rust test guards that side, this
  // one guards that the value actually travels.
  const TOKENS = {
    "--og-font": "cursive",
    "--og-font-size": "19px",
    "--og-surface": "rgb(1, 2, 3)",
    "--og-surface-2": "rgb(4, 5, 6)",
    "--og-ink": "rgb(7, 8, 9)",
    "--og-ink-muted": "rgb(10, 11, 12)",
    "--og-line": "rgb(13, 14, 15)",
    "--og-line-strong": "rgb(16, 17, 18)",
    "--og-accent": "rgb(19, 20, 21)",
    "--og-radius": "7px",
    "--og-pad": "13px",
    "--og-focus-width": "5px",
    "--og-row-height": "41px",
    "--og-header-height": "43px",
    "--og-filter-height": "47px",
    "--og-status-height": "29px",
  };
  const arrived = await page.evaluate((tokens) => {
    const host = document.querySelector("opengrid-grid");
    for (const [name, value] of Object.entries(tokens)) host.style.setProperty(name, value);
    const root = host.shadowRoot;
    // `outline-width` only has the themed value while the cell is focused —
    // unfocused, the browser reports `medium` and the token looks lost.
    root.querySelector('td[data-row="0"][data-col="0"]').focus();
    const of = (selector, property) =>
      getComputedStyle(root.querySelector(selector)).getPropertyValue(property).trim();
    return {
      "--og-font": of("tbody td", "font-family"),
      "--og-font-size": of("tbody td", "font-size"),
      "--og-surface": of("tbody tr", "background-color"),
      "--og-surface-2": of("thead th", "background-color"),
      "--og-ink": of('[part="layout"]', "color"),
      "--og-ink-muted": of('[part="status"]', "color"),
      "--og-line": of("tbody td", "border-bottom-color"),
      "--og-line-strong": of('[part="filter"]', "border-bottom-color"),
      "--og-accent": getComputedStyle(host).getPropertyValue("--og-accent").trim(),
      "--og-radius": of('[part="filter"] select', "border-radius"),
      "--og-pad": of("tbody td", "padding-left"),
      "--og-focus-width": of('td[data-row="0"][data-col="0"]', "outline-width"),
      "--og-row-height": of("tbody td", "height"),
      "--og-header-height": of("thead th", "height"),
      "--og-filter-height": of('[part="filter"]', "height"),
      "--og-status-height": of('[part="status"]', "min-height"),
    };
  }, TOKENS);

  expect(arrived).toEqual({
    ...TOKENS,
    // `--og-radius` is capped for the small controls (`min(radius, 8px)`), and
    // 7px is under the cap, so it arrives unchanged.
    "--og-radius": "7px",
  });
});

test("a selected row is more than a tint", async ({ page }) => {
  // Colour alone would be 1.4.1, so the row also carries an inset accent bar.
  // Before point 57 a selected row was announced and looked like every other.
  await focusIn(page, 'td[data-row="0"][data-col="0"]');
  await page.keyboard.press(" ");
  await page.waitForFunction(
    () =>
      !!document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('tbody tr[data-selected="true"]'),
  );
  const row = await computed(page, 'tbody tr[data-selected="true"]', [
    "background-color",
    "box-shadow",
  ]);
  expect(row["box-shadow"]).toContain("inset");
  expect(row["background-color"]).not.toBe("rgba(0, 0, 0, 0)");
});

test("::part reaches header, row and cell", async ({ page }) => {
  expect(await computed(page, "thead th", ["font-variant-caps"])).toEqual({
    "font-variant-caps": "small-caps",
  });
  expect(await computed(page, "tbody td", ["font-style"])).toEqual({
    "font-style": "italic",
  });
  // `::part(row)` is themed too — the fixture gives it a transition, which the
  // reduced-motion test then switches off.
  expect(await computed(page, "tbody tr", ["transition-property"])).toEqual({
    "transition-property": "transform",
  });
});

test("the focus ring is drawn inside the cell, in the themed width", async ({ page }) => {
  await focusIn(page, 'td[data-row="0"][data-col="0"]');

  const ring = await computed(page, 'td[data-row="0"][data-col="0"]', [
    "outline-width",
    "outline-style",
    "outline-offset",
  ]);
  expect(ring["outline-width"]).toBe("4px");
  expect(ring["outline-style"]).toBe("solid");
  // Negative offset: the ring is inside the cell, so the scroll container cannot
  // clip it at the edge of the viewport — which is where a focused cell is.
  expect(ring["outline-offset"]).toBe("-4px");
});

test("the focus ring stays visible and unclipped at 400% zoom", async ({ page }) => {
  // 400% zoom of a 1280x1024 window is a 320x256 CSS-pixel viewport (WCAG 1.4.10).
  await page.setViewportSize({ width: 320, height: 256 });
  // At this size the fixture's heading alone fills the window, so bring the grid
  // on screen first — the question here is what the grid does once it is.
  await page.locator("opengrid-grid").scrollIntoViewIfNeeded();
  await focusIn(page, 'th[data-col="0"]');
  await page.keyboard.press("ArrowDown");

  const focus = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const cell = root.activeElement;
    const viewport = root.querySelector('[part="viewport"]').getBoundingClientRect();
    const box = cell.getBoundingClientRect();
    return {
      row: cell.getAttribute("data-row"),
      width: getComputedStyle(cell).outlineWidth,
      // The whole cell — and with it its inset ring — is inside the scroll
      // container and inside the window.
      insideViewport:
        box.top >= viewport.top - 0.5 &&
        box.bottom <= viewport.bottom + 0.5 &&
        box.left >= viewport.left - 0.5 &&
        box.right <= viewport.right + 0.5,
      insideWindow: box.top >= 0 && box.bottom <= window.innerHeight,
      // Reflow: no horizontal scrolling of the page at 320px.
      documentFits: document.documentElement.scrollWidth <= window.innerWidth,
    };
  });

  expect(focus).toMatchObject({
    row: "0",
    width: "4px",
    insideViewport: true,
    insideWindow: true,
    documentFits: true,
  });

  // The baseline holds the rest of the reflow: the filter row scrolls instead of
  // pushing the grid wider, and the columns squeeze rather than overflow.
  await expect(page.locator("opengrid-grid")).toHaveScreenshot("grid-zoom-400.png");
});

test("prefers-reduced-motion beats a theme that animates a part", async ({ page }) => {
  // The fixture animates `::part(row)` with a 2s transition …
  expect(await computed(page, "tbody tr", ["transition-duration"])).toEqual({
    "transition-duration": "2s",
  });

  // … and the shadow tree's `!important` rule switches it off. Between shadow
  // trees an important declaration from the inner tree wins over the page, so
  // the guarantee does not depend on the theme's cooperation.
  await page.emulateMedia({ reducedMotion: "reduce" });
  const reduced = await computed(page, "tbody tr", [
    "transition-duration",
    "animation-duration",
  ]);
  expect(Number.parseFloat(reduced["transition-duration"])).toBeLessThan(0.001);
  expect(Number.parseFloat(reduced["animation-duration"])).toBeLessThan(0.001);
  expect(await computed(page, '[part="viewport"]', ["scroll-behavior"])).toEqual({
    "scroll-behavior": "auto",
  });
});

test("the filter controls meet the minimum target size", async ({ page }) => {
  // WCAG 2.2 §2.5.8: 24x24 CSS pixels. The column list is a disclosure
  // (point 36), so it is opened first — a control nobody can see is not a
  // target, but every one that is shown has to be big enough to hit.
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="columns-toggle"]')
      ?.click(),
  );
  const sizes = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll('[part="filter"] select, [part="filter"] input, [part="filter"] button')]
      .filter((control) => control.getBoundingClientRect().width > 0)
      .map((control) => {
        const box = control.getBoundingClientRect();
        return { width: Math.round(box.width), height: Math.round(box.height) };
      });
  });
  expect(sizes.length).toBeGreaterThan(0);
  for (const size of sizes) {
    expect(size.height).toBeGreaterThanOrEqual(24);
    expect(size.width).toBeGreaterThanOrEqual(24);
  }
});

test("matches the themed baseline", async ({ page }) => {
  await focusIn(page, 'td[data-row="0"][data-col="0"]');
  await expect(page.locator("opengrid-grid")).toHaveScreenshot("grid-theming.png");
});

// The context is created in forced colors, so the grid renders that way from the
// first paint. Switching mid-life with `emulateMedia` was flaky: the computed
// style reported the forced palette while the sticky header still showed the
// themed blue rule in the screenshot.
test.describe("forced colors", () => {
  test.use({ forcedColors: "active" });

  test("a forced palette overrides the theme's colours", async ({ page }) => {
    // The theme asks for blue rules; the user's palette wins, for the header as
    // well as the body — nothing of the grid is drawn in a colour of its own.
    for (const selector of ["thead th", "tbody td", '[part="filter"]']) {
      expect(await computed(page, selector, ["border-bottom-color"])).not.toEqual({
        "border-bottom-color": "rgb(0, 0, 255)",
      });
    }
  });

  test("no computed colour survives as a mix", async ({ page }) => {
    // A `color-mix` of two system colours resolves unpredictably once a forced
    // palette is active, so every computed token is reset to a system colour.
    const mixed = await page.evaluate(() => {
      const style = getComputedStyle(document.querySelector("opengrid-grid"));
      return ["--og-accent-ink", "--og-selected", "--og-hover", "--og-ink-muted",
              "--og-line", "--og-line-strong"]
        .map((name) => [name, style.getPropertyValue(name).trim()])
        .filter(([, value]) => value.includes("color-mix"));
    });
    expect(mixed).toEqual([]);
  });

  test("matches the forced-colors baseline", async ({ page }) => {
    await focusIn(page, 'td[data-row="0"][data-col="0"]');
    await expect(page.locator("opengrid-grid")).toHaveScreenshot("grid-forced-colors.png");
  });

  test("has no axe violations", async ({ page }) => {
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
});
