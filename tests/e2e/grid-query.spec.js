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

/**
 * Waits until the grid has asked for rows under a filter that contains `part`
 * — the sign that a restriction reached the query, rather than a guessed time.
 */
const askedWith = (page, part) =>
  page.waitForFunction(
    (part) =>
      window.__queries
        .map((json) => JSON.parse(json))
        .some(
          (query) =>
            !query.group && "offset" in query && JSON.stringify(query.filter ?? null).includes(part),
        ),
    part,
  );

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

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
  await askedWith(page, '"field":"qty","op":"gte","value":2');
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
  await askedWith(page, '"contains"');

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
  // Groups first — NULL last said explicitly, as the group query says it —
  // then the sort; no group or total rows — a plain rows query.
  expect(query.sort).toEqual([
    { field: "country", direction: "asc", nulls: "last" },
    { field: "amount", direction: "desc" },
  ]);
  expect(query).not.toHaveProperty("group");
  expect(query).not.toHaveProperty("aggregate");
});

/**
 * Applies a grouped `view` with every group open, in one window so every row is
 * drawn, and answers the ids of the drawn data rows (no group or total rows)
 * next to what `get_query` answers with a `limit` of as many.
 */
async function groupedRows(page, view) {
  await page.evaluate((view) => {
    const grid = document.querySelector("opengrid-grid");
    // The data rows as drawn, in display order: the group and total rows
    // carry a `data-kind`, rows outside the window no transform.
    window.__drawnIds = () =>
      [...grid.shadowRoot.querySelectorAll("tbody tr")]
        .filter((tr) => tr.style.transform && !tr.style.display && !tr.dataset.kind)
        .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
        .map((tr) => tr.querySelector('td[data-col="0"]').textContent);
    // Every row in one window: 200 rows and their group rows fit in 400.
    grid.setAttribute("window-size", "400");
    // Every path of the grouping, open — taken from the data itself.
    const result = JSON.parse(
      window.__engine.execute(JSON.stringify({ source: "orders", select: view.group })),
    );
    const paths = new Map();
    for (let row = 0; row < result.row_count; row += 1) {
      const keys = result.columns.map((column) => column.values[row]);
      for (let depth = 1; depth <= keys.length; depth += 1) {
        paths.set(JSON.stringify(keys.slice(0, depth)), keys.slice(0, depth));
      }
    }
    window.__opengridModule.set_view(grid, { ...view, expanded: [...paths.values()] });
  }, view);
  await page.waitForFunction(() => {
    const ids = window.__drawnIds();
    return ids.length === 200 && ids.every((text) => text !== "");
  });
  return page.evaluate(() => {
    const query = window.__opengridModule.get_query(document.querySelector("opengrid-grid"));
    const shown = window.__drawnIds().map(Number);
    const result = JSON.parse(
      window.__engine.execute(JSON.stringify({ ...query, limit: shown.length })),
    );
    const column = (name) => result.columns.find((column) => column.name === name).values;
    return { query, shown, ids: column("id").map(Number), countries: column("country") };
  });
}

test("grouped and expanded: the rows are the ones the grid draws, in its order", async ({ page }) => {
  await open(page);
  const { query, shown, ids, countries } = await groupedRows(page, {
    group: ["country"],
    sort: [{ field: "amount", direction: "desc" }],
  });
  expect(ids).toEqual(shown);
  expect(query.sort[0]).toEqual({ field: "country", direction: "asc", nulls: "last" });
  // The six rows without a country are a group of their own, and the last.
  expect(countries.slice(-6)).toEqual([null, null, null, null, null, null]);
  expect(countries.slice(0, -6)).not.toContain(null);
});

test("grouped by the column the grid is sorted by: the groups still ascend", async ({ page }) => {
  await open(page);
  // The grid orders its groups ascending whatever the sort says about the
  // column; `get_query` follows the grid and drops the sort's own key.
  const { query, shown, ids } = await groupedRows(page, {
    group: ["country"],
    sort: [{ field: "country", direction: "desc" }],
  });
  expect(ids).toEqual(shown);
  expect(query.sort).toEqual([{ field: "country", direction: "asc", nulls: "last" }]);
});

test("grouped on two levels, NULL keys on both: the rows the grid draws", async ({ page }) => {
  await open(page);
  const { query, shown, ids } = await groupedRows(page, {
    group: ["country", "customer"],
    sort: [{ field: "id", direction: "desc" }],
  });
  expect(ids).toEqual(shown);
  expect(query.sort).toEqual([
    { field: "country", direction: "asc", nulls: "last" },
    { field: "customer", direction: "asc", nulls: "last" },
    { field: "id", direction: "desc" },
  ]);
});

test("null where there is no query to give", async ({ page }) => {
  await open(page);
  const nulls = await page.evaluate(() => {
    const module = window.__opengridModule;
    const loose = document.createElement("opengrid-grid");
    const table = document.createElement("opengrid-table");
    // In the document, with columns, but without a source to ask.
    const sourceless = document.createElement("opengrid-grid");
    sourceless.setAttribute("columns", "id,customer");
    const pivot = document.createElement("opengrid-pivot");
    pivot.setAttribute("datasource", "orders");
    pivot.setAttribute("rows", "country");
    document.body.append(sourceless, pivot);
    return [loose, table, sourceless, pivot].map((host) => module.get_query(host));
  });
  expect(nulls).toEqual([null, null, null, null]);
});

test("null for a filter that does not hold — grouped or not, as the grid says", async ({ page }) => {
  await open(page);
  // A range bound that is not a value of its column: the status line names it,
  // and there is no query to export.
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      facets: { amount: { min: "abc", max: "" } },
    }),
  );
  await expect.poll(() => status(page)).toContain("abc");
  expect(await getQuery(page)).toBeNull();

  // Grouped, the same sentence: the grid does not fall back to the filter row
  // alone and show rows `get_query` would not export.
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      group: ["country"],
      facets: { amount: { min: "abc", max: "" } },
    }),
  );
  await expect.poll(() => status(page)).toContain("abc");
  expect(await getQuery(page)).toBeNull();
});
