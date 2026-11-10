import { test, expect } from "@playwright/test";

// The shipped German texts (issue #333): `@casoon/opengrid/texts/de`, handed to
// `set_texts` on a grid, a table and a pivot.
//
// The claim is "a German page shows and announces no English word of the
// component". A list of English words to look for would be a second copy of
// the defaults, out of date with the next key. So the page itself says which
// words are the component's: the same walk is done twice, once without texts
// (English) and once with the German ones, and everything the elements wrote
// or said on the way is recorded. Every word of the English recording that is
// not the page's own is an English word of the component, and none of them
// may be anywhere in the German one — not even inside a sentence that is
// otherwise German.

const PAGE = "/tests/e2e/fixtures/texts-de.html";

/**
 * Words both languages have — each one a decision: German says "Filter",
 * "Normal", "Text" and "Pivot" too, `Enter` is the key's name on the keyboard,
 * `wasm` and `DE` are no language's, and `a` is the sign of "starts with"
 * (`a…`).
 */
const SHARED = new Set(["Filter", "Normal", "Text", "Pivot", "Enter", "wasm", "DE", "a"]);

/** What the walk types: the page's words, as its data is. */
const TYPED = ["zzz", "zwei", "farbe = rot", "k"];

/** In the shadow root of the element `selector` names. */
function shadow(page, selector, action, argument) {
  return page.evaluate(
    ([selector, action, argument]) =>
      new Function("root", "argument", `return (${action})(root, argument)`)(
        document.querySelector(selector).shadowRoot,
        argument,
      ),
    [selector, action.toString(), argument],
  );
}

/**
 * Starts recording every text the three elements show or name something with:
 * text nodes and the attributes a screen reader reads. A MutationObserver
 * takes each state on the way, so a sentence the status line said and
 * replaced is in the recording too; `said` keeps the status lines in order.
 */
async function record(page) {
  await page.evaluate(() => {
    const NAMES = ["aria-label", "aria-description", "aria-roledescription", "aria-valuetext", "title", "placeholder", "alt"];
    const words = new Set();
    const said = [];
    const read = (root) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (node.nodeType === Node.TEXT_NODE) {
          if (node.parentElement?.tagName !== "STYLE" && node.data.trim()) words.add(node.data.trim());
        } else {
          for (const name of NAMES) {
            const value = node.getAttribute(name)?.trim();
            if (value) words.add(value);
          }
        }
      }
      for (const line of root.querySelectorAll('[role="status"], [role="alert"], [aria-live]')) {
        const text = line.textContent.trim();
        if (text && said.at(-1) !== text) said.push(text);
      }
    };
    for (const host of document.querySelectorAll("opengrid-grid, opengrid-table, opengrid-pivot")) {
      const root = host.shadowRoot;
      read(root);
      new MutationObserver(() => read(root)).observe(root, {
        subtree: true,
        childList: true,
        characterData: true,
        attributes: true,
        attributeFilter: NAMES,
      });
    }
    window.__words = words;
    window.__said = said;
  });
}

/**
 * What a click or a key started has been drawn and observed: two frames, no
 * status line still loading, and two frames more.
 */
async function settle(page) {
  const frames = () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
  await page.evaluate(frames);
  await page.waitForFunction(() =>
    [...document.querySelectorAll("opengrid-grid, opengrid-pivot")].every(
      (host) => host.shadowRoot.querySelector('[part="status"]').dataset.state !== "loading",
    ),
  );
  await page.evaluate(frames);
}

/**
 * The same walk through all three elements, whatever language they speak:
 * nothing here finds a control by its words.
 */
