import { test, expect } from "@playwright/test";

// Density (plan point 58): `density` switches row height, cell padding and font
// size in one step.
//
// The interesting test is not that it looks different — it is that the
// virtualization still adds up afterwards. The row height is the virtualization
// contract: the sizer's height, every row's `translateY` and the window the
// provider is asked for are all derived from it, and the element resolves it
// from the computed style. Resolving it once at connect is not enough, and a
// grid that does not re-resolve it draws rows at offsets that no longer match
// their slots — which looks like a rendering glitch and is a data error.

const DENSITIES = {
  compact: { rowHeight: 34, pad: 10 },
  normal: { rowHeight: 42, pad: 12 },
  comfortable: { rowHeight: 50, pad: 16 },
};

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-density.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

/** Everything the window math depends on, read from the live DOM. */
async function geometry(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const rows = [...root.querySelectorAll("tbody tr")];
    const cell = root.querySelector("tbody td");
    const sizer = root.querySelector("tbody");
    // `data-row` sits on the cells, not on the `<tr>` — the row carries
    // `aria-rowindex` and the transform. Reading it off the `<tr>` silently
    // yields NaN and turns the offset check below into a loop over nothing.
    const read = (row) => ({
      row: Number(row.querySelector("td[data-row]")?.dataset.row ?? NaN),
      // `translateY(Npx)` — the row's position inside the sizer.
      top: Number(/translateY\(([-\d.]+)px\)/.exec(row.style.transform)?.[1] ?? NaN),
    });
    return {
      rowHeight: Math.round(cell.getBoundingClientRect().height),
      pad: getComputedStyle(cell).paddingLeft,
      fontSize: getComputedStyle(cell).fontSize,
      sizerHeight: Number(/height: ([\d.]+)px/.exec(sizer.getAttribute("style"))?.[1] ?? NaN),
      total: Number(root.querySelector("table").getAttribute("aria-rowcount")) - 1,
      placed: rows.map(read).filter((r) => Number.isFinite(r.row) && Number.isFinite(r.top)),
    };
  });
}

test("a grid without the attribute is a normal one", async ({ page }) => {
  // There is no fourth, nameless density: `normal` is declared on `:host`
  // itself, so absent and `density="normal"` are the same grid.
  const bare = await geometry(page);
  expect(bare.rowHeight).toBe(DENSITIES.normal.rowHeight);

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("density", "normal"),
  );
  expect((await geometry(page)).rowHeight).toBe(DENSITIES.normal.rowHeight);

  // An unknown value falls back to normal rather than to nothing.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("density", "roomy"),
  );
  expect((await geometry(page)).rowHeight).toBe(DENSITIES.normal.rowHeight);
});

for (const [density, expected] of Object.entries(DENSITIES)) {
  test(`density="${density}" keeps the window math consistent`, async ({ page }) => {
    await page.evaluate((value) => {
      document.querySelector("opengrid-grid").setAttribute("density", value);
    }, density);
    // Let the re-resolved height reach the DOM.
    await page.waitForFunction(
      (height) => {
        const root = document.querySelector("opengrid-grid").shadowRoot;
        return Math.round(root.querySelector("tbody td").getBoundingClientRect().height) === height;
      },
      expected.rowHeight,
    );

    const g = await geometry(page);
    expect(g.rowHeight).toBe(expected.rowHeight);
    expect(g.pad).toBe(`${expected.pad}px`);

    // The three numbers that have to agree. This is the assertion that bites:
    // without re-resolving the row height, the sizer keeps the old total and
    // every row sits at an offset computed from a height it no longer has.
    expect(g.sizerHeight).toBe(g.total * expected.rowHeight);
    // Guard against a vacuous loop: if `placed` were empty the offsets below
    // would assert nothing at all.
    expect(g.placed.length).toBeGreaterThan(0);
    for (const { row, top } of g.placed) {
      expect(top).toBe(row * expected.rowHeight);
    }
  });
}

test("scrolling after a density change lands on the right row", async ({ page }) => {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("density", "compact"),
  );
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return Math.round(root.querySelector("tbody td").getBoundingClientRect().height) === 34;
  });

  // Scroll to exactly row 50 in the *new* height and check that row 50 is what
  // sits at the top of the viewport.
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('[part="viewport"]').scrollTop = 50 * 34;
  });
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-row="50"]'),
  );

  const at = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const viewport = root.querySelector('[part="viewport"]').getBoundingClientRect();
    const header = root.querySelector("thead").getBoundingClientRect();
    const row = root.querySelector('td[data-row="50"]').closest("tr");
    const box = row.getBoundingClientRect();
    return {
      // Row 50 is the first one under the sticky header, within a pixel.
      atTop: Math.abs(box.top - header.bottom) <= 1,
      inside: box.top >= viewport.top - 1,
      text: row.querySelector('td[data-row="50"]').textContent.trim(),
    };
  });
  expect(at.atTop).toBe(true);
  expect(at.inside).toBe(true);
  // 200 rows of ids 1..200, so the logical row 50 is id 51.
  expect(at.text).toBe("51");
});

test("every control stays hittable in every density", async ({ page }) => {
  // WCAG 2.2 §2.5.8: 24x24 CSS pixels. Compact is the one that could fall
  // through, and its 34px row leaves room — but the controls inside it are
  // what has to be measured, not the row.
  for (const density of Object.keys(DENSITIES)) {
    await page.evaluate((value) => {
      document.querySelector("opengrid-grid").setAttribute("density", value);
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('[part="columns-toggle"]')
        ?.click();
    }, density);

    const sizes = await page.evaluate(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      return [
        ...root.querySelectorAll(
          '[part="filter"] select, [part="filter"] input, [part="filter"] button',
        ),
      ]
        .filter((control) => control.getBoundingClientRect().width > 0)
        .map((control) => {
          const box = control.getBoundingClientRect();
          return { width: Math.round(box.width), height: Math.round(box.height) };
        });
    });

    expect(sizes.length, `${density} has controls to measure`).toBeGreaterThan(0);
    for (const size of sizes) {
      expect(size.height, `${density} control height`).toBeGreaterThanOrEqual(24);
      expect(size.width, `${density} control width`).toBeGreaterThanOrEqual(24);
    }
  }
});
