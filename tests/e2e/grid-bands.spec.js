import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Column bands (issue #319): a header above several columns.
//
// What is under test is the contract of docs/api.md §Column bands: a second
// header row with the bands across their columns, what a data cell names as
// its headers, the keys through a header whose cells span, a column moving
// inside its band and a band moving, hiding and being pinned as a whole — in
// both reading directions.

/** Runs `fn(root, arg)` in the shadow root of the grid `selector` names. */
const inside = (page, selector, fn, arg) =>
  page.evaluate(
    ({ selector, fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(document.querySelector(selector).shadowRoot, arg),
    { selector, fn: fn.toString(), arg },
  );

const status = (page, selector) => inside(page, selector, (root) => root.querySelector('[part="status"]').textContent);

/** The column names in the order the columns stand, whichever header row holds them. */
const columns = (page, selector) =>
  inside(page, selector, (root) =>
    [...root.querySelectorAll("thead th[data-col]")]
      .sort((a, b) => a.dataset.col - b.dataset.col)
      .map((th) => th.querySelector("span").textContent),
  );

/** The bands, each with the names of the columns its cell spans. */
const bands = (page, selector) =>
  inside(page, selector, (root) =>
    [...root.querySelectorAll('thead th[part="band"]')].map((th) => [
      th.textContent,
      th.dataset.band.split(" ").map((col) => root.querySelector(`th[data-col="${col}"] span`).textContent),
    ]),
  );

const settled = (page, selector) =>
  page.waitForFunction((selector) => {
    const root = document.querySelector(selector)?.shadowRoot;
    const line = root?.querySelector('[part="status"]');
    return !!line && line.getAttribute("data-state") !== "loading" && !!root.querySelector("td[data-row]");
  }, selector);

const focusHeader = (page, selector, name) =>
  inside(
    page,
    selector,
    (root, name) =>
      [...root.querySelectorAll("thead th")].find((th) => th.querySelector("span")?.textContent === name).focus(),
    name,
  );

/**
 * The cell that has the focus: `band:` and its name, `header:` and its name,
 * `select-all`, or `cell:` with its row and the name of its column.
 */
const focused = (page, selector) =>
  inside(page, selector, (root) => {
    const active = root.activeElement;
    if (!active) return null;
    if (active.closest("[data-select='all']")) return "select-all";
    const cell = active.closest("th, td");
    if (cell.part.contains("band")) return `band:${cell.textContent}`;
    const name = root.querySelector(`th[data-col="${cell.dataset.col}"] span`).textContent;
    return cell.tagName === "TH" ? `header:${name}` : `cell:${cell.dataset.row}:${name}`;
  });

/** How many cells of the table are in the tab sequence: one, always. */
const tabStops = (page, selector) =>
  inside(page, selector, (root) => root.querySelectorAll('table [tabindex="0"]').length);

const queries = (page, id) => page.evaluate((id) => window.__queries.filter((query) => query.id === id).length, id);
const views = (page, id) =>
  page.evaluate((id) => window.__views.filter((view) => view.id === id).map((view) => view.columns), id);

const setColumns = async (page, selector, change) => {
  await page.evaluate(
    ({ selector, change }) => {
      const columns = structuredClone(window.__columns);
      for (const [name, options] of Object.entries(change)) Object.assign(columns[name], options);
      window.__opengridModule.set_columns(document.querySelector(selector), columns);
    },
    { selector, change },
  );
};

const scrollTo = async (page, selector, left) => {
  await inside(page, selector, (root, left) => (root.querySelector('[part~="viewport"]').scrollLeft = left), left);
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
};

/** Where the header cells, the cells of the first row and the filter fields stand, by name. */
const layout = (page, selector) =>
  inside(page, selector, (root) => {
    const box = (element) => {
      const rect = element.getBoundingClientRect();
      return [Math.round(rect.left), Math.round(rect.right)];
    };
    const viewport = root.querySelector('[part~="viewport"]');
    const row = root.querySelector('td[data-row="0"]').closest("tr");
    const edge = viewport.getBoundingClientRect().left + viewport.clientLeft;
    return {
      viewport: [Math.round(edge), Math.round(edge + viewport.clientWidth)],
      bands: Object.fromEntries(
        [...root.querySelectorAll('th[part="band"]')].map((th) => [
          th.textContent,
          { header: box(th), sticky: getComputedStyle(th).position === "sticky" },
        ]),
      ),
      columns: Object.fromEntries(
        [...root.querySelectorAll("th[data-col]")].map((th) => [
          th.querySelector("span").textContent,
          {
            header: box(th),
            cell: box(row.querySelector(`td[data-col="${th.dataset.col}"]`)),
            field: box(root.querySelector(`[part="filter-operator"][data-col="${th.dataset.col}"]`).parentElement),
            sticky: getComputedStyle(th).position === "sticky",
          },
        ]),
      ),
    };
  });

const near = (a, b) => Math.abs(a[0] - b[0]) <= 1 && Math.abs(a[1] - b[1]) <= 1;

const ORDER = ["id", "customer", "country", "amount", "qty", "ordered_on"];

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-bands.html");
  await page.waitForFunction(() => window.__opengridReady);
  for (const selector of ["#banded", "#rtl"]) await settled(page, selector);
  // The page's presentation is checked against the types of the first result
  // and drawn again; wait for that second answer.
  await page.waitForFunction(() =>
    ["banded", "rtl"].every((id) => window.__queries.filter((query) => query.id === id).length >= 2),
  );
  for (const selector of ["#banded", "#rtl"]) await settled(page, selector);
});

test("the header has two rows: a band across its columns, the others across both", async ({ page }) => {
  for (const selector of ["#banded", "#rtl"]) {
    expect(await columns(page, selector)).toEqual(ORDER);
    expect(await bands(page, selector)).toEqual([
      ["Customer", ["customer", "country"]],
      ["Order", ["amount", "qty"]],
    ]);
    const header = await inside(page, selector, (root) => {
      const cell = (th) => ({
        name: th.querySelector("span:not([role])")?.textContent ?? "select-all",
        scope: th.scope,
        colSpan: th.colSpan,
        rowSpan: th.rowSpan,
      });
      return {
        rows: [...root.querySelectorAll("thead tr")].map((tr) => ({
          index: tr.getAttribute("aria-rowindex"),
          cells: [...tr.children].map(cell),
        })),
        rowcount: root.querySelector("table").getAttribute("aria-rowcount"),
        colcount: root.querySelector("table").getAttribute("aria-colcount"),
        first: root.querySelector('td[data-row="0"]').closest("tr").getAttribute("aria-rowindex"),
      };
    });
    expect(header.rows).toEqual([
      {
        index: "1",
        cells: [
          { name: "select-all", scope: "col", colSpan: 1, rowSpan: 2 },
          { name: "id", scope: "col", colSpan: 1, rowSpan: 2 },
          { name: "Customer", scope: "colgroup", colSpan: 2, rowSpan: 1 },
          { name: "Order", scope: "colgroup", colSpan: 2, rowSpan: 1 },
          { name: "ordered_on", scope: "col", colSpan: 1, rowSpan: 2 },
        ],
      },
      {
        index: "2",
        cells: ["customer", "country", "amount", "qty"].map((name) => ({ name, scope: "col", colSpan: 1, rowSpan: 1 })),
      },
    ]);
    // Both header rows are counted: 200 rows and two, the first data row third.
    expect([header.rowcount, header.colcount, header.first]).toEqual(["202", "7", "3"]);

    // On the screen: a band stands exactly above its columns, and every
    // column's header above its cells — a column under a band has the width
    // the page gave it, although the first header row no longer says it.
    const at = await layout(page, selector);
    for (const name of ORDER) {
      expect(near(at.columns[name].header, at.columns[name].cell), name).toBe(true);
      // The filter row has one field per column, under its header.
      expect(near(at.columns[name].header, at.columns[name].field), name).toBe(true);
    }
    const span = (first, last) => {
      const edges = [...at.columns[first].header, ...at.columns[last].header];
      return [Math.min(...edges), Math.max(...edges)];
    };
    expect(near(at.bands.Customer.header, span("customer", "country"))).toBe(true);
    expect(near(at.bands.Order.header, span("amount", "qty"))).toBe(true);
    const width = ([left, right]) => right - left;
    expect(["customer", "country", "amount"].map((name) => width(at.columns[name].header))).toEqual([136, 150, 130]);
  }
});

test("every header cell says its column: aria-colindex is the place it has on the screen", async ({ page }) => {
  // The cells of the two header rows are not the columns in DOM order, so the
  // index is said — and it has to be the column the cell starts in.
  for (const selector of ["#banded", "#rtl"]) {
    const cells = await inside(page, selector, (root) => {
      const rtl = getComputedStyle(root.host).direction === "rtl";
      // Where a column starts, by reading direction, from the first data row.
      const start = (element) => {
        const rect = element.getBoundingClientRect();
        return Math.round(rtl ? -rect.right : rect.left);
      };
      const starts = [...root.querySelector('td[data-row="0"]').closest("tr").children].map(start);
      return [...root.querySelectorAll("thead th")].map((th) => ({
        said: Number(th.getAttribute("aria-colindex")),
        stands: starts.findIndex((at) => Math.abs(at - start(th)) <= 1) + 1,
      }));
    });
    expect(cells).toHaveLength(9);
    for (const cell of cells) expect(cell.said).toBe(cell.stands);
    expect(cells.map((cell) => cell.said)).toEqual([1, 2, 3, 5, 7, 3, 4, 5, 6]);
  }
});

test("a data cell is announced with its band and its column", async ({ page, browserName }) => {
  // The tree is read over the Chrome DevTools Protocol, which only Chromium has.
  test.skip(browserName !== "chromium", "reads the accessibility tree over CDP");
  // What the cell names as its headers, resolved as a browser resolves
  // `headers`: ids in the cell's own tree, the band first.
  const announced = (selector, name) =>
    inside(
      page,
      selector,
      (root, name) => {
        const col = [...root.querySelectorAll("th[data-col]")].find(
          (th) => th.querySelector("span").textContent === name,
        ).dataset.col;
        return [...root.querySelectorAll(`td[data-col="${col}"][data-row]`)].map((td) =>
          td.headers.split(" ").filter(Boolean).map((id) => {
            const th = root.getElementById(id);
            return `${th.tagName.toLowerCase()}[scope=${th.scope}] ${th.querySelector("span").textContent}`;
          }),
        );
      },
      name,
    );
  for (const selector of ["#banded", "#rtl"]) {
    for (const [name, band] of [
      ["customer", "Customer"],
      ["country", "Customer"],
      ["amount", "Order"],
      ["qty", "Order"],
    ]) {
      const cells = await announced(selector, name);
      expect(cells).toHaveLength(12);
      for (const headers of cells) expect(headers).toEqual([`th[scope=colgroup] ${band}`, `th[scope=col] ${name}`]);
    }
    // A column without a band has the one header above it, found by the table.
    for (const headers of await announced(selector, "id")) expect(headers).toEqual([]);
  }

  // The accessibility tree: the bands are column headers of the first header
  // row, the columns under them of the second, the cells grid cells.
  await expect(page.locator("#banded").locator("thead")).toMatchAriaSnapshot(`
    - rowgroup:
      - row "Select all matching rows id Customer Order ordered_on":
        - columnheader "Select all matching rows":
          - checkbox "Select all matching rows"
        - columnheader "id"
        - columnheader "Customer"
        - columnheader "Order"
        - columnheader "ordered_on"
      - row "customer country amount qty":
        - columnheader "customer"
        - columnheader "country"
        - columnheader "amount"
        - columnheader "qty"
  `);
  // And each cell of the tree knows its place: Chromium works the headers of
  // a cell out from the column it stands in, so a cell under `Customer` has
  // to be in a column the band's cell spans.
  const client = await page.context().newCDPSession(page);
  const { nodes } = await client.send("Accessibility.getFullAXTree");
  const grid = nodes.find((node) => node.role?.value === "grid" && node.name?.value === "Orders");
  expect(grid).toBeTruthy();
  const byId = new Map(nodes.map((node) => [node.nodeId, node]));
  const rows = [];
  const walk = (node) => {
    if (node.role?.value === "row") rows.push(node.childIds.map((id) => byId.get(id)).filter((cell) => !cell.ignored));
    else for (const id of node.childIds ?? []) walk(byId.get(id));
  };
  walk(grid);
  // The names as drawn: the header's capitals are CSS, and Chromium's tree
  // carries them.
  const said = (row) => row.map((cell) => [cell.role.value, cell.name.value.toLowerCase()]);
  expect(said(rows[0])).toEqual([
    ["columnheader", "select all matching rows"],
    ["columnheader", "id"],
    ["columnheader", "customer"],
    ["columnheader", "order"],
    ["columnheader", "ordered_on"],
  ]);
  expect(said(rows[1])).toEqual(["customer", "country", "amount", "qty"].map((name) => ["columnheader", name]));
  expect(rows[2].map((cell) => cell.role.value)).toEqual(Array(7).fill("gridcell"));
});

test("the arrow keys cross the header by what is on the screen", async ({ page }) => {
  const press = async (key, lands) => {
    await page.keyboard.press(key);
    expect(await focused(page, "#banded"), key).toBe(lands);
    // The roving tabindex: one cell of the grid, a band's included.
    expect(await tabStops(page, "#banded")).toBe(1);
  };
  await focusHeader(page, "#banded", "country");
  expect(await focused(page, "#banded")).toBe("header:country");

  // Up to the band and back to the column the reader came from.
  await press("ArrowUp", "band:Customer");
  await press("ArrowUp", "band:Customer");
  await press("ArrowDown", "header:country");
  await press("ArrowUp", "band:Customer");

  // Sideways in the first row: the bands and the headers that span both rows.
  await press("ArrowRight", "band:Order");
  await press("ArrowRight", "header:ordered_on");
  await press("ArrowRight", "header:ordered_on");
  await press("ArrowUp", "header:ordered_on");
  await press("ArrowLeft", "header:qty");
  await press("ArrowUp", "band:Order");
  await press("ArrowLeft", "band:Customer");
  // Reached sideways, a band leads down to its first column.
  await press("ArrowDown", "header:customer");
  await press("ArrowUp", "band:Customer");
  await press("ArrowLeft", "header:id");
  await press("ArrowRight", "header:customer");

  // Sideways in the second row: the columns' headers, one by one.
  await press("ArrowRight", "header:country");
  await press("ArrowRight", "header:amount");
  await press("Home", "select-all");
  await press("End", "header:ordered_on");

  // The ends of the first row, from a band.
  await press("ArrowLeft", "header:qty");
  await press("ArrowUp", "band:Order");
  await press("End", "header:ordered_on");
  await press("ArrowLeft", "header:qty");
  await press("ArrowUp", "band:Order");
  await press("Home", "select-all");

  // Down into the rows and back up: a cell, its column's header, the band.
  await press("ArrowRight", "header:id");
  await press("ArrowRight", "header:customer");
  await press("ArrowRight", "header:country");
  await press("ArrowDown", "cell:0:country");
  await press("ArrowDown", "cell:1:country");
  await press("ArrowUp", "cell:0:country");
  await press("ArrowUp", "header:country");
  await press("ArrowUp", "band:Customer");
  await press("ArrowDown", "header:country");
  await press("ArrowDown", "cell:0:country");
  await press("Control+Home", "select-all");

  // A band sorts nothing, and its keys change no query.
  await focusHeader(page, "#banded", "Order");
  const asked = await queries(page, "banded");
  for (const key of ["Enter", "Space", "Control+Shift+ArrowRight"]) await press(key, "band:Order");
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  expect(await queries(page, "banded")).toBe(asked);
  expect(await inside(page, "#banded", (root) => root.querySelectorAll('th[aria-sort]:not([aria-sort="none"])').length)).toBe(1);
});

test("right to left the arrows change places in the header rows too", async ({ page }) => {
  const press = async (key, lands) => {
    await page.keyboard.press(key);
    expect(await focused(page, "#rtl"), key).toBe(lands);
    expect(await tabStops(page, "#rtl")).toBe(1);
  };
  await focusHeader(page, "#rtl", "country");
  await press("ArrowUp", "band:Customer");
  // `Order` stands on the left of `Customer`.
  await press("ArrowLeft", "band:Order");
  await press("ArrowLeft", "header:ordered_on");
  await press("ArrowRight", "header:qty");
  await press("ArrowUp", "band:Order");
  await press("ArrowRight", "band:Customer");
  await press("ArrowRight", "header:id");
  await press("ArrowDown", "cell:0:id");
});

test("a column moves inside its band and stops at the band's edge", async ({ page }) => {
  const asked = await queries(page, "banded");
  await focusHeader(page, "#banded", "customer");
  await page.keyboard.press("Control+ArrowLeft");
  await expect.poll(() => status(page, "#banded")).toContain("customer is already at the end");
  expect(await columns(page, "#banded")).toEqual(ORDER);
  expect(await queries(page, "banded")).toBe(asked);

  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("customer moved to position 3 of 6");
  await settled(page, "#banded");
  expect(await columns(page, "#banded")).toEqual(["id", "country", "customer", "amount", "qty", "ordered_on"]);
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["country", "customer"]],
    ["Order", ["amount", "qty"]],
  ]);
  await expect.poll(() => focused(page, "#banded")).toBe("header:customer");
  expect((await views(page, "banded")).at(-1).order).toEqual(["id", "country", "customer", "amount", "qty", "ordered_on"]);

  // The band ends here: `amount` is another band's.
  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("customer is already at the end");
  expect(await columns(page, "#banded")).toEqual(["id", "country", "customer", "amount", "qty", "ordered_on"]);

  // A column without a band steps over a band as over one column.
  await focusHeader(page, "#banded", "id");
  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("id moved to position 3 of 6");
  await settled(page, "#banded");
  expect(await columns(page, "#banded")).toEqual(["country", "customer", "id", "amount", "qty", "ordered_on"]);
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["country", "customer"]],
    ["Order", ["amount", "qty"]],
  ]);
});