async function walk(page, query) {
  await page.goto(`${PAGE}${query}`);
  await page.waitForFunction(() => window.__opengridReady);
  await page.evaluate(() => window.__opengridReady);
  for (const selector of ["#bestellungen", "#seiten"]) {
    await page.waitForFunction(
      (selector) => !!document.querySelector(selector).shadowRoot?.querySelector("td[data-row]"),
      selector,
    );
  }
  await page.waitForFunction(
    () =>
      document.querySelector("opengrid-pivot").shadowRoot?.querySelector('[part="status"]')?.dataset.state ===
        "ready" && !!document.querySelector("opengrid-table").shadowRoot?.querySelector("tbody td"),
  );
  await record(page);

  const grid = "#bestellungen";
  // The operator menu of `kunde`.
  await shadow(page, grid, (root) => root.querySelector('[part="filter-operator"][data-col="1"]').click());
  await settle(page);
  await page.keyboard.press("Escape");

  // A filter nothing matches: the empty state and its chip, then cleared.
  await shadow(page, grid, (root) => root.querySelector('input[part="filter-value"][data-col="1"]').focus());
  await page.keyboard.type(TYPED[0]);
  await page.keyboard.press("Enter");
  await page.waitForFunction(
    () => !!document.querySelector("#bestellungen").shadowRoot.querySelector('[part="chips"] button'),
  );
  await settle(page);
  await shadow(page, grid, (root) => root.querySelector('[part="chips"] button').click());
  await settle(page);

  // A value the column cannot hold.
  await shadow(page, grid, (root) => root.querySelector('input[part="filter-value"][data-col="0"]').focus());
  await page.keyboard.type(TYPED[1]);
  await page.keyboard.press("Enter");
  await settle(page);
  await shadow(page, grid, (root) => root.querySelector('[part="filter-clear"]').click());
  await settle(page);

  // The search field: its suggestions with their types, and a refusal.
  await shadow(page, grid, (root) => root.querySelector('[part="search-input"]').focus());
  await page.keyboard.type(TYPED[3]);
  await settle(page);
  await page.keyboard.press("Escape");
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(TYPED[2]);
  await page.keyboard.press("Enter");
  await settle(page);
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Enter");
  await settle(page);

  // *Add filter*, the dialog.
  await shadow(page, grid, (root) => root.querySelector('[part="add-filter"]').click());
  await settle(page);
  await page.keyboard.press("Escape");

  // Select every match from the header of the selection column.
  await shadow(page, grid, (root) => root.querySelector('[data-select="all"]').click());
  await settle(page);

  // The column menu of `kunde`: every entry, then grouped by it.
  await shadow(page, grid, (root) => root.querySelector('th[data-col="1"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await settle(page);
  await shadow(page, grid, (root) => root.querySelector('[part="column-menu"] [data-action="hide"]').click());
  await settle(page);
  await shadow(page, grid, (root) => root.querySelector('[part="columns"] input[data-column="kunde"]').click());
  await settle(page);
  await shadow(page, grid, (root) => root.querySelector('th[data-col="2"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await settle(page);
  await shadow(page, grid, (root) => root.querySelector('[part="column-menu"] [data-action="group:first"]').click());
  await page.waitForFunction(
    () => !!document.querySelector("#bestellungen").shadowRoot.querySelector("tbody tr[aria-expanded]"),
  );
  await settle(page);
  // Open the first group: said in the status line.
  await shadow(page, grid, (root) => root.querySelector('tbody tr[aria-expanded="false"] td:not([part~="select"])').click());
  await settle(page);

  // The paged grid: the next page.
  await shadow(page, "#seiten", (root) => root.querySelector('[part="page-next"]').click());
  await settle(page);

  // The table: sorted by `kunde`.
  await shadow(page, "opengrid-table", (root) => root.querySelector('[part="sort-button"][data-column="kunde"]').click());
  await settle(page);

  // The pivot: the menu behind "+ Row", and a filter it cannot read.
  await shadow(page, "opengrid-pivot", (root) => root.querySelector('[part="add-field"][data-add="rows"]').click());
  await settle(page);
  await page.keyboard.press("Escape");
  await shadow(page, "opengrid-pivot", (root) => root.querySelector("[data-filter-input]").focus());
  await page.keyboard.type(TYPED[2]);
  await page.keyboard.press("Enter");
  await settle(page);

  return page.evaluate(() => ({
    words: [...window.__words],
    said: window.__said,
    page: window.__pageWords,
    // Whose language the status lines claim.
    lang: [...document.querySelectorAll("opengrid-grid, opengrid-pivot")].map((host) =>
      host.shadowRoot.querySelector('[part="status"]').getAttribute("lang"),
    ),
  }));
}

/** The runs of letters in `text` — its words, whatever is between them. */
const letters = (text) => text.match(/\p{L}+/gu) ?? [];

test("a German page shows and announces no English word of the component", async ({ page }) => {
  test.slow();
  const english = await walk(page, "?texts=none");
  const german = await walk(page, "");

  // The walk reached the component's own words, and without texts they are
  // English: the comparison below has something to compare with.
  expect(english.words).toContain("5 matches");
  expect(english.said).toContain("5 matches · 5 rows selected");
  expect(english.lang).toEqual(["en", "en", "en"]);

  // The English words of the component: every word of the English walk that
  // is not the page's — its data, its column names, its labels, what was typed.
  const own = new Set([...german.page, ...TYPED].flatMap(letters));
  const theirs = new Set(
    [...english.words, ...english.said].flatMap(letters).filter((word) => !own.has(word) && !SHARED.has(word)),
  );
  expect(theirs.size).toBeGreaterThan(60);

  // Shown, named or said: none of them is on the German page.
  const untranslated = [...german.words, ...german.said].filter((text) =>
    letters(text).some((word) => theirs.has(word)),
  );
  expect(untranslated).toEqual([]);

  // Announced: in a German voice, and these are the words.
  expect(german.lang).toEqual(["de", "de", "de"]);
  expect(german.said).toEqual(
    expect.arrayContaining([
      "5 Treffer",
      "Keine Treffer",
      "5 Treffer · kunde enthält zzz entfernt",
      "farbe ist keine Spalte dieses Grids",
      "5 Treffer · 5 Zeilen ausgewählt",
      "5 Treffer · kunde ausgeblendet, 5 von 6 Spalten sichtbar",
      "5 Treffer · Deutschland aufgeklappt, 2 Zeilen",
      "5 Treffer · Seite 2 von 3",
    ]),
  );
});
