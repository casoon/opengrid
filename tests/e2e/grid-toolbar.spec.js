import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The toolbar and the chips (plan point 65).
//
// The chips are a display of the view (point 59), not a second truth about the
// filters. The filter-row switch is a view setting, on by default — the row has
// always been there, and a boolean attribute is off by default in HTML.

const root = "document.querySelector('opengrid-grid').shadowRoot";

async function q(page, selector, read) {
  return page.evaluate(
    ({ selector, read }) => {
      const node = document.querySelector("opengrid-grid").shadowRoot.querySelector(selector);
      return node ? new Function("node", `return ${read}`)(node) : null;
    },
    { selector, read },
  );
}

async function status(page) {
  return q(page, '[part="status"]', "node.textContent.trim()");
}

async function chips(page) {
  return page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="chip"]')].map((chip) => ({
      text: chip.querySelector("span").textContent,
      remove: chip.querySelector('[part="chip-remove"]').getAttribute("aria-label"),
    })),
  );
}

/** Filters a column through the filter row, the way a reader would. */
async function filter(page, col, op, value) {
  await page.evaluate(
    ({ col, op, value }) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const select = root.querySelector(`select[data-col="${col}"]`);
      select.value = op;
      select.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
      const input = root.querySelector(`input[data-col="${col}"]`);
      input.value = value;
      input.focus();
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, composed: true }));
    },
    { col, op, value },
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-toolbar.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 matches");
});

test("the toolbar holds the switch, the column list and the density", async ({ page }) => {
  const bar = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const toolbar = root.querySelector('[part="toolbar"]');
    return {
      role: toolbar.getAttribute("role"),
      label: root.getElementById(toolbar.getAttribute("aria-labelledby")).textContent,
      toggle: toolbar.querySelector('[part="filter-row-toggle"]').getAttribute("aria-pressed"),
      columns: !!toolbar.querySelector('[part="columns-toggle"]'),
      columnsInFilterRow: !!root.querySelector('[part="filter"] [part="columns-toggle"]'),
      pressed: [...toolbar.querySelectorAll("[data-density]")]
        .filter((button) => button.getAttribute("aria-pressed") === "true")
        .map((button) => button.dataset.density),
    };
  });
  expect(bar).toEqual({
    role: "group",
    label: "Grid tools",
    toggle: "true",
    columns: true,
    columnsInFilterRow: false,
    pressed: ["normal"],
  });
});

test("hiding the filter row gives the viewport exactly its height", async ({ page }) => {
  // `display: none`, not `visibility`: a hidden row that kept its height would
  // shrink the viewport PageUp/PageDown step by (phase E (h)).
  const measure = () =>
    page.evaluate(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      return {
        viewport: root.querySelector('[part="viewport"]').getBoundingClientRect().height,
        filter: root.querySelector('[part="filter"]').getBoundingClientRect().height,
      };
    });
  const before = await measure();
  expect(before.filter).toBeGreaterThan(0);

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="filter-row-toggle"]').click(),
  );
  const after = await measure();
  expect(after.filter).toBe(0);
  expect(after.viewport).toBeCloseTo(before.viewport + before.filter, 0);
  expect(await q(page, '[part="filter-row-toggle"]', 'node.getAttribute("aria-pressed")')).toBe("false");

  // And it is part of the view.
  const view = await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));
  expect(view.filterRow).toBe(false);
});

test("a filter shows as a chip in words", async ({ page }) => {
  await filter(page, 2, "eq", "DE");
  await expect.poll(() => status(page)).toBe("52 matches");
  expect(await chips(page)).toEqual([{ text: "country is DE", remove: "Remove country is DE" }]);
});

test("each remove button names its own filter", async ({ page }) => {
  // Ten buttons called "Remove" are ten buttons nobody can tell apart.
  await filter(page, 2, "eq", "DE");
  await expect.poll(() => status(page)).toBe("52 matches");
  await filter(page, 1, "is_null", "");
  await expect.poll(async () => (await chips(page)).length).toBe(2);
  const names = (await chips(page)).map((chip) => chip.remove);
  expect(new Set(names).size).toBe(names.length);
  expect(names).toContain("Remove customer has no value");
});