test("a band moves as a whole, and the focus stays on it", async ({ page }) => {
  const asked = await queries(page, "banded");
  await focusHeader(page, "#banded", "Customer");
  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("Customer moved to position 4 of 6");
  await settled(page, "#banded");
  expect(await columns(page, "#banded")).toEqual(["id", "amount", "qty", "customer", "country", "ordered_on"]);
  expect(await bands(page, "#banded")).toEqual([
    ["Order", ["amount", "qty"]],
    ["Customer", ["customer", "country"]],
  ]);
  await expect.poll(() => focused(page, "#banded")).toBe("band:Customer");
  expect(await tabStops(page, "#banded")).toBe(1);
  // One query, in the new order, and the view says it.
  expect(await queries(page, "banded")).toBe(asked + 1);
  expect((await views(page, "banded")).at(-1).order).toEqual(["id", "amount", "qty", "customer", "country", "ordered_on"]);

  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("Customer moved to position 5 of 6");
  await settled(page, "#banded");
  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("Customer is already at the end");
  expect(await columns(page, "#banded")).toEqual(["id", "amount", "qty", "ordered_on", "customer", "country"]);

  // Right to left the arrows change places: the left one moves towards the end.
  await focusHeader(page, "#rtl", "Customer");
  await page.keyboard.press("Control+ArrowLeft");
  await expect.poll(() => status(page, "#rtl")).toContain("Customer moved to position 4 of 6");
  await settled(page, "#rtl");
  expect(await columns(page, "#rtl")).toEqual(["id", "amount", "qty", "customer", "country", "ordered_on"]);
});

