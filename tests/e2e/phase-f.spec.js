import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Phase F as a whole (plan point 70): every surface it added, in every state it
// can be in, checked for three things a feature-by-feature spec cannot see.
//
// 1. **Whose word is it.** The fixture's page speaks English and the grid's own
//    words are German. Every text node must inherit `de` if it is ours and
//    nothing from inside the grid if it is the page's — a column name, a value,
//    a sentence that splices one in. Phase E (k): the *inherited* value, read in
//    the real DOM, not the attribute somebody meant to set.
// 2. **Every part in the DOM is documented.** The Rust freeze gathers the parts
//    from the skeleton plus a hand-kept list of those drawn later; this reads
//    the ones a running grid actually writes.
// 3. **axe**, in each state, including the open popovers and the sidebar.

const DOCS = readFileSync(fileURLToPath(new URL("../../docs/api.md", import.meta.url)), "utf8");
const DOCUMENTED_PARTS = new Set(
  DOCS.split("**Parts:**")[1]
    .split("\n\n")[0]
    .split("`")
    .filter((_, index) => index % 2 === 1),
);

// Where a text node sits (its nearest `part`) decides whose word it is.
const OURS = new Set([
  "status", "filter-operator", "filter-clear", "page-first", "page-previous", "page-next", "page-last",
  "search-hint", "filter-row-toggle", "facets-toggle", "density", "columns-toggle", "chips-clear",
  "facet-cost", "facets-head", "facet-bounds", "column-menu", "menu-label", "empty-text", "empty-reset",
]);
const PAGE = new Set([
  // Column names and values.
  "header", "column-toggle", "facet", "facet-value", "facet-count", "facet-pill", "search-list",
  // Our sentence with a column name or a value spliced in: mixed by
  // construction, and a `lang` of ours would claim the page's word.
  "chip",
]);

/** Every exposed text node in the shadow root: its text, part and inherited `lang`. */
async function leaves(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const out = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = node.textContent.trim();
      const element = node.parentElement;
      if (!text || element.closest("style, [hidden], [aria-hidden='true']")) continue;
      // A glyph (`×`, `▲`) is in no language; the control around it is named
      // by its `aria-label`.
      if (!/[\p{L}\p{N}]/u.test(text)) continue;
      const cell = element.closest("td");
      out.push({
        text,
        part: element.closest("[part]")?.getAttribute("part").split(" ")[0] ?? null,
        // Cells are told apart by row kind: the total's label is ours, every
        // other cell holds a value or a group's name for one.
        kind: cell ? (cell.closest("tr").dataset.kind ?? "row") + (cell.dataset.col === "0" ? ":label" : "") : null,
        lang: element.closest("[lang]")?.getAttribute("lang") ?? null,
      });
    }
    return out;
  });
}

async function checkLeaves(page, state) {
  const found = await leaves(page);
  expect(found.length, `${state}: there are leaves to check`).toBeGreaterThan(0);
  for (const leaf of found) {
    const where = `${state}: "${leaf.text}" in ${leaf.part}${leaf.kind ? ` (${leaf.kind})` : ""}`;
    if (leaf.part === "cell") {
      expect(leaf.lang, where).toBe(leaf.kind === "total:label" ? "de" : null);
    } else if (OURS.has(leaf.part)) {
      expect(leaf.lang, where).toBe("de");
    } else if (PAGE.has(leaf.part)) {
      expect(leaf.lang, where).toBeNull();
    } else {
      throw new Error(`${where}: a new leaf nobody classified — ours or the page's?`);
    }
  }
}

/**
 * Every accessible name the grid gives itself: where it comes from, and the
 * language of the words it is made of.
 */
async function names(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("[aria-label], [aria-labelledby]")]
      .filter((node) => !node.closest("[hidden]"))
      .map((node) => {
        const id = node.getAttribute("aria-labelledby");
        const source = id ? root.getElementById(id) : node;
        return {
          part: node.getAttribute("part")?.split(" ")[0] ?? node.tagName.toLowerCase(),
          name: id ? source?.textContent : node.getAttribute("aria-label"),
          by: id ? "reference" : "attribute",
          lang: source?.closest("[lang]")?.getAttribute("lang") ?? null,
        };
      });
  });
}

