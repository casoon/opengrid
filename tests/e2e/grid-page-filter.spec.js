import { test, expect } from "@playwright/test";

// The page's filter on `<opengrid-grid>` (issue #330): the `filter` attribute,
// a wire filter and-ed onto every query the grid sends.
//
// It is the page's and not the reader's, so the tests look at both sides: the
// queries — the fixture records every one the grid asks — and everything a
// reader's filter would show up in, where this one must not: the chips, the
// view, the view-change event, "remove all", the empty state's reset.
//
// The fixture of `grid-query.spec.js`: 200 orders with a toolbar, the search
// and all four kinds of facet. 52 of them are from DE.

const DE = { field: "country", op: "eq", value: "DE" };

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-query.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
  await page.evaluate(() => {
    window.__viewChanges = 0;
    document
      .querySelector("opengrid-grid")
      .addEventListener("opengrid-view-change", () => (window.__viewChanges += 1));
  });
}

/** Sets the attribute — an object as JSON, a string as written, `null` removes it. */
const setFilter = (page, filter) =>
  page.evaluate((filter) => {
    const grid = document.querySelector("opengrid-grid");
    window.__queries.length = 0;
    if (filter === null) grid.removeAttribute("filter");
    else grid.setAttribute("filter", typeof filter === "string" ? filter : JSON.stringify(filter));
  }, filter);

const status = (page) =>
  page.evaluate(() => {
    const line = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]');
    return { state: line.getAttribute("data-state"), text: line.textContent.trim() };
  });

/** Every query asked since the last `setFilter`, parsed. */
const queries = (page) => page.evaluate(() => window.__queries.map((json) => JSON.parse(json)));

/** The grid's last query for rows. */
const lastRows = async (page) =>
  (await queries(page)).filter((query) => !query.group && "offset" in query).at(-1);

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

const chips = (page) =>
  page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="chip"]')].map(
      (chip) => chip.textContent,
    ),
  );

test("the filter is in every query the grid sends, the facets' included", async ({ page }) => {
  await open(page);
  await setFilter(page, DE);
  await expect.poll(async () => (await status(page)).text).toBe("52 matches");
  // The rows, and each facet's values and counts: nothing asks past it.
  await expect.poll(async () => (await queries(page)).filter((query) => query.group).length).toBeGreaterThan(2);
  const asked = await queries(page);
  expect(asked.length).toBeGreaterThan(3);
  for (const query of asked) {
    expect(JSON.stringify(query.filter ?? null), JSON.stringify(query)).toContain(JSON.stringify(DE));
  }
  expect((await lastRows(page)).filter).toEqual(DE);
  // And the export is the same rows.
  const exported = await page.evaluate(() =>
    window.__opengridModule.get_query(document.querySelector("opengrid-grid")),
  );
  expect(exported.filter).toEqual(DE);
});

test("a grouped grid asks for its groups, their totals and their rows under it", async ({ page }) => {
  await open(page);
  await setFilter(page, DE);
  await expect.poll(async () => (await status(page)).text).toBe("52 matches");
  await page.evaluate(() => {
    window.__queries.length = 0;
    document.querySelector("opengrid-grid").setAttribute("group-by", "customer");
  });
  await expect(page.locator("opengrid-grid tbody tr[aria-expanded]").first()).toBeVisible();
  const asked = await queries(page);
  expect(asked.filter((query) => query.group).length).toBeGreaterThan(0);
  for (const query of asked) {
    expect(JSON.stringify(query.filter ?? null), JSON.stringify(query)).toContain(JSON.stringify(DE));
  }
});

test("the reader filters inside it, and removing all filters leaves it", async ({ page }) => {
  await open(page);
  await setFilter(page, DE);
  await expect.poll(async () => (await status(page)).text).toBe("52 matches");

  const qty = { field: "qty", op: "gte", value: 500 };
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      filters: [{ column: "qty", op: "gte", value: "500" }],
    }),
  );
  const both = await counted(page, { and: [DE, qty] });
  expect(both).toBeGreaterThan(0);
  expect(both).toBeLessThan(52);
  await expect.poll(async () => (await status(page)).text).toBe(`${both} matches`);
  const filter = (await lastRows(page)).filter;
  expect(filter.and[0]).toEqual(DE);
  expect(JSON.stringify(filter.and[1])).toContain(JSON.stringify(qty));
  // One chip: the reader's. The page's filter is not theirs to see or remove.
  expect(await chips(page)).toHaveLength(1);

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="chips-clear"]').click(),
  );
  await expect.poll(async () => (await status(page)).text).toBe("52 matches · All filters removed");
  expect((await lastRows(page)).filter).toEqual(DE);
  expect(await chips(page)).toEqual([]);
});

