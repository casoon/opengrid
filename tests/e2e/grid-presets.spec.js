import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// The page's presets (issue #532): named views as a segmented control in the
// toolbar — a radiogroup, one tab stop, the arrows choose. The chosen preset is
// and-ed onto the page's `filter` and the reader's filters in every query, it
// is in the view as `preset`, and it is not a chip.
//
// The fixture: the 200 orders of grid-virtual.csv, four presets — All orders,
// Germany (52 rows), Big (qty >= 500), Nothing (no row) — with the filter row,
// the search, a list facet and a selection column by `row-key` at hand. Every
// query the grid asks is recorded, so each path that reaches the source is
// looked at on its own.

const DE = { field: "country", op: "eq", value: "DE" };
const BIG = { field: "qty", op: "gte", value: 500 };
const PAGE = { field: "id", op: "lte", value: 150 };

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-presets.html${query}`);
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
}

const grid = "opengrid-grid";

/** Every query asked since `clear`, parsed. */
const queries = (page) => page.evaluate(() => window.__queries.map((json) => JSON.parse(json)));
const clear = (page) => page.evaluate(() => (window.__queries.length = 0));

const isRows = (query) => "offset" in query && !query.group && !query.aggregate;
const isGroups = (query) => Array.isArray(query.group);
/** A count of the whole filter: an aggregate, and no group. */
const isTotal = (query) => query.aggregate && !query.group;

const lastRows = async (page) => (await queries(page)).filter(isRows).at(-1);

/** How many rows the engine itself counts under `filter`. */
const counted = (page, filter) =>
  page.evaluate(
    (filter) =>
      JSON.parse(
        window.__engine.execute(
          JSON.stringify({
            source: "orders",
            select: ["id"],
            filter,
            sort: [{ field: "id", direction: "asc" }],
            offset: 0,
            limit: 1,
          }),
        ),
      ).total_count,
    filter,
  );

/** The segments: id, name, whether chosen, and the tab stop. */
const segments = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll('[part="presets"] [part="preset"]')].map((button) => ({
      id: button.getAttribute("data-preset"),
      name: button.textContent,
      role: button.getAttribute("role"),
      checked: button.getAttribute("aria-checked"),
      tabindex: button.getAttribute("tabindex"),
    })),
  );

const chosen = async (page) => (await segments(page)).find((segment) => segment.checked === "true")?.id;

const focusSegment = (page, id) =>
  shadow(page, (root, id) => root.querySelector(`[part="preset"][data-preset="${id}"]`).focus(), id);

const focusedSegment = (page) =>
  shadow(page, (root) => root.activeElement?.getAttribute("data-preset") ?? null);

const view = (page) => page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));

const setView = (page, value) =>
  page.evaluate((value) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), value), value);

const chips = (page) =>
  shadow(page, (root) => [...root.querySelectorAll('[part="chip"]')].map((chip) => chip.textContent));

const setPresets = (page, presets, options) =>
  page.evaluate(
    ([presets, options]) =>
      window.__opengridModule.set_presets(document.querySelector("opengrid-grid"), presets, options),
    [presets, options],
  );

test("the presets are a radiogroup first in the toolbar, the first one chosen", async ({ page }) => {
  await open(page);
  expect(await segments(page)).toEqual([
    { id: "all", name: "All orders", role: "radio", checked: "true", tabindex: "0" },
    { id: "de", name: "Germany", role: "radio", checked: "false", tabindex: "-1" },
    { id: "big", name: "Big", role: "radio", checked: "false", tabindex: "-1" },
    { id: "none", name: "Nothing", role: "radio", checked: "false", tabindex: "-1" },
  ]);
  const group = await shadow(page, (root) => {
    const presets = root.querySelector('[part="presets"]');
    return {
      role: presets.getAttribute("role"),
      name: presets.getAttribute("aria-label"),
      first: presets.parentElement.firstElementChild === presets,
      inToolbar: presets.parentElement.getAttribute("part"),
    };
  });
  expect(group).toEqual({ role: "radiogroup", name: "Views", first: true, inToolbar: "toolbar" });
  // The first preset has no filter: every row, and the query has none either.
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
  expect((await lastRows(page)).filter).toBeUndefined();
});

test("the arrows choose, wrap, and Home and End go to the ends; the group is one tab stop", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => chosen(page)).toBe("de");
  expect(await focusedSegment(page)).toBe("de");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  expect((await lastRows(page)).filter).toEqual(DE);
  // Exactly one stop, on the chosen segment.
  expect((await segments(page)).filter((segment) => segment.tabindex === "0").map((s) => s.id)).toEqual(["de"]);

  await page.keyboard.press("ArrowDown");
  await expect.poll(() => chosen(page)).toBe("big");
  await page.keyboard.press("End");
  await expect.poll(() => chosen(page)).toBe("none");
  // Wraps round the end, and back.
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => chosen(page)).toBe("all");
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => chosen(page)).toBe("none");
  await page.keyboard.press("ArrowUp");
  await expect.poll(() => chosen(page)).toBe("big");
  await page.keyboard.press("Home");
  await expect.poll(() => chosen(page)).toBe("all");
  expect(await focusedSegment(page)).toBe("all");
  await expect.poll(() => status(page)).toMatch(/^200 matches/);

  // Tab leaves the group — it is not a stop per segment.
  await page.keyboard.press("Tab");
  expect(await focusedSegment(page)).toBeNull();
  expect(await shadow(page, (root) => !!root.activeElement?.closest('[part="presets"]'))).toBe(false);
  // And Shift+Tab comes back to the chosen one.
  await page.keyboard.press("Shift+Tab");
  expect(await focusedSegment(page)).toBe("all");
});

test("in a right-to-left page ArrowLeft is the next segment", async ({ page }) => {
  await open(page, "?rtl");
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => chosen(page)).toBe("de");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => chosen(page)).toBe("all");
});

test("a click chooses, and the segment keeps the focus", async ({ page }) => {
  await open(page);
  await page.locator('[part="preset"][data-preset="big"]').click();
  await expect.poll(() => chosen(page)).toBe("big");
  expect(await focusedSegment(page)).toBe("big");
  const big = await counted(page, BIG);
  expect(big).toBeGreaterThan(0);
  await expect.poll(() => status(page)).toBe(`${big} matches · Big`);
  // Choosing the one that is chosen asks nothing and says nothing.
  await clear(page);
  await page.locator('[part="preset"][data-preset="big"]').click();
  await page.waitForTimeout(150);
  expect(await queries(page)).toEqual([]);
});

test("the result is said once, with the preset's name", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    const line = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]');
    window.__said = [];
    new MutationObserver(() => {
      const text = line.textContent;
      if (window.__said.at(-1) !== text) window.__said.push(text);
    }).observe(line, { childList: true, characterData: true, subtree: true });
  });
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  // Scrolling and a second round of counts do not say it again.
  await shadow(page, (root) => (root.querySelector('[part="viewport"]').scrollTop = 200));
  await page.waitForTimeout(300);
  const said = await page.evaluate(() => window.__said);
  expect(said.filter((text) => text.includes("Germany"))).toEqual(["52 matches · Germany"]);
});

test("the preset is and-ed with the page's filter and the reader's, in that order", async ({ page }) => {
  await open(page);
  await page.evaluate((filter) => document.querySelector("opengrid-grid").setAttribute("filter", JSON.stringify(filter)), PAGE);
  await expect.poll(() => status(page)).toBe("150 matches");
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  const both = await counted(page, { and: [PAGE, DE] });
  await expect.poll(() => status(page)).toBe(`${both} matches · Germany`);
  expect((await lastRows(page)).filter).toEqual({ and: [PAGE, DE] });

  // The reader filters inside both, and the view says the preset still.
  await setView(page, { ...(await view(page)), filters: [{ column: "qty", op: "gte", value: "300" }] });
  const all = await counted(page, { and: [PAGE, DE, { field: "qty", op: "gte", value: 300 }] });
  expect(all).toBeGreaterThan(0);
  expect(all).toBeLessThan(both);
  await expect.poll(() => status(page)).toBe(`${all} matches`);
  const filter = (await lastRows(page)).filter;
  expect(filter.and[0]).toEqual(PAGE);
  expect(filter.and[1]).toEqual(DE);
  expect(JSON.stringify(filter.and[2])).toContain('"qty"');
  expect((await view(page)).preset).toBe("de");
});

test("it is in every query the grid sends: the rows and the facets' values and counts", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await clear(page);
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  await expect.poll(async () => (await queries(page)).filter(isGroups).length).toBeGreaterThan(1);
  const asked = await queries(page);
  expect(asked.filter(isRows).length).toBeGreaterThan(0);
  // The customer facet: its values (once, under the preset) and its counts.
  for (const query of asked) {
    expect(JSON.stringify(query.filter ?? null), JSON.stringify(query)).toContain(JSON.stringify(DE));
  }
  // What the facet lists is what the preset shows: no customer without a German order.
  const listed = () =>
    shadow(page, (root) => root.querySelectorAll('[part="facets"] [data-facet="customer"][data-key]').length);
  const german = await page.evaluate(() =>
    JSON.parse(
      window.__engine.execute(
        JSON.stringify({
          source: "orders",
          group: ["customer"],
          aggregate: [{ fn: "count", as: "n" }],
          select: ["customer", "n"],
          filter: { field: "country", op: "eq", value: "DE" },
        }),
      ),
    ).total_count,
  );
  await expect.poll(listed).toBe(german);
});

test("a facet lists the values of the preset's rows, not of the source", async ({ page }) => {
  await open(page);
  const listed = () =>
    shadow(page, (root) => root.querySelectorAll('[part="facets"] [data-facet="customer"][data-key]').length);
  // Nine customers in the source, the empty one among them.
  await expect.poll(listed).toBe(9);
  // France has orders of seven of them.
  const fr = { field: "country", op: "eq", value: "FR" };
  await setPresets(page, [...(await page.evaluate(() => window.__presets)), { id: "fr", label: "France", filter: fr }]);
  await page.locator('[part="preset"][data-preset="fr"]').click();
  await expect.poll(() => status(page)).toMatch(/^46 matches/);
  await expect.poll(listed).toBe(7);
  // And back: the list is the source's again.
  await page.locator('[part="preset"][data-preset="all"]').click();
  await expect.poll(listed).toBe(9);
  // A view that names the preset does the same.
  await setView(page, { ...(await view(page)), preset: "fr" });
  await expect.poll(listed).toBe(7);
});

test("choosing a preset starts at its first rows, wherever the reader had scrolled", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => (root.querySelector('[part="viewport"]').scrollTop = 100000));
  await expect
    .poll(() =>
      shadow(page, (root) => Math.min(...[...root.querySelectorAll("td[data-row]")].map((cell) => Number(cell.dataset.row)))),
    )
    .toBeGreaterThan(100);
  await focusSegment(page, "all");
  await clear(page);
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await queries(page)).filter(isRows).length).toBeGreaterThan(0);
  expect((await queries(page)).filter(isRows)[0].offset).toBe(0);
  await expect.poll(() => status(page)).toMatch(/^52 matches/);
});

test("a grouped grid asks for its groups, their total and their rows under it", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  await clear(page);
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("group-by", "customer"));
  await expect(page.locator("opengrid-grid tbody tr[aria-expanded]").first()).toBeVisible();
  // Open the first group: its rows are a query of their own.
  await page.locator("opengrid-grid tbody tr[aria-expanded]").first().click();
  await expect.poll(async () => (await queries(page)).filter(isRows).length).toBeGreaterThan(0);
  const asked = await queries(page);
  expect(asked.filter(isGroups).length).toBeGreaterThan(0);
  expect(asked.filter(isTotal).length).toBeGreaterThan(0);
  for (const query of asked) {
    expect(JSON.stringify(query.filter ?? null), JSON.stringify(query)).toContain(JSON.stringify(DE));
  }
  // Choosing another preset asks the groups again, under it.
  await clear(page);
  await focusSegment(page, "de");
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await queries(page)).filter(isGroups).length).toBeGreaterThan(0);
  for (const query of (await queries(page)).filter((query) => isGroups(query) || isTotal(query))) {
    expect(JSON.stringify(query.filter ?? null), JSON.stringify(query)).toContain(JSON.stringify(BIG));
    expect(JSON.stringify(query.filter ?? null)).not.toContain(JSON.stringify(DE));
  }
});

test("select all selects the rows of the preset, and only those", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  await clear(page);
  await shadow(page, (root) => root.querySelector('[data-select="all"]').click());
  await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.count)).toBe(52);
  const keys = await page.evaluate(() => window.__selections.at(-1).keys);
  const german = await page.evaluate(
    (filter) =>
      JSON.parse(
        window.__engine.execute(
          JSON.stringify({
            source: "orders",
            select: ["id"],
            filter,
            sort: [{ field: "id", direction: "asc" }],
            offset: 0,
            limit: 200,
          }),
        ),
      ).columns[0].values,
    DE,
  );
  expect([...keys].sort((a, b) => a - b)).toEqual(german);
  // The query that listed the keys was under the preset.
  const keyed = (await queries(page)).filter((query) => isRows(query) && query.limit > 20);
  expect(keyed.length).toBeGreaterThan(0);
  for (const query of keyed) {
    expect(query.filter).toEqual(DE);
  }
});

test("get_query, an export and the print table are the rows of the preset", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");

  const query = await page.evaluate(() => window.__opengridModule.get_query(document.querySelector("opengrid-grid")));
  expect(query.filter).toEqual(DE);

  const csv = await page.evaluate(async () => {
    const { exportRows } = await import("/packages/opengrid/loader.js");
    const query = window.__opengridModule.get_query(document.querySelector("opengrid-grid"));
    const blob = await exportRows(window.__provider, query, { format: "csv", bom: false });
    return blob.text();
  });
  const lines = csv.trim().split("\r\n");
  // The header and the 52 German orders — nobody else.
  expect(lines).toHaveLength(53);
  expect(lines.slice(1).every((line) => line.split(",")[2] === "DE")).toBe(true);

  const rows = await page.evaluate(() =>
    window.__opengridModule.prepare_print(document.querySelector("opengrid-grid")),
  );
  expect(rows).toBe(52);
  const caption = await page.evaluate(
    () =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector("[data-print]")
        .shadowRoot.querySelector("caption").textContent,
  );
  expect(caption).toBe("Orders · Germany · 52 matches");
});

test("it is in the view, and the view round-trips it", async ({ page }) => {
  await open(page);
  expect((await view(page)).preset).toBe("all");
  await page.evaluate(() => (window.__views.length = 0));
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  // One change event, with the view in it.
  expect(await page.evaluate(() => window.__views.map((view) => view.preset))).toEqual(["de"]);
  expect((await view(page)).preset).toBe("de");

  // Setting the view it has costs nothing and says nothing.
  const same = await view(page);
  await clear(page);
  await setView(page, same);
  await page.waitForTimeout(150);
  expect(await queries(page)).toEqual([]);
  expect(await page.evaluate(() => window.__views.length)).toBe(1);

  // A view names another: one query, the control follows, and it is said.
  await setView(page, { ...same, preset: "big" });
  const big = await counted(page, BIG);
  await expect.poll(() => status(page)).toBe(`${big} matches · Big`);
  expect(await chosen(page)).toBe("big");
  expect((await view(page)).preset).toBe("big");
  expect((await lastRows(page)).filter).toEqual(BIG);

  // A view without a preset is the default one.
  await setView(page, { sort: [{ field: "amount", direction: "desc" }] });
  await expect.poll(() => chosen(page)).toBe("all");
  expect((await view(page)).preset).toBe("all");
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
});

test("a view naming a preset the page has not is said and applied not at all", async ({ page }) => {
  await open(page);
  await setView(page, { sort: [{ field: "amount", direction: "desc" }], preset: "nope" });
  await expect.poll(async () => await shadow(page, (root) => root.querySelector('[part="status"]').getAttribute("data-state"))).toBe("error");
  expect(await status(page)).toContain("preset: nope is not a preset of this grid");
  // Nothing of it was applied: the sort is as it was, and so is the preset.
  expect((await view(page)).sort[0].field).not.toBe("amount");
  expect(await chosen(page)).toBe("all");
});

test("a selection by position is dropped when another preset is chosen, one by key is kept", async ({ page }) => {
  // By key (the fixture has a `row-key`): rows 1 and 2 stay selected.
  await open(page);
  await shadow(page, (root) => {
    root.querySelector('td[data-row="0"][data-col="0"]').focus();
  });
  await page.keyboard.press(" ");
  await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.count)).toBe(1);
  const key = await page.evaluate(() => window.__selections.at(-1).keys[0]);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  // The record is still selected, whether it is among the rows or not; the page
  // was not told it was dropped.
  expect((await view(page)).selected).toEqual([key]);
  expect(await page.evaluate(() => window.__selections.at(-1).count)).toBe(1);

  // By position (no key): gone, and said once.
  await open(page);
  await page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    host.removeAttribute("row-key");
  });
  await expect.poll(async () => await shadow(page, (root) => !!root.querySelector('td[data-row="0"][data-col="0"]'))).toBe(true);
  await page.evaluate(() => (window.__selections.length = 0));
  await shadow(page, (root) => root.querySelector('td[data-row="1"][data-col="0"]').focus());
  await page.keyboard.press(" ");
  await expect.poll(() => page.evaluate(() => window.__selections.at(-1)?.count)).toBe(1);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toContain("Germany");
  expect(await page.evaluate(() => window.__selections.at(-1).count)).toBe(0);
  expect(await status(page)).toContain("Selection cleared");
});

test("'Remove all' leaves the preset, and a preset is not a chip", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  expect(await chips(page)).toEqual([]);

  await setView(page, { ...(await view(page)), filters: [{ column: "qty", op: "gte", value: "300" }] });
  await expect.poll(async () => (await chips(page)).length).toBe(1);
  await shadow(page, (root) => root.querySelector('[part="chips-clear"]').click());
  await expect.poll(async () => (await chips(page)).length).toBe(0);
  await expect.poll(() => status(page)).toBe("52 matches · All filters removed");
  expect(await chosen(page)).toBe("de");
  expect((await view(page)).preset).toBe("de");
  expect((await lastRows(page)).filter).toEqual(DE);
});

test("the empty state names the preset, and its reset takes back the reader's filters only", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("End");
  await expect.poll(() => chosen(page)).toBe("none");
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('[part="status"]').getAttribute("data-state")))
    .toBe("empty");
  const empty = () =>
    shadow(page, (root) => ({
      shown: !root.querySelector('[part="empty"]').hidden,
      text: root.querySelector('[part="empty-text"]').textContent,
      reset: !root.querySelector('[part="empty-reset"]').hidden,
    }));
  // No filter of the reader's: nothing to reset, but the preset is named.
  expect(await empty()).toEqual({
    shown: true,
    text: "No row matches these filters in “Nothing”.",
    reset: false,
  });

  // With one, the reset is there — and it leaves the preset.
  await setView(page, { ...(await view(page)), filters: [{ column: "qty", op: "gte", value: "300" }] });
  await expect.poll(async () => (await empty()).reset).toBe(true);
  await shadow(page, (root) => root.querySelector('[part="empty-reset"]').click());
  await expect.poll(async () => (await empty()).reset).toBe(false);
  expect(await chosen(page)).toBe("none");
  expect((await empty()).text).toContain("“Nothing”");

  // A preset without a filter and no rows at all says the plain thing.
  await focusSegment(page, "none");
  await page.keyboard.press("Home");
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
  expect((await empty()).shown).toBe(false);
});

test("counts: one query per preset, in the label, refreshed with the result", async ({ page }) => {
  await open(page, "?counts");
  const de = await counted(page, DE);
  const big = await counted(page, BIG);
  await expect
    .poll(async () => (await segments(page)).map((segment) => segment.name))
    .toEqual(["All orders (200)", `Germany (${de})`, `Big (${big})`, "Nothing (0)"]);
  // One count per preset, each under its own filter and no other.
  const totals = (await queries(page)).filter(isTotal).map((query) => JSON.stringify(query.filter ?? null));
  expect([...new Set(totals)].sort()).toEqual(
    [null, DE, BIG, { field: "customer", op: "eq", value: "" }].map((filter) => JSON.stringify(filter)).sort(),
  );
  // The cost is said as the facets say theirs.
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('[part="presets-cost"]')?.textContent))
    .toBe("Counted with 4 queries");
  // The segment is named with its count, for a screen reader too.
  expect(await shadow(page, (root) => root.querySelector('[data-preset="de"]').textContent)).toBe(`Germany (${de})`);

  // The reader's filter moves them: the rows the preset would show.
  await setView(page, { ...(await view(page)), filters: [{ column: "qty", op: "gte", value: "300" }] });
  const deBig = await counted(page, { and: [DE, { field: "qty", op: "gte", value: 300 }] });
  await expect
    .poll(async () => (await segments(page)).find((segment) => segment.id === "de").name)
    .toBe(`Germany (${deBig})`);
  // Choosing keeps the counts, and the label is the name with its count.
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe(`${deBig} matches · Germany`);
  expect(await focusedSegment(page)).toBe("de");
});

test("the footer counts the presets' queries too", async ({ page }) => {
  const asked = async (query) => {
    await open(page, query);
    await expect.poll(() => status(page)).toMatch(/^200 matches/);
    // The count rounds are asynchronous: wait for the last one.
    await expect
      .poll(async () => (await shadow(page, (root) => root.querySelector('[part="source"]').textContent)).match(/\d+/)?.[0])
      .toBeTruthy();
    await page.waitForTimeout(300);
    return Number((await shadow(page, (root) => root.querySelector('[part="source"]').textContent)).match(/\d+/)[0]);
  };
  const plain = await asked("");
  const counted = await asked("?counts");
  expect(counted - plain).toBe(4);
});

test("without counts nothing is counted", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  expect((await queries(page)).filter(isTotal)).toEqual([]);
  expect((await segments(page)).map((segment) => segment.name)).toEqual(["All orders", "Germany", "Big", "Nothing"]);
  expect(await shadow(page, (root) => root.querySelector('[part="presets-cost"]'))).toBeNull();
});

test("a list that is refused is applied not at all, and said", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  await clear(page);
  for (const [bad, says] of [
    [[{ id: "a", label: "A" }, { id: "a", label: "Again" }], "presets[1]: a is the id of another preset"],
    [[{ id: "a" }], "presets[0] has no label"],
    [[{ label: "A" }], "presets[0] has no id"],
    [[{ id: "a", label: "A", flter: null }], "flter is not a field of a preset"],
    [[{ id: "a", label: "A", filter: { field: "x" } }], "presets[0]: the filter is not a filter"],
    [{ id: "a", label: "A" }, "presets is not a list"],
  ]) {
    await setPresets(page, bad);
    await expect
      .poll(() => shadow(page, (root) => root.querySelector('[part="status"]').getAttribute("data-state")))
      .toBe("error");
    expect(await status(page)).toContain(says);
    // The control is what it was.
    expect((await segments(page)).map((segment) => segment.id)).toEqual(["all", "de", "big", "none"]);
    expect(await chosen(page)).toBe("de");
    // Restore the status line with the next good call.
    await setPresets(page, await page.evaluate(() => window.__presets));
  }
  // Nothing was asked on account of a refused list.
  expect((await queries(page)).filter(isRows)).toEqual([]);
});

test("a new list keeps the choice where it can, and the rows follow the chosen filter", async ({ page }) => {
  await open(page);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toBe("52 matches · Germany");
  await page.evaluate(() => (window.__views.length = 0));

  // The same ids and the same filters: nothing is asked.
  await clear(page);
  await setPresets(page, await page.evaluate(() => window.__presets));
  await page.waitForTimeout(150);
  expect((await queries(page)).filter(isRows)).toEqual([]);

  // The chosen id with another filter: other rows.
  await setPresets(page, [
    { id: "all", label: "All orders" },
    { id: "de", label: "Germany", filter: { field: "country", op: "eq", value: "FR" } },
  ]);
  const fr = await counted(page, { field: "country", op: "eq", value: "FR" });
  await expect.poll(() => status(page)).toBe(`${fr} matches`);
  expect(await chosen(page)).toBe("de");
  expect(await page.evaluate(() => window.__views.length)).toBe(0);

  // A list without it: the default, and the page hears the view changed.
  await setPresets(page, [{ id: "all", label: "Everything" }]);
  await expect.poll(() => chosen(page)).toBe("all");
  await expect.poll(() => status(page)).toMatch(/^200 matches/);
  expect(await page.evaluate(() => window.__views.map((view) => view.preset))).toEqual(["all"]);

  // No presets: no control, no filter, no field in the view.
  await setPresets(page, []);
  await expect.poll(() => segments(page)).toEqual([]);
  expect((await view(page)).preset).toBeUndefined();
});

test("presets given before the grid is connected are taken when it is built", async ({ page }) => {
  await open(page);
  const result = await page.evaluate(async () => {
    const { module } = await window.__opengridReady.then(() => ({ module: window.__opengridModule }));
    const host = document.createElement("opengrid-grid");
    host.setAttribute("datasource", "orders");
    host.setAttribute("columns", "id,country");
    host.setAttribute("toolbar", "");
    module.set_presets(host, [
      { id: "all", label: "All" },
      { id: "de", label: "Germany", filter: { field: "country", op: "eq", value: "DE" } },
    ]);
    // Invalid ones are said when the grid is built, not thrown at the page.
    const bad = document.createElement("opengrid-grid");
    bad.setAttribute("datasource", "orders");
    bad.setAttribute("columns", "id,country");
    bad.setAttribute("toolbar", "");
    module.set_presets(bad, [{ id: "x" }]);
    const asked = [];
    for (const element of [host, bad]) {
      module.set_provider(element, {
        execute: (query) => {
          asked.push(query);
          return window.__engine.execute(query);
        },
      });
    }
    host.id = "late";
    bad.id = "bad";
    document.body.append(host, bad);
    return true;
  });
  expect(result).toBe(true);
  await expect
    .poll(() => page.evaluate(() => document.querySelector("#late").shadowRoot.querySelectorAll('[part="preset"]').length))
    .toBe(2);
  await expect
    .poll(() => page.evaluate(() => document.querySelector("#late").shadowRoot.querySelector('[part="status"]').textContent))
    .toBe("200 matches");
  expect(await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("#late")).preset)).toBe("all");
  // The refused list is said with the first result and none of it is applied.
  await expect
    .poll(() => page.evaluate(() => document.querySelector("#bad").shadowRoot.querySelector('[part="status"]').textContent))
    .toContain("presets[0] has no label");
  expect(await page.evaluate(() => document.querySelector("#bad").shadowRoot.querySelectorAll('[part="preset"]').length)).toBe(0);
});

test("connect takes presets and their options, and a view naming one", async ({ page }) => {
  await open(page);
  const outcome = await page.evaluate(async () => {
    const { connect } = await import("/packages/opengrid/loader.js");
    const host = document.createElement("opengrid-grid");
    host.setAttribute("datasource", "orders");
    host.setAttribute("columns", "id,country,qty");
    host.setAttribute("toolbar", "");
    host.id = "connected";
    document.body.append(host);
    const asked = [];
    const connection = connect(host, {
      presets: window.__presets,
      presetOptions: { counts: true },
      view: { preset: "de" },
      provider: {
        execute: (query) => {
          asked.push(query);
          return window.__engine.execute(query);
        },
      },
    });
    await connection.ready;
    return asked.length;
  });
  expect(outcome).toBeGreaterThan(0);
  await expect
    .poll(() => page.evaluate(() => document.querySelector("#connected").shadowRoot.querySelector('[part="status"]').textContent))
    .toBe("52 matches");
  const de = await counted(page, DE);
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.querySelector("#connected").shadowRoot.querySelector('[data-preset="de"]').textContent,
      ),
    )
    .toBe(`Germany (${de})`);
  expect(
    await page.evaluate(() =>
      document.querySelector("#connected").shadowRoot.querySelector('[aria-checked="true"]').getAttribute("data-preset"),
    ),
  ).toBe("de");
});

test("the segments stay inside the toolbar, whole and on one line, on a narrow grid", async ({ page }) => {
  // Four counted labels are wider than the toolbar here: the segments wrap as
  // whole segments, none is squeezed into two lines.
  await page.setViewportSize({ width: 360, height: 800 });
  await open(page, "?counts");
  await expect
    .poll(async () => (await segments(page)).map((segment) => segment.name))
    .toContain("Nothing (0)");
  const box = await shadow(page, (root) => {
    const rect = (element) => {
      const { left, right, height } = element.getBoundingClientRect();
      return { left, right, height };
    };
    const lines = (element) => {
      const range = document.createRange();
      range.selectNodeContents(element);
      return new Set([...range.getClientRects()].map((one) => Math.round(one.top))).size;
    };
    return {
      bar: rect(root.querySelector('[part="toolbar"]')),
      group: rect(root.querySelector('[part="presets"]')),
      buttons: [...root.querySelectorAll('[part="preset"]')].map((button) => ({ ...rect(button), lines: lines(button) })),
    };
  });
  expect(box.group.left).toBeGreaterThanOrEqual(box.bar.left - 1);
  expect(box.group.right).toBeLessThanOrEqual(box.bar.right + 1);
  for (const button of box.buttons) {
    expect(button.right).toBeLessThanOrEqual(box.bar.right + 1);
    expect(button.height).toBeGreaterThanOrEqual(24);
    expect(button.lines).toBe(1);
  }
  // No sideways scroll on the page for them.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test("no violations of axe with the presets drawn, counted and chosen", async ({ page }) => {
  await open(page, "?counts");
  await expect
    .poll(async () => (await segments(page)).map((segment) => segment.name))
    .toContain("Nothing (0)");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  await focusSegment(page, "all");
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => status(page)).toMatch(/Germany/);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  // The empty state under a preset.
  await page.keyboard.press("End");
  await expect.poll(() => chosen(page)).toBe("none");
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('[part="status"]').getAttribute("data-state")))
    .toBe("empty");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