test("a view's order cannot take a band apart", async ({ page }) => {
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.getElementById("banded"), {
      columns: { order: ["country", "id", "qty", "customer", "ordered_on", "amount"] },
    }),
  );
  await expect.poll(() => columns(page, "#banded")).toEqual(["country", "customer", "id", "qty", "amount", "ordered_on"]);
  await settled(page, "#banded");
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["country", "customer"]],
    ["Order", ["qty", "amount"]],
  ]);
});

test("hiding the last shown column of a band hides the band", async ({ page }) => {
  const hide = async (hidden) => {
    await page.evaluate(
      (hidden) => window.__opengridModule.set_view(document.getElementById("banded"), { columns: { hidden } }),
      hidden,
    );
    await expect.poll(() => columns(page, "#banded")).toEqual(ORDER.filter((name) => !hidden.includes(name)));
    await settled(page, "#banded");
  };
  const header = () =>
    inside(page, "#banded", (root) => ({
      rows: root.querySelectorAll("thead tr").length,
      tall: root.querySelectorAll('thead th[rowspan="2"]').length,
      rowcount: root.querySelector("table").getAttribute("aria-rowcount"),
      first: root.querySelector('td[data-row="0"]').closest("tr").getAttribute("aria-rowindex"),
    }));

  // One column of the band left: the band spans it alone.
  await hide(["country"]);
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["customer"]],
    ["Order", ["amount", "qty"]],
  ]);
  // The reader hides the last one from the column list.
  await inside(page, "#banded", (root) => root.querySelector('[part="columns"] input[data-column="customer"]').click());
  await expect.poll(() => columns(page, "#banded")).toEqual(["id", "amount", "qty", "ordered_on"]);
  await settled(page, "#banded");
  expect(await bands(page, "#banded")).toEqual([["Order", ["amount", "qty"]]]);
  expect(await header()).toEqual({ rows: 2, tall: 3, rowcount: "202", first: "3" });

  // No band left: the header is the one row of a grid without bands.
  await hide(["customer", "country", "amount", "qty"]);
  expect(await bands(page, "#banded")).toEqual([]);
  expect(await header()).toEqual({ rows: 1, tall: 0, rowcount: "201", first: "2" });
  expect(await inside(page, "#banded", (root) => root.querySelectorAll("[aria-colindex], td[headers], col").length)).toBe(0);

  // Shown again, the band is back where it stood.
  await hide([]);
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["customer", "country"]],
    ["Order", ["amount", "qty"]],
  ]);
  expect(await header()).toEqual({ rows: 2, tall: 3, rowcount: "202", first: "3" });
});

