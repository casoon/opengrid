import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The texts a component writes itself (plan point 48).
//
// Two claims are tested here. The built-in texts are **English** — before this
// point the status line was German and the filter labels English, so a screen
// reader announced half of the grid in the wrong voice. And a page overrides
// **any subset** of them through `set_texts`, with the language they are in, so
// `lang` follows the text rather than the document.

/** Every text the grid writes itself, plus the language it claims. */
async function texts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const select = root.querySelector('select[data-col="1"]');
    return {
      status: root.querySelector('[part="status"]').textContent,
      clear: root.querySelector('[part="filter-clear"]').textContent,
      filterGroup: root.querySelector('[part="filter"]').getAttribute("aria-label"),
      operatorLabel: select.getAttribute("aria-label"),
      valueLabel: root.querySelector('input[data-col="1"]').getAttribute("aria-label"),
      // What the user reads, and what the query actually sends.
      operatorTexts: [...select.options].map((option) => option.textContent),
      operatorValues: [...select.options].map((option) => option.value),
      // Only the component's own texts claim a language, and only on nodes
      // whose whole subtree is ours — `lang` is inherited, so a container that
      // also holds column names must not carry it.
      statusLang: root.querySelector('[part="status"]').getAttribute("lang"),
      operatorLang: select.getAttribute("lang"),
      clearLang: root.querySelector('[part="filter-clear"]').getAttribute("lang"),
      columnsToggleLang: root.querySelector('[part="columns-toggle"]').getAttribute("lang"),
      // The filter row holds the column disclosure (point 37), so it is a
      // container of data and claims nothing itself.
      filterLang: root.querySelector('[part="filter"]').getAttribute("lang"),
      columnsLang: root.querySelector('[part="columns"]').getAttribute("lang"),
      // The data must keep the page's language: nothing above the table claims
      // one, and no cell, header or column checkbox does either.
      layoutLang: root.querySelector('[part="layout"]').getAttribute("lang"),
      tableLang: root.querySelector('table[role="grid"]').closest("[lang]")?.getAttribute("lang"),
      // The regression guard: a column name is the page's word. Walking up from
      // its checkbox must not meet a `lang` anywhere inside the shadow root.
      columnLabelLang: root
        .querySelector('[part="column-toggle"]')
        .closest("[lang]")
        ?.getAttribute("lang"),
    };
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-texts.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("the built-in texts are English, in one language", async ({ page }) => {
  const built_in = await texts(page);
  expect(built_in).toMatchObject({
    status: "5 matches",
    clear: "Clear",
    filterGroup: "Filter",
    operatorLabel: "customer operator",
    valueLabel: "customer value",
    statusLang: "en",
    operatorLang: "en",
    clearLang: "en",
    columnsToggleLang: "en",
  });
  // The data is the page's, in the page's language — declaring it English
  // because the built-in texts are would be the WCAG 3.1.2 failure this point
  // removes, moved from the chrome to the content.
  expect(built_in.layoutLang).toBeNull();
  expect(built_in.tableLang).toBeUndefined();
  // Neither container claims a language: both hold column names. The filter row
  // does because the column disclosure sits inside it (point 37) — that is the
  // inheritance that declared every column name English until 2026-09-20.
  expect(built_in.filterLang).toBeNull();
  expect(built_in.columnsLang).toBeNull();
  expect(built_in.columnLabelLang).toBeUndefined();
});

test("the operator names are words, the query keeps the wire tokens", async ({
  page,
}) => {
  const { operatorTexts, operatorValues } = await texts(page);
  // `gte` is not a word; the user reads one and the query sends the token.
  expect(operatorTexts).toEqual([
    "contains",
    "starts with",
    "is",
    "is not",
    "greater than",
    "greater or equal",
    "less than",
    "less or equal",
    // Not "is empty": an empty string *is* a value (rule S14).
    "has no value",
    "has a value",
  ]);
  expect(operatorValues).toEqual([
    "contains",
    "starts_with",
    "eq",
    "ne",
    "gt",
    "gte",
    "lt",
    "lte",
    "is_null",
    "is_not_null",
  ]);
});

test("set_texts overrides a subset and carries its language", async ({ page }) => {
  await page.evaluate(() =>
    window.__setTexts({
      lang: "de",
      matchesOne: "{count} Treffer",
      matchesOther: "{count} Treffer",
      empty: "Keine Treffer",
      clear: "Leeren",
      filterGroup: "Filter",
      operators: { contains: "enthält", gte: "größer gleich" },
    }),
  );
  await expect.poll(() => texts(page).then((t) => t.status)).toBe("5 Treffer");

  const german = await texts(page);
  expect(german).toMatchObject({
    clear: "Leeren",
    // The announcement is now spoken German, although the document is English —
    // and the data is still English, because it did not change.
    statusLang: "de",
    operatorLang: "de",
    clearLang: "de",
    columnsToggleLang: "de",
  });
  expect(german.tableLang).toBeUndefined();
  // A translation moves our words into German and leaves the column names where
  // they were: in the language of the page.
  expect(german.filterLang).toBeNull();
  expect(german.columnLabelLang).toBeUndefined();
  expect(german.operatorTexts[5]).toBe("größer gleich");
  expect(german.operatorTexts[0]).toBe("enthält");
  // Keyed by the wire token, so translating two of them leaves the other six
  // alone instead of shifting every label by one.
  expect(german.operatorTexts[1]).toBe("starts with");
  // The wire tokens are untouched by a translation.
  expect(german.operatorValues[5]).toBe("gte");
  expect(german.operatorValues[1]).toBe("starts_with");
  // Keys the page did not mention keep their English default.
  expect(german.operatorLabel).toBe("customer operator");
  expect(german.valueLabel).toBe("customer value");
});

