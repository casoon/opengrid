import { test, expect } from "@playwright/test";

// A view whose filter is on a column that is not text (plan point 88).
//
// Point 59 tested `set_view` with `country eq "DE"` — a text column, where a
// string literal is right by accident. On any other type the filter has to
// become that type's literal, the way the filter row types it (point 51):
// a saved view with `qty ≥ 2` or `ordered_on ≥ 2025-01-01` is the normal case
// of the view tabs of Phase F and of `v-model:view`/`bind:view`.
//
// The fixture also configures a range and a period facet, which the presentation
// check must still accept after the view was applied.

const CASES = [
  { column: "qty", op: "gte", value: "500", literal: 500 },
  { column: "ratio", op: "lt", value: "0", literal: 0 },
  // Decimal and timestamp literals are strings on the wire — as typed, like the
  // filter row sends them; these two held before and are here so they keep.
  { column: "amount", op: "gte", value: "100000000", literal: "100000000" },
  { column: "ordered_on", op: "gte", value: "2026-01-01", literal: "2026-01-01" },
  { column: "created_at", op: "lt", value: "2025-06-01T00:00:00Z", literal: "2025-06-01T00:00:00Z" },
];

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-view-typed.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
}

const status = (page) =>
  page.evaluate(() => {
    const line = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]');
    return { state: line.getAttribute("data-state"), text: line.textContent };
  });

for (const { column, op, value, literal } of CASES) {
  test(`a view filtering ${column} (${op} ${value}) is applied with a typed literal`, async ({
    page,
  }) => {
    await open(page);
    await page.evaluate(() => {
      window.__queries.length = 0;
    });
    await page.evaluate(
      (filter) =>
        window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
          filters: [filter],
        }),
      { column, op, value },
    );
    await expect.poll(async () => (await status(page)).state).toBe("ready");
    expect((await status(page)).text).toMatch(/match(es)?$/);

    const sent = await page.evaluate(() =>
      window.__queries
        .map((json) => JSON.parse(json))
        .filter((query) => !query.group && "offset" in query)
        .at(-1),
    );
    expect(JSON.stringify(sent.filter)).toContain(
      JSON.stringify({ field: column, op, value: literal }).slice(1, -1),
    );
    // And the view reads back as it was given.
    const view = await page.evaluate(() =>
      window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
    );
    expect(view.filters).toEqual([{ column, op, value }]);
  });
}

/**
 * A fresh grid in the page with a view before its first result — the order
 * `connect` uses on mount — then `between(grid)` right after the provider.
 */
async function beforeFirstResult(page, { presentation = false, between = "" } = {}) {
  return page.evaluate(
    async ({ presentation, between }) => {
      const module = window.__opengridModule;
      const grid = document.createElement("opengrid-grid");
      for (const [name, value] of [
        ["label", "Später"],
        ["datasource", "orders"],
        ["columns", "id,qty,amount,ordered_on"],
        ["facets", ""],
      ]) {
        grid.setAttribute(name, value);
      }
      document.querySelector("main").append(grid);
      if (presentation) {
        module.set_columns(grid, { amount: { facet: "range" }, ordered_on: { facet: "period" } });
      }
      module.set_view(grid, { filters: [{ column: "qty", op: "gte", value: "500" }] });
      window.__queries.length = 0;
      module.set_provider(grid, window.__provider());
      if (between === "density") grid.setAttribute("density", "compact");
      const line = () => grid.shadowRoot.querySelector('[part="status"]');
      for (let i = 0; i < 50 && line().getAttribute("data-state") !== "ready"; i += 1) {
        if (line().getAttribute("data-state") === "error") break;
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      await new Promise((resolve) => setTimeout(resolve, 200));
      return {
        state: line().getAttribute("data-state"),
        text: line().textContent,
        queries: window.__queries.map((json) => JSON.parse(json)),
        filters: module.get_view(grid).filters,
      };
    },
    { presentation, between },
  );
}

/** The queries that asked for rows (not a probe, not a facet count). */
const rowQueries = (queries) =>
  queries.filter((query) => !query.group && "offset" in query && query.limit !== 0);

test("a view with a typed filter before the first result asks for the types first", async ({
  page,
}) => {
  // What `connect` does on mount: the view before the provider, so no result
  // has typed a column yet. The grid asks for the schema with `limit 0` (no
  // filter, so it cannot fail on the literal), types the filter, then asks.
  await open(page);
  const outcome = await beforeFirstResult(page);
  expect(outcome.state).toBe("ready");
  const probes = outcome.queries.filter((query) => query.limit === 0);
  expect(probes).toHaveLength(1);
  expect(probes[0]).not.toHaveProperty("filter");
  expect(outcome.queries.indexOf(probes[0])).toBe(0);
  for (const query of rowQueries(outcome.queries)) {
    expect(JSON.stringify(query.filter)).toContain('"field":"qty","op":"gte","value":500');
  }
});

test("a query that comes in while the types are asked for waits for them too", async ({
  page,
}) => {
  // The density changes right after the provider — a query while the probe is
  // out. It must not send the text literal (review of point 88).
  await open(page);
  const outcome = await beforeFirstResult(page, { between: "density" });
  expect(outcome.state).toBe("ready");
  for (const query of rowQueries(outcome.queries)) {
    expect(JSON.stringify(query.filter)).toContain('"value":500');
  }
});

test("with facets configured, the view's filter survives the first result", async ({ page }) => {
  // The first typed result lets the range and period facets through, and the
  // skeleton is rebuilt for them — around the same state: the filter stays
  // (review of point 88; before, it was silently dropped).
  await open(page);
  const outcome = await beforeFirstResult(page, { presentation: true });
  expect(outcome.state).toBe("ready");
  expect(outcome.filters).toEqual([{ column: "qty", op: "gte", value: "500" }]);
  const rows = rowQueries(outcome.queries);
  expect(rows.length).toBeGreaterThan(0);
  expect(JSON.stringify(rows.at(-1).filter)).toContain('"field":"qty","op":"gte","value":500');
});

test("a literal the column cannot hold is said, and the view is not applied", async ({
  page,
}) => {
  await open(page);
  const before = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  await page.evaluate(() => {
    window.__queries.length = 0;
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      sort: [{ field: "amount", direction: "desc" }],
      filters: [{ column: "qty", op: "gte", value: "abc" }],
    });
  });
  await expect.poll(async () => (await status(page)).state).toBe("error");
  expect((await status(page)).text).toContain("abc");
  expect(await page.evaluate(() => window.__queries.length)).toBe(0);
  const after = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  expect(after).toEqual(before);
});