test("print and exports name the band with its column: Band · Column", async ({ page }) => {
  // The page's words for the banded columns, and none for the others.
  const headers = await page.evaluate(() => window.__opengridModule.get_export_headers(document.getElementById("banded")));
  expect(headers).toEqual({
    customer: "Customer \u00b7 customer",
    country: "Customer \u00b7 country",
    amount: "Order \u00b7 amount",
    qty: "Order \u00b7 qty",
  });

  // A CSV with them: the band in the header row, the data untouched.
  const csv = await page.evaluate(async () => {
    const { exportRows } = await import("/packages/opengrid/loader.js");
    const grid = document.getElementById("banded");
    const query = window.__opengridModule.get_query(grid);
    const blob = await exportRows(window.__provider, query, {
      headers: window.__opengridModule.get_export_headers(grid),
      bom: false,
      chunkSize: 50,
    });
    return await blob.text();
  });
  expect(csv.split("\r\n")[0]).toBe("id,Customer \u00b7 customer,Customer \u00b7 country,Order \u00b7 amount,Order \u00b7 qty,ordered_on");
  expect(csv.split("\r\n").length).toBe(202);

  // Without them the header is the field names, as before.
  const plain = await page.evaluate(async () => {
    const { exportRows } = await import("/packages/opengrid/loader.js");
    const blob = await exportRows(window.__provider, window.__opengridModule.get_query(document.getElementById("banded")), { bom: false });
    return (await blob.text()).split("\r\n")[0];
  });
  expect(plain).toBe("id,customer,country,amount,qty,ordered_on");

  // JSON has no header row, and says so.
  const refused = await page.evaluate(async () => {
    const { exportRows } = await import("/packages/opengrid/loader.js");
    try {
      await exportRows(window.__provider, window.__opengridModule.get_query(document.getElementById("banded")), { format: "json", headers: { id: "x" } });
    } catch (error) {
      return error.message;
    }
    return null;
  });
  expect(refused).toContain("a JSON export has none");

  // The paper has one header row too.
  await page.evaluate(() => window.__opengridModule.prepare_print(document.getElementById("banded")));
  const printed = await page.evaluate(() =>
    [...document.getElementById("banded").shadowRoot.querySelector("[data-print]").shadowRoot.querySelectorAll("thead th")].map((th) => th.textContent),
  );
  expect(printed).toEqual(["id", "Customer \u00b7 customer", "Customer \u00b7 country", "Order \u00b7 amount", "Order \u00b7 qty", "ordered_on"]);
});