test("the grid still works after its texts changed", async ({ page }) => {
  // `set_texts` rebuilds the skeleton, so the filter row, the query and the
  // keyboard have to come back with it.
  await page.evaluate(() => window.__setTexts({ lang: "de", empty: "Keine Treffer" }));
  // Wait for the rebuild itself, not for a value the status already had.
  await expect.poll(() => texts(page).then((t) => t.statusLang)).toBe("de");
  expect(await texts(page).then((t) => t.status)).toBe("5 matches");

  await page.evaluate(() => {
    const input = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('input[data-col="1"]');
    input.focus();
  });
  await page.keyboard.type("Alpha");
  await page.keyboard.press("Enter");

  await expect.poll(() => texts(page).then((t) => t.status)).toBe("2 matches");
  expect(
    await page.evaluate(() =>
      [
        ...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr[aria-rowindex] td:nth-child(2)"),
      ].map((td) => td.textContent),
    ),
  ).toEqual(["Alpha", "Alpha"]);

  // And the override is still in force after the query.
  await page.evaluate(() => {
    const input = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('input[data-col="1"]');
    input.focus();
    input.select();
  });
  await page.keyboard.type("Delta");
  await page.keyboard.press("Enter");
  await expect.poll(() => texts(page).then((t) => t.status)).toBe("Keine Treffer");
});

test("texts set before the element connects are the ones it renders with", async ({
  page,
}) => {
  // The documented order — texts first, then the provider — takes the branch in
  // `set_texts` that does nothing but store, because there is no shadow root
  // yet. The element has to pick them up when it connects.
  await page.evaluate(() =>
    window.__createLate({ lang: "de", clear: "Leeren", empty: "Keine Treffer" }),
  );
  await page.waitForFunction(() => {
    const root = document.querySelector("#late")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });

  const late = await page.evaluate(() => {
    const root = document.querySelector("#late").shadowRoot;
    return {
      clear: root.querySelector('[part="filter-clear"]').textContent,
      lang: root.querySelector('[part="status"]').getAttribute("lang"),
      status: root.querySelector('[part="status"]').textContent,
    };
  });
  // Rendered German from the first paint, with no flash of the defaults to fix
  // up afterwards.
  expect(late).toEqual({ clear: "Leeren", lang: "de", status: "5 matches" });
});

test("an empty lang leaves the document's language alone", async ({ page }) => {
  // The documented escape hatch for a page whose texts are already in the
  // document's language.
  await page.evaluate(() => window.__setTexts({ lang: "", clear: "Leeren" }));
  await expect.poll(() => texts(page).then((t) => t.clear)).toBe("Leeren");

  const quiet = await texts(page);
  expect(quiet.statusLang).toBeNull();
  expect(quiet.filterLang).toBeNull();
});

test("a rebuild keeps the filter row, the scroll position and the focus", async ({
  page,
}) => {
  // `set_texts` rebuilds the skeleton. Everything the user can see has to come
  // back with it, or they find an empty filter row above filtered data.
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('select[data-col="1"]').value = "eq";
    const input = root.querySelector('input[data-col="1"]');
    input.focus();
    input.value = "Beta";
  });
  await page.keyboard.press("Enter");
  await expect.poll(() => texts(page).then((t) => t.status)).toBe("2 matches");

  await page.evaluate(() => {
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('td[data-row="1"][data-col="0"]')
      .focus();
  });

  await page.evaluate(() => window.__setTexts({ lang: "de", clear: "Leeren" }));
  await expect.poll(() => texts(page).then((t) => t.clear)).toBe("Leeren");

  const after = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      operator: root.querySelector('select[data-col="1"]').value,
      value: root.querySelector('input[data-col="1"]').value,
      focused: root.activeElement?.getAttribute("data-row") ?? null,
      status: root.querySelector('[part="status"]').textContent,
    };
  });
  expect(after).toEqual({ operator: "eq", value: "Beta", focused: "1", status: "2 matches" });
});

test("has no axe violations with overridden texts", async ({ page }) => {
  await page.evaluate(() => window.__setTexts({ lang: "de", clear: "Leeren" }));
  await expect.poll(() => texts(page).then((t) => t.clear)).toBe("Leeren");

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