// Names that are the page's word or splice one into ours — the open question
// of phase E (k) for single attributes, which no `lang` can settle.
const MIXED_NAMES = new Set(["table", "filter-value", "chip-remove", "cell"]);

async function partsInDom(page) {
  return page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("[part]")].flatMap((node) =>
      node.getAttribute("part").split(" "),
    ),
  );
}

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

// The states, each reached from the fresh grid. Each answers once it has settled.
const STATES = {
  rows: async () => {},
  "chips, search and grouping": async (page) => {
    await page.evaluate(() =>
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        filters: [{ column: "country", op: "eq", value: "DE" }],
        group: ["country", "customer"],
        expanded: [["DE"]],
        aggregates: { amount: "sum", qty: "sum" },
      }),
    );
    await page.evaluate(() => {
      const input = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-input"]');
      input.value = "Alpha";
      input.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, composed: true }));
    });
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="total"]'));
    await expect.poll(async () => (await leaves(page)).filter((leaf) => leaf.part === "chip").length).toBe(3);
  },
  "column menu open": async (page) => {
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="column-menu-button"]').click(),
    );
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="column-menu"]:not([hidden])'));
  },
  "search suggestions open": async (page) => {
    const input = page.locator("opengrid-grid").locator('[part="search-input"]');
    await input.focus();
    await page.keyboard.type("cou");
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-list"]:not([hidden]) [role="option"]'));
  },
  empty: async (page) => {
    await page.evaluate(() =>
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        filters: [{ column: "country", op: "eq", value: "XX" }],
      }),
    );
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="empty"]:not([hidden])'));
  },
};

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-phase-f.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 Treffer");
});

for (const [state, reach] of Object.entries(STATES)) {
  test(`${state}: every leaf inherits the language of whoever wrote it`, async ({ page }) => {
    await reach(page);
    await checkLeaves(page, state);
  });

  test(`${state}: every name of ours is in our language`, async ({ page }) => {
    // F9, decided 2026-09-24: a container that holds the page's words is named
    // by reference, so the name can carry `lang` without the container.
    await reach(page);
    const found = await names(page);
    expect(found.length).toBeGreaterThan(0);
    for (const name of found) {
      const where = `${state}: "${name.name}" on ${name.part} (${name.by})`;
      if (MIXED_NAMES.has(name.part)) continue;
      expect(name.name, where).toBeTruthy();
      expect(name.lang, where).toBe("de");
    }
  });

  test(`${state}: every part in the DOM is documented`, async ({ page }) => {
    await reach(page);
    const undocumented = [...new Set(await partsInDom(page))].filter((part) => !DOCUMENTED_PARTS.has(part));
    expect(undocumented).toEqual([]);
  });

  test(`${state}: has no axe violations`, async ({ page }) => {
    await reach(page);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

test("the type beside a suggestion is translatable and in our language", async ({ page }) => {
  // F8, decided 2026-09-24: "text", "integer" … are our words, not the page's.
  await STATES["search suggestions open"](page);
  const kind = await page.evaluate(() => {
    const option = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-list"] [data-column="country"]');
    const span = option.querySelector("span");
    return { text: span.textContent, lang: span.closest("[lang]")?.getAttribute("lang") ?? null, option: option.closest("[lang]")?.getAttribute("lang") ?? null };
  });
  expect(kind).toEqual({ text: "Text", lang: "de", option: null });
});

test("the total row's label stops being ours when its slot shows a value again", async ({ page }) => {
  // Rows are pooled: the slot that drew "Total (6 rows)" with our `lang` may
  // draw a customer next, and the customer must not keep it.
  await STATES["chips, search and grouping"](page);
  await page.evaluate(() => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {}));
  await expect.poll(() => status(page)).toBe("200 Treffer");
  const tagged = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody td[lang]")].length,
  );
  expect(tagged).toBe(0);
});