test("removing a chip removes the filter, says so once, and keeps the focus", async ({ page }) => {
  await filter(page, 2, "eq", "DE");
  await filter(page, 3, "gte", "0");
  await expect.poll(async () => (await chips(page)).length).toBe(2);

  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const line = root.querySelector('[part="status"]');
    window.__said = [];
    new MutationObserver(() => {
      const text = line.textContent.trim();
      if (window.__said.at(-1) !== text) window.__said.push(text);
    }).observe(line, { childList: true, characterData: true, subtree: true });
  });
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="column:country"]').focus(),
  );
  await page.keyboard.press("Enter");

  await expect.poll(async () => (await chips(page)).map((chip) => chip.text)).toEqual(["amount greater or equal 0"]);
  await page.waitForTimeout(250);
  const said = await page.evaluate(() => window.__said);
  // Said once, with the result — not with the "loading" frame before it.
  expect(said.filter((text) => text.includes("country is DE removed"))).toHaveLength(1);
  expect(said.at(-1)).toMatch(/matches · country is DE removed$/);

  // The focus moved to the chip that took its place, not to the document.
  expect(await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.activeElement?.dataset.chipRemove)).toBe(
    "column:amount",
  );
});

test("an is-null filter is removed too, not left standing with an empty field", async ({ page }) => {
  // `is_null` takes no value, so emptying the field alone would keep it.
  await filter(page, 1, "is_null", "");
  await expect.poll(async () => (await chips(page)).length).toBe(1);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="column:customer"]').click(),
  );
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
  expect(await chips(page)).toEqual([]);
});

test("the grouping is a chip too, and removing it ungroups", async ({ page }) => {
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("group-by", "country"));
  await expect.poll(async () => (await chips(page)).map((chip) => chip.text)).toEqual(["Grouped by country"]);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="group"]').click(),
  );
  await expect
    .poll(() => page.evaluate(() => document.querySelector("opengrid-grid").hasAttribute("group-by")))
    .toBe(false);
});

test("Remove all clears filters and grouping, once, and focuses the toolbar", async ({ page }) => {
  await filter(page, 2, "eq", "DE");
  await expect.poll(() => status(page)).toBe("52 matches");
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("group-by", "customer"));
  await expect.poll(async () => (await chips(page)).length).toBe(2);

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="chips-clear"]').click(),
  );
  await expect.poll(() => status(page)).toBe("200 matches · All filters removed");
  expect(await chips(page)).toEqual([]);
  expect(await q(page, '[part="chips"]', 'node.hasAttribute("hidden")')).toBe(true);
  expect(
    await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.activeElement?.getAttribute("part")),
  ).toBe("filter-row-toggle");
});

test("a scroll does not rebuild the chips under the focus", async ({ page }) => {
  // The chips are redrawn only when what they say changed. A scroll frame that
  // rebuilt them would take the focus off the chip the reader stands on.
  await filter(page, 3, "gte", "0");
  await expect.poll(async () => (await chips(page)).length).toBe(1);
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector("[data-chip-remove]").focus(),
  );
  for (const top of [300, 900, 0]) {
    await page.evaluate((top) => {
      document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]').scrollTop = top;
    }, top);
    await page.waitForTimeout(80);
  }
  expect(
    await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.activeElement?.dataset.chipRemove),
  ).toBe("column:amount");
});

test("the density buttons switch the density", async ({ page }) => {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-density="compact"]').click(),
  );
  await expect
    .poll(() => page.evaluate(() => document.querySelector("opengrid-grid").getAttribute("density")))
    .toBe("compact");
  expect(await q(page, '[data-density="compact"]', 'node.getAttribute("aria-pressed")')).toBe("true");
  expect(await q(page, '[data-density="normal"]', 'node.getAttribute("aria-pressed")')).toBe("false");
});

test("the column names in the toolbar claim no language of ours", async ({ page }) => {
  // Phase E (k): check the *inherited* value, not the set attribute. The column
  // list moved into the toolbar; the toolbar must not tag the page's words.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="columns-toggle"]').click(),
  );
  const inherited = await page.evaluate(() => {
    const box = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="column-toggle"] input');
    return box.closest("[lang]")?.getAttribute("lang") ?? null;
  });
  expect(inherited).toBeNull();
});

test("every toolbar control meets the minimum target size", async ({ page }) => {
  await filter(page, 2, "eq", "DE");
  await expect.poll(async () => (await chips(page)).length).toBe(1);
  const sizes = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="toolbar"] button, [part="chips"] button')].map(
      (button) => {
        const box = button.getBoundingClientRect();
        return [Math.round(box.width), Math.round(box.height)];
      },
    ),
  );
  expect(sizes.length).toBeGreaterThan(4);
  for (const [width, height] of sizes) {
    expect(width).toBeGreaterThanOrEqual(24);
    expect(height).toBeGreaterThanOrEqual(24);
  }
});

test("has no axe violations with toolbar and chips", async ({ page }) => {
  await filter(page, 2, "eq", "DE");
  await expect.poll(async () => (await chips(page)).length).toBe(1);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
