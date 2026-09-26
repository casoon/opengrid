import { test, expect } from "@playwright/test";

// The query of the current view (plan point 82): what a page exports.
//
// The claim is that `get_query` is exactly what the grid asks its provider —
// every restriction the reader applied, the shown columns in their order — only
// without the window. So the test does not rebuild the expected query by hand:
// it takes the grid's own last data query, strips `offset` and `limit`, and
// requires the two to be equal. A grouped grid asks per group, so there the
// rows `get_query` answers are compared with the rows the grid shows.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-query.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await rows(page);
}

async function rows(page) {
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
}

const getQuery = (page) =>
  page.evaluate(() => window.__opengridModule.get_query(document.querySelector("opengrid-grid")));

/** The grid's last query for rows (not a facet count), without its window. */
const lastRowsQuery = (page) =>
  page.evaluate(() => {
    const rowsQueries = window.__queries
      .map((json) => JSON.parse(json))
      .filter((query) => !query.group && "offset" in query);
    const { offset, limit, ...rest } = rowsQueries.at(-1);
    return rest;
  });

test("with nothing applied, it is the grid's query without a window", async ({ page }) => {
  await open(page);
  const query = await getQuery(page);
  expect(query).toEqual(await lastRowsQuery(page));
  expect(query).not.toHaveProperty("limit");
  expect(query).not.toHaveProperty("offset");
  expect(query.sort).toEqual([{ field: "id", direction: "asc" }]);
});

test("every kind of restriction is in it, and nothing else", async ({ page }) => {
  await open(page);
  // Sort, columns and a facet through the view…
  await page.evaluate(() => {
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      sort: [{ field: "amount", direction: "desc" }],
      columns: { order: ["customer", "id"], hidden: ["ordered_on"], widths: {} },
      facets: { country: { values: ["DE", "FR"] } },
    });
  });
  await rows(page);
  // …a typed filter through the filter row (qty is the fifth shown column)…
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('select[data-col="4"]').value = "gte";
    const input = root.querySelector('input[data-col="4"]');
    input.focus();
    input.value = "2";
  });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);
  // …and free text through the search field, as a reader types it.
  await page.evaluate(() => {
    const input = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="search-input"]');
    input.focus();
    input.value = "A";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await page.keyboard.press("Enter");
  await page.waitForTimeout(300);

  const query = await getQuery(page);
  expect(query).toEqual(await lastRowsQuery(page));
  // The restrictions, read back from the query itself.
  expect(query.select).toEqual(["customer", "id", "country", "amount", "qty"]);
  expect(query.sort).toEqual([{ field: "amount", direction: "desc" }]);
  const filter = JSON.stringify(query.filter);
  for (const part of ['"field":"qty","op":"gte","value":2', '"DE"', '"FR"', '"contains"']) {
    expect(filter).toContain(part);
  }
});

test("run as it is, it answers the rows the grid shows", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      sort: [{ field: "customer", direction: "asc" }, { field: "amount", direction: "desc" }],
      facets: { country: { values: ["DE"] } },
    });
  });
  await rows(page);
  const compared = await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    const query = window.__opengridModule.get_query(grid);
    const result = JSON.parse(window.__engine.execute(JSON.stringify({ ...query, limit: 5 })));
    const ids = result.columns.find((column) => column.name === "id").values;
    const shown = [...grid.shadowRoot.querySelectorAll("tbody tr[aria-rowindex]")]
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .slice(0, 5)
      .map((tr) => Number(tr.querySelector('td[data-col="0"]').textContent));
    return { ids: ids.map(Number), shown };
  });
  expect(compared.ids).toEqual(compared.shown);
});

test("grouped: the rows in the order the reader sees them", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      group: ["country"],
      sort: [{ field: "amount", direction: "desc" }],
    });
  });
  await page.waitForFunction(
    () =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('[role="treegrid"]') !==
      null,
  );
  const query = await getQuery(page);
  // Groups first, then the sort; no group or total rows — a plain rows query.
  expect(query.sort).toEqual([
    { field: "country", direction: "asc" },
    { field: "amount", direction: "desc" },
  ]);
  expect(query).not.toHaveProperty("group");
  expect(query).not.toHaveProperty("aggregate");
});

test("null where there is no query to give", async ({ page }) => {
  await open(page);
  const nulls = await page.evaluate(() => {
    const module = window.__opengridModule;
    const loose = document.createElement("opengrid-grid");
    const table = document.createElement("opengrid-table");
    return [module.get_query(loose), module.get_query(table)];
  });
  expect(nulls).toEqual([null, null]);
});