test("pinning a column of a band pins the band: one cell above its columns, in place", async ({ page }) => {
  const asked = await queries(page, "banded");
  await focusHeader(page, "#banded", "country");
  await page.keyboard.press("Alt+Shift+ArrowLeft");
  // The band is what is pinned, and what is said.
  await expect.poll(() => status(page, "#banded")).toContain("Customer pinned at the start");
  await settled(page, "#banded");
  expect(await columns(page, "#banded")).toEqual(["customer", "country", "id", "amount", "qty", "ordered_on"]);
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["customer", "country"]],
    ["Order", ["amount", "qty"]],
  ]);
  await expect.poll(() => focused(page, "#banded")).toBe("header:country");
  expect(await queries(page, "banded")).toBe(asked + 1);
  // The view holds the choice for each column of the band.
  expect((await views(page, "banded")).at(-1).pinned).toEqual({ country: "start", customer: "start" });

  // Scrolled sideways the band's cell stays above its two columns, in one
  // piece, right after the selection column — and the rest scrolls under.
  const check = async (selector, left) => {
    await scrollTo(page, selector, left);
    const at = await layout(page, selector);
    expect(at.bands.Customer.sticky && at.columns.customer.sticky && at.columns.country.sticky).toBe(true);
    expect(at.bands.Order.sticky || at.columns.id.sticky).toBe(false);
    const edges = [...at.columns.customer.header, ...at.columns.country.header];
    expect(near(at.bands.Customer.header, [Math.min(...edges), Math.max(...edges)])).toBe(true);
    for (const name of ORDER) {
      expect(near(at.columns[name].header, at.columns[name].cell), name).toBe(true);
      // The fields of the filter row follow: the pinned band's stay above it.
      expect(near(at.columns[name].header, at.columns[name].field), name).toBe(true);
    }
    return at;
  };
  const still = await check("#banded", 0);
  expect(still.bands.Customer.header).toEqual([still.viewport[0] + 44, still.viewport[0] + 44 + 286]);
  const scrolled = await check("#banded", 200);
  expect(scrolled.bands.Customer.header).toEqual(still.bands.Customer.header);
  expect(scrolled.columns.id.header[0]).toBe(still.columns.id.header[0] - 200);

  // Nothing of what scrolls under shows in the pinned band or its columns'
  // headers — not between the two rows either: the same picture wherever the
  // grid is scrolled to, with the `id` header's marks passing under.
  const pinnedHeader = async (left) => {
    await scrollTo(page, "#banded", left);
    const clip = await inside(page, "#banded", (root) => {
      const band = root.querySelector('th[part="band"]').getBoundingClientRect();
      const head = root.querySelector("thead").getBoundingClientRect();
      return { x: band.left, y: head.top, width: band.width, height: head.height };
    });
    return page.screenshot({ clip });
  };
  expect((await pinnedHeader(120)).equals(await pinnedHeader(200))).toBe(true);

  // From the band's own cell: one step back lets the band go.
  await focusHeader(page, "#banded", "Customer");
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => status(page, "#banded")).toContain("Customer no longer pinned");
  await settled(page, "#banded");
  await expect.poll(() => focused(page, "#banded")).toBe("band:Customer");
  expect((await views(page, "banded")).at(-1).pinned).toEqual({});
  expect((await layout(page, "#banded")).bands.Customer.sticky).toBe(false);

  // Right to left: the same band at the start is at the right edge.
  await focusHeader(page, "#rtl", "Customer");
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => status(page, "#rtl")).toContain("Customer pinned at the start");
  await settled(page, "#rtl");
  expect(await columns(page, "#rtl")).toEqual(["customer", "country", "id", "amount", "qty", "ordered_on"]);
  const mirrored = await check("#rtl", -200);
  expect(mirrored.bands.Customer.header).toEqual([mirrored.viewport[1] - 44 - 286, mirrored.viewport[1] - 44]);
});