test("it is not in the view, and changing it is not a view change", async ({ page }) => {
  await open(page);
  const before = await page.evaluate(() =>
    JSON.stringify(window.__opengridModule.get_view(document.querySelector("opengrid-grid"))),
  );
  await setFilter(page, DE);
  await expect.poll(async () => (await status(page)).text).toBe("52 matches");
  await setFilter(page, { not: DE });
  // Not 148: an order without a country is neither DE nor not DE (S1).
  const others = await counted(page, { not: DE });
  expect(others).toBeGreaterThan(0);
  await expect.poll(async () => (await status(page)).text).toBe(`${others} matches`);
  const after = await page.evaluate(() =>
    JSON.stringify(window.__opengridModule.get_view(document.querySelector("opengrid-grid"))),
  );
  expect(after).toBe(before);
  expect(await page.evaluate(() => window.__viewChanges)).toBe(0);
  expect(await chips(page)).toEqual([]);

  // A view the page applies does not take it away either.
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      sort: [{ field: "amount", direction: "desc" }],
    }),
  );
  await expect.poll(async () => (await lastRows(page))?.sort?.[0]?.field).toBe("amount");
  expect((await lastRows(page)).filter).toEqual({ not: DE });
  await setFilter(page, null);
  await expect.poll(async () => (await status(page)).text).toBe("200 matches");
  expect((await lastRows(page)).filter).toBeUndefined();
});

test("the empty string, NULL, `or` and `not` reach the source as written", async ({ page }) => {
  await open(page);
  const nested = {
    or: [
      { field: "flag", op: "is_null" },
      { and: [{ not: { field: "qty", op: "gte", value: 100 } }, DE] },
    ],
  };
  await setFilter(page, nested);
  const expected = await counted(page, nested);
  expect(expected).toBeGreaterThan(0);
  expect(expected).toBeLessThan(200);
  await expect.poll(async () => (await status(page)).text).toBe(`${expected} matches`);
  expect((await lastRows(page)).filter).toEqual(nested);

  // The empty string is a value — not "no filter", and not NULL. No order has
  // an empty customer, so nothing matches; and since no filter of the reader's
  // caused that, there is no reset to offer.
  const empty = { field: "customer", op: "eq", value: "" };
  await setFilter(page, empty);
  await expect.poll(async () => (await status(page)).state).toBe("empty");
  expect((await lastRows(page)).filter).toEqual(empty);
  const reset = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      panel: !root.querySelector('[part="empty"]').hidden,
      reset: !root.querySelector('[part="empty-reset"]').hidden,
    };
  });
  expect(reset).toEqual({ panel: true, reset: false });
});

test("changing it drops the selection, as a filter of the reader's does", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    window.__selection = [];
    document.addEventListener("opengrid-selection-change", (event) => window.__selection.push(event.detail.count));
    document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-row="2"][data-col="0"]').focus();
  });
  await page.keyboard.press(" ");
  await expect.poll(() => page.evaluate(() => window.__selection.at(-1))).toBe(1);

  await setFilter(page, DE);
  await expect.poll(async () => (await status(page)).text).toMatch(/^52 matches/);
  // Other records sit at those positions now: the page hears that none is selected.
  expect(await page.evaluate(() => window.__selection.at(-1))).toBe(0);
});

test("what is not a filter is said, and nothing is asked", async ({ page }) => {
  await open(page);
  await setFilter(page, "country = DE");
  await expect.poll(async () => (await status(page)).state).toBe("error");
  expect((await status(page)).text).toContain("the filter attribute is not a filter");
  expect(await queries(page)).toEqual([]);
  expect(
    await page.evaluate(() => window.__opengridModule.get_query(document.querySelector("opengrid-grid"))),
  ).toBeNull();

  // A filter object the source refuses is the source's error, said the same way.
  await setFilter(page, { field: "nowhere", op: "eq", value: 1 });
  await expect.poll(async () => (await lastRows(page)) !== undefined).toBe(true);
  await expect.poll(async () => (await status(page)).state).toBe("error");
  expect((await status(page)).text).toContain("nowhere");

  // And a filter that holds brings the grid back.
  await setFilter(page, DE);
  await expect.poll(async () => (await status(page)).text).toBe("52 matches");
});
