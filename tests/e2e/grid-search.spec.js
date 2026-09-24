import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The search field (plan point 67). F5, decided 2026-09-24: the query language
// is an input method, not a second truth — it produces the filter row's own
// entries, and the field empties afterwards.

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

async function field(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const input = root.querySelector('[part="search-input"]');
    const list = root.querySelector('[part="search-list"]');
    return {
      value: input.value,
      expanded: input.getAttribute("aria-expanded"),
      active: input.getAttribute("aria-activedescendant"),
      options: [...list.querySelectorAll('[role="option"]')].map((option) => ({
        id: option.id,
        column: option.dataset.column,
        selected: option.getAttribute("aria-selected"),
      })),
      hint: !root.querySelector('[part="search-hint"]').hidden,
      focused: root.activeElement === input,
    };
  });
}

async function focusSearch(page) {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-input"]').focus(),
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-search.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 matches");
});

test("the field is a combobox that controls a listbox", async ({ page }) => {
  const roles = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const input = root.querySelector('[part="search-input"]');
    const list = root.getElementById(input.getAttribute("aria-controls"));
    return {
      role: input.getAttribute("role"),
      autocomplete: input.getAttribute("aria-autocomplete"),
      label: root.getElementById(input.getAttribute("aria-labelledby")).textContent,
      listRole: list?.getAttribute("role"),
    };
  });
  expect(roles).toEqual({ role: "combobox", autocomplete: "list", label: "Search or filter", listRole: "listbox" });
});

test("the list follows the last word, and the arrows move through it", async ({ page }) => {
  await focusSearch(page);
  await page.keyboard.type("c");
  let state = await field(page);
  expect(state.expanded).toBe("true");
  expect(state.options.map((option) => option.column)).toEqual(["customer", "country"]);

  await page.keyboard.press("ArrowDown");
  state = await field(page);
  expect(state.active).toBe(state.options[0].id);
  expect(state.options[0].selected).toBe("true");
  await page.keyboard.press("ArrowDown");
  expect((await field(page)).active).toBe(state.options[1].id);
  await page.keyboard.press("ArrowDown");
  expect((await field(page)).active).toBe(state.options[0].id);
  // The focus never left the field.
  expect((await field(page)).focused).toBe(true);
});

test("Enter takes a suggestion, and the next word is the operator", async ({ page }) => {
  await focusSearch(page);
  await page.keyboard.type("cou");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  const state = await field(page);
  expect(state.value).toBe("country ");
  expect(state.expanded).toBe("false");
  expect(state.focused).toBe(true);
});

test("Escape closes the list, then empties the field", async ({ page }) => {
  await focusSearch(page);
  await page.keyboard.type("am");
  expect((await field(page)).expanded).toBe("true");
  await page.keyboard.press("Escape");
  expect(await field(page)).toMatchObject({ expanded: "false", value: "am" });
  await page.keyboard.press("Escape");
  expect((await field(page)).value).toBe("");
});

test("an expression becomes the filter row's own entries", async ({ page }) => {
  // One place a filter lives: the same entries the filter row would hold, in
  // its fields, in the view, as chips.
  await focusSearch(page);
  await page.keyboard.type("country = DE and amount >= 0");
  expect((await field(page)).hint).toBe(true);
  await page.keyboard.press("Enter");

  await expect.poll(() => status(page)).toMatch(/^\d+ matches$/);
  expect((await field(page)).value).toBe("");
  const view = await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));
  expect(view.filters).toEqual([
    { column: "country", op: "eq", value: "DE" },
    { column: "amount", op: "gte", value: "0" },
  ]);
  const row = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      op: root.querySelector('select[data-col="2"]').value,
      value: root.querySelector('input[data-col="2"]').value,
    };
  });
  expect(row).toEqual({ op: "eq", value: "DE" });
});

test("an expression that does not parse is named, and kept for correcting", async ({ page }) => {
  // Falling back to free text would look like it worked and show wrong rows.
  await focusSearch(page);
  await page.keyboard.type("amount > abc");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toContain("abc");
  expect((await field(page)).value).toBe("amount > abc");
  const view = await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));
  expect(view.filters).toEqual([]);
});

test("a typo in the first column is named, not searched as text", async ({ page }) => {
  // Point 72, E30: `colour = red` has the shape of a filter. Reading it as free
  // text would search for the literal words, find nothing, and say "No
  // matches" — the reader would never learn that the column name was wrong.
  await focusSearch(page);
  await page.keyboard.type("colour = red");
  expect((await field(page)).hint).toBe(true);
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("colour is not a column of this grid");
  expect((await field(page)).value).toBe("colour = red");
  const chips = await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="chip"]').length,
  );
  expect(chips).toBe(0);
});

test("a type the operator does not fit is named", async ({ page }) => {
  await focusSearch(page);
  await page.keyboard.type("amount ~ 5");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("amount does not take ~");
});

test("free text searches the values of the text columns, case-sensitively", async ({ page }) => {
  await focusSearch(page);
  await page.keyboard.type("DE");
  expect((await field(page)).hint).toBe(false);
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("52 matches");

  // S5: `contains` is case-sensitive in V1, and the field does not pretend
  // otherwise.
  await page.keyboard.press("Escape");
  await expect.poll(() => status(page)).toBe("200 matches");
  await page.keyboard.type("de");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("No matches");
});

test("free text never searches formatted display", async ({ page }) => {
  // Phase E: formatting is display and never reaches a query. A number column
  // is not searched by its printed text.
  await focusSearch(page);
  await page.keyboard.type("30.00");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).toBe("No matches");
});

test("a free-text search is a chip, and removing it clears it", async ({ page }) => {
  await focusSearch(page);
  await page.keyboard.type("Alpha");
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page)).not.toBe("200 matches");
  const chip = await page.evaluate(
    () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="search"]').getAttribute("aria-label"),
  );
  expect(chip).toBe("Remove Text contains “Alpha”");
  await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector('[data-chip-remove="search"]').click());
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
});

test("the translated joining word works, and the English one still does", async ({ page }) => {
  await page.evaluate(() =>
    window.__opengridModule.set_texts(document.querySelector("opengrid-grid"), { queryAnd: "und" }),
  );
  await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-input"]'));
  await focusSearch(page);
  await page.keyboard.type("country = DE und amount >= 0");
  await page.keyboard.press("Enter");
  await expect.poll(async () =>
    (await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")))).filters.length,
  ).toBe(2);
});

test("has no axe violations, list open and closed", async ({ page }) => {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await focusSearch(page);
  await page.keyboard.type("c");
  await page.keyboard.press("ArrowDown");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