test("the page pins a band with one of its columns, at the end too", async ({ page }) => {
  for (const selector of ["#banded", "#rtl"]) {
    await setColumns(page, selector, { country: { pin: "end" } });
    await expect.poll(() => columns(page, selector)).toEqual(["id", "amount", "qty", "ordered_on", "customer", "country"]);
    await settled(page, selector);
    expect(await bands(page, selector)).toEqual([
      ["Order", ["amount", "qty"]],
      ["Customer", ["customer", "country"]],
    ]);
    const at = await layout(page, selector);
    expect(at.bands.Customer.sticky && at.columns.customer.sticky && at.columns.country.sticky).toBe(true);
    // At the end edge of the grid, 286 px wide, above its columns.
    expect(at.bands.Customer.header).toEqual(
      selector === "#rtl" ? [at.viewport[0], at.viewport[0] + 286] : [at.viewport[1] - 286, at.viewport[1]],
    );
    const edges = [...at.columns.customer.header, ...at.columns.country.header];
    expect(near(at.bands.Customer.header, [Math.min(...edges), Math.max(...edges)])).toBe(true);
  }
  // The column menu of a column in the band shows where the band is.
  await focusHeader(page, "#banded", "customer");
  await page.keyboard.press("Alt+ArrowDown");
  await expect
    .poll(() =>
      inside(page, "#banded", (root) =>
        [...root.querySelectorAll('[part="column-menu"] [data-action^="pin:"]')]
          .filter((item) => item.getAttribute("aria-checked") === "true")
          .map((item) => item.dataset.action),
      ),
    )
    .toEqual(["pin:end"]);
  await page.keyboard.press("Escape");
});

test("a band the configuration cannot mean is reported, and nothing of the call applied", async ({ page }) => {
  await setColumns(page, "#banded", { customer: { pin: "start" }, country: { pin: "end" }, qty: { band: " " } });
  await expect
    .poll(() => status(page, "#banded"))
    .toContain("country: is pinned at another end than customer, of the same band");
  expect(await status(page, "#banded")).toContain("qty: has an empty band");
  await settled(page, "#banded");
  // As the grid was: both bands, nothing pinned.
  expect(await columns(page, "#banded")).toEqual(ORDER);
  expect(await bands(page, "#banded")).toEqual([
    ["Customer", ["customer", "country"]],
    ["Order", ["amount", "qty"]],
  ]);
  expect((await layout(page, "#banded")).bands.Customer.sticky).toBe(false);
});

test("set_columns takes bands away and puts them elsewhere", async ({ page }) => {
  // No configuration: the columns stand as they stood, and the header is the
  // one row of a grid without bands.
  await page.evaluate(() => window.__opengridModule.set_columns(document.getElementById("banded"), {}));
  await expect.poll(() => bands(page, "#banded")).toEqual([]);
  await settled(page, "#banded");
  expect(await columns(page, "#banded")).toEqual(ORDER);
  expect(await inside(page, "#banded", (root) => root.querySelectorAll("thead tr").length)).toBe(1);

  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.getElementById("banded"), {
      id: { band: "Key" },
      amount: { band: "Key" },
    }),
  );
  // `amount` stands with `id` now: a band is in one piece.
  await expect.poll(() => columns(page, "#banded")).toEqual(["id", "amount", "customer", "country", "qty", "ordered_on"]);
  await settled(page, "#banded");
  expect(await bands(page, "#banded")).toEqual([["Key", ["id", "amount"]]]);

  // Another name for the same columns: the order stays, the header follows.
  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.getElementById("banded"), {
      id: { band: "Identity" },
      amount: { band: "Identity" },
    }),
  );
  await expect.poll(() => bands(page, "#banded")).toEqual([["Identity", ["id", "amount"]]]);
});

test("a cell scrolled to stays clear of both header rows", async ({ page }) => {
  // WCAG 2.4.11: the focus is not under the sticky header, which is two rows
  // tall here. Down past the fold, then back up: each cell the browser
  // scrolls to has to land below the header.
  await focusHeader(page, "#banded", "country");
  for (let step = 0; step < 10; step += 1) await page.keyboard.press("ArrowDown");
  for (let step = 0; step < 6; step += 1) {
    await page.keyboard.press("ArrowUp");
    const at = await inside(page, "#banded", (root) => ({
      top: Math.round(root.activeElement.getBoundingClientRect().top),
      header: Math.round(root.querySelector("thead").getBoundingClientRect().bottom),
      scrolled: root.querySelector('[part~="viewport"]').scrollTop,
    }));
    expect(at.top, `step ${step}`).toBeGreaterThanOrEqual(at.header - 1);
  }
  expect(await focused(page, "#banded")).toBe("cell:3:country");
  expect(await inside(page, "#banded", (root) => root.querySelector('[part~="viewport"]').scrollTop)).toBeGreaterThan(0);
});

test("the page has no axe violations, in both directions, with a band focused and one pinned", async ({ page }) => {
  await focusHeader(page, "#banded", "Order");
  await focusHeader(page, "#rtl", "country");
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => status(page, "#rtl")).toContain("Customer pinned at the start");
  await settled(page, "#rtl");
  await scrollTo(page, "#rtl", -200);
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
