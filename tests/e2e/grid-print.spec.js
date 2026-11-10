import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The print path (issue #334): `prepare_print` turns the view of a virtualized
// grid — 200 rows behind a window of 8 — into a plain table of every match,
// which only `@media print` shows, in place of the grid. The browser makes the
// PDF; the screen is what it was before and after.

const GRID = "/tests/e2e/fixtures/grid-print.html";

async function open(page, url = GRID) {
  await page.goto(url);
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot?.querySelector("td[data-row]"),
  );
}

/** `prepare_print` of the page's grid: `{ rows }`, or `{ error, code }`. */
const prepare = (page) =>
  page.evaluate(() =>
    window.__opengridModule.prepare_print(document.querySelector("opengrid-grid")).then(
      (rows) => ({ rows }),
      (error) => ({ error: error.message, code: error.code ?? null }),
    ),
  );

/** The print table, or null: its caption and its rows, each cell as `tag[scope]:text`. */
const printed = (page) =>
  page.evaluate(() => {
    const holder = document.querySelector("opengrid-grid").shadowRoot.querySelector("[data-print]");
    if (!holder) return null;
    const table = holder.shadowRoot.querySelector("table");
    const rows = [...table.querySelectorAll("tr")].map((tr) =>
      [...tr.children].map((cell) => {
        const scope = cell.getAttribute("scope");
        return `${cell.localName}${scope ? `[${scope}]` : ""}:${cell.textContent}`;
      }),
    );
    return {
      caption: table.querySelector("caption").textContent,
      head: [...table.querySelectorAll("thead tr")].length,
      rows,
      roles: table.querySelectorAll("[role], [tabindex], button, input").length,
    };
  });

/** What the screen holds of the grid: its window, its focus, its scroll offset. */
const screen = (page) =>
  page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const active = root.activeElement;
    return {
      windowRows: root.querySelectorAll("tbody tr[aria-rowindex]").length,
      focus: active && `${active.localName}:${active.dataset.row}:${active.dataset.col}`,
      scrollTop: root.querySelector('[part="viewport"]').scrollTop,
      sheets: root.adoptedStyleSheets.length,
      table: !!root.querySelector("[data-print]"),
    };
  });

/** Boxes and colours, as the current medium lays them out. */
const layout = (page) =>
  page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    const root = host.shadowRoot;
    const holder = root.querySelector("[data-print]");
    const table = holder?.shadowRoot.querySelector("table");
    const box = (node) => node.getBoundingClientRect();
    const cell = table?.querySelector("td");
    return {
      host: box(host).height,
      grid: box(root.querySelector('[part="layout"]')).height,
      table: table ? box(table).height : 0,
      tableBottom: table ? box(table).bottom : 0,
      after: box(document.querySelector("#after")).top,
      ink: cell && getComputedStyle(cell).color,
      ground: table && getComputedStyle(table).backgroundColor,
      liveInk: getComputedStyle(root.querySelector("td[data-row]")).color,
      hostGround: getComputedStyle(host).backgroundColor,
    };
  });

test.describe("the print table", () => {
  test.beforeEach(async ({ page }) => open(page));

  test("holds every match, not the window", async ({ page }) => {
    expect(await prepare(page)).toEqual({ rows: 200 });
    const table = await printed(page);
    // One header row, repeated by the browser on every page, and a row per match.
    expect(table.head).toBe(1);
    expect(table.rows).toHaveLength(201);
    // The shown columns under their titles — no selection column, no action column.
    expect(table.rows[0]).toEqual([
      "th[col]:id",
      "th[col]:customer",
      "th[col]:country",
      "th[col]:Amount (EUR)",
      "th[col]:qty",
    ]);
    // The values as the grid shows them: through the page's format.
    expect(table.rows[1]).toEqual(["td:1", "td:Epsilon", "td:GB", "td:512761511.96", "td:183 pcs"]);
    expect(table.rows[200][0]).toBe("td:200");
    expect(table.caption).toBe("Orders · 200 matches");
    // A table for paper: nothing to operate, no grid roles.
    expect(table.roles).toBe(0);
    // The grid itself still holds its window.
    expect((await screen(page)).windowRows).toBe(8);
  });

  test("is asked for in one request, as the view's query with a window of 10 000", async ({ page }) => {
    await page.evaluate(() => (window.__queries.length = 0));
    await prepare(page);
    const queries = await page.evaluate(() => window.__queries.map((query) => JSON.parse(query)));
    expect(queries).toHaveLength(1);
    const view = await page.evaluate(() =>
      window.__opengridModule.get_query(document.querySelector("opengrid-grid")),
    );
    expect(queries[0]).toEqual({ ...view, offset: 0, limit: 10000 });
  });

  test("shows on paper only, in place of the grid", async ({ page }) => {
    await prepare(page);
    const onScreen = await layout(page);
    expect(onScreen.table).toBe(0);
    expect(onScreen.host).toBe(320);
    expect(onScreen.grid).toBe(320);

    await page.emulateMedia({ media: "print" });
    const onPaper = await layout(page);
    // The chrome and the scroller take no room; the table takes all of its own,
    // and the host grows with it, so what follows the grid follows the table.
    expect(onPaper.grid).toBe(0);
    expect(onPaper.table).toBeGreaterThan(200 * 15);
    expect(onPaper.host).toBeGreaterThanOrEqual(onPaper.table);
    expect(onPaper.after).toBeGreaterThanOrEqual(onPaper.tableBottom);
  });

  test("is black on white in the dark look", async ({ page }) => {
    // The fixture's grid is `theme="dark"`: light ink on the screen.
    const onScreen = await layout(page);
    expect(onScreen.liveInk).not.toBe("rgb(0, 0, 0)");
    await prepare(page);
    await page.emulateMedia({ media: "print" });
    const onPaper = await layout(page);
    expect(onPaper.ink).toBe("rgb(0, 0, 0)");
    expect(onPaper.ground).toBe("rgb(255, 255, 255)");
    expect(onPaper.hostGround).toBe("rgb(255, 255, 255)");
  });

  test("passes axe on paper", async ({ page }) => {
    await prepare(page);
    await page.emulateMedia({ media: "print" });
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });

  test("reaches the PDF whole, as a table", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "page.pdf() is Chromium's");
    await prepare(page);
    const pdf = (await page.pdf({ format: "A4", tagged: true })).toString("latin1");
    const count = (pattern) => (pdf.match(pattern) ?? []).length;
    // 200 rows do not fit one sheet of A4 …
    expect(count(/\/Type\s*\/Page\b/g)).toBeGreaterThan(1);
    // … and the PDF's structure has the table: its caption, the header cells
    // and a row per match under the header row.
    expect(count(/\/S\s*\/Table\b/g)).toBe(1);
    expect(count(/\/S\s*\/Caption\b/g)).toBe(1);
    expect(count(/\/S\s*\/TH\b/g)).toBe(5);
    expect(count(/\/S\s*\/TR\b/g)).toBe(201);
  });

  test("leaves the screen as it was, and is gone after the print", async ({ page, browserName }) => {
    await page.evaluate(async () => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const viewport = root.querySelector('[part="viewport"]');
      viewport.scrollTop = 1000;
      await new Promise((done) => viewport.addEventListener("scroll", done, { once: true }));
    });
    await page.waitForFunction(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      return root.querySelector("td[data-row]")?.dataset.row !== "0";
    });
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-col="1"]').focus(),
    );
    const before = await screen(page);
    expect(before.focus).toMatch(/^td:\d+:1$/);
    expect(before.scrollTop).toBeGreaterThan(0);

    await prepare(page);
    expect(await screen(page)).toEqual({ ...before, sheets: before.sheets + 1, table: true });

    // The print itself: a PDF where the browser writes one, and `afterprint`
    // either way — the event every print ends with.
    await page.emulateMedia({ media: "print" });
    expect(await screen(page)).toEqual({ ...before, sheets: before.sheets + 1, table: true });
    await page.emulateMedia({ media: null });
    if (browserName === "chromium") {
      await page.pdf({ format: "A4" });
    } else {
      await page.evaluate(() => window.dispatchEvent(new Event("afterprint")));
    }
    expect(await screen(page)).toEqual(before);

    // The grid is alive: the focus moves with the keys.
    const row = Number(before.focus.split(":")[1]);
    await page.keyboard.press("ArrowDown");
    expect((await screen(page)).focus).toBe(`td:${row + 1}:1`);
    // And the next print without preparing is the grid again, not an old table.
    await page.evaluate(() => window.dispatchEvent(new Event("afterprint")));
    expect(await printed(page)).toBeNull();
  });

  test("is the view: its filter, its sort, its columns", async ({ page }) => {
    await page.evaluate(() => {
      const grid = document.querySelector("opengrid-grid");
      const module = window.__opengridModule;
      module.set_view(grid, {
        ...module.get_view(grid),
        columns: {
          order: ["customer", "id", "qty", "country", "amount"],
          hidden: ["amount"],
          widths: {},
        },
        filters: [{ column: "country", op: "eq", value: "DE" }],
        sort: [{ field: "id", direction: "desc" }],
      });
    });
    await expect
      .poll(() =>
        page.evaluate(
          () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent,
        ),
      )
      .toContain("52 matches");
    expect(await prepare(page)).toEqual({ rows: 52 });
    const table = await printed(page);
    expect(table.caption).toBe("Orders · 52 matches · country is DE");
    // The hidden column is not on the paper; the moved ones are where they were moved to.
    expect(table.rows[0]).toEqual(["th[col]:customer", "th[col]:id", "th[col]:qty", "th[col]:country"]);
    const ids = table.rows.slice(1).map((row) => Number(row[1].slice(3)));
    expect(ids).toHaveLength(52);
    expect(ids).toEqual([...ids].sort((a, b) => b - a));
  });

  test("holds the page's filter, as every query of the grid does", async ({ page }) => {
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .setAttribute("filter", JSON.stringify({ field: "country", op: "eq", value: "DE" })),
    );
    await expect
      .poll(() =>
        page.evaluate(
          () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent,
        ),
      )
      .toContain("52 matches");
    expect(await prepare(page)).toEqual({ rows: 52 });
    const table = await printed(page);
    expect(table.rows).toHaveLength(53);
    expect(new Set(table.rows.slice(1).map((row) => row[2]))).toEqual(new Set(["td:DE"]));
    // It is the page's, not the reader's: no chip names it, so the caption does not either.
    expect(table.caption).toBe("Orders \u00b7 52 matches");
  });

  test("prints a grouped grid with every group open under its header", async ({ page }) => {
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").setAttribute("group-by", "country,customer"),
    );
    await page.waitForFunction(
      () => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="group"]'),
    );
    expect(await prepare(page)).toEqual({ rows: 200 });
    const table = await printed(page);
    expect(table.caption).toBe("Orders · 200 matches · Grouped by country › customer");
    expect(table.rows.slice(1, 4)).toEqual([
      ["th[rowgroup]:country: DE (52 rows)"],
      ["th[rowgroup]:customer: Alpha (6 rows)"],
      ["td:9", "td:Alpha", "td:DE", "td:546403668.79", "td:673 pcs"],
    ]);
    // Every header's count is the number of rows under it, and the outer
    // groups' counts add up to every match.
    const headers = table.rows.map((row, at) => ({ row, at })).filter(({ row }) => row[0].startsWith("th[rowgroup]"));
    const size = ({ row }) => Number(/\((\d+) rows?\)$/.exec(row[0])[1]);
    const outer = headers.filter(({ row }) => row[0].includes(":country:"));
    expect(outer.map(size).reduce((sum, n) => sum + n, 0)).toBe(200);
    for (const [index, header] of headers.entries()) {
      if (!header.row[0].includes(":customer:")) continue;
      const next = headers[index + 1]?.at ?? table.rows.length;
      expect(next - header.at - 1, header.row[0]).toBe(size(header));
    }
    // NULL is a group with a name, as on the screen.
    expect(outer.at(-1).row[0]).toBe("th[rowgroup]:country: (no value) (6 rows)");

    // A third level opens under the second, a step further in.
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").setAttribute("group-by", "country,customer,qty"),
    );
    await expect
      .poll(() =>
        page.evaluate(
          () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="chips"]').textContent,
        ),
      )
      .toContain("qty");
    expect(await prepare(page)).toEqual({ rows: 200 });
    const levels = await page.evaluate(() => {
      const holder = document.querySelector("opengrid-grid").shadowRoot.querySelector("[data-print]");
      return [...holder.shadowRoot.querySelectorAll("th[scope=rowgroup]")]
        .slice(0, 3)
        .map((th) => `${th.dataset.level}:${th.textContent}`);
    });
    expect(levels).toEqual(["1:country: DE (52 rows)", "2:customer: Alpha (6 rows)", "3:qty: 143 pcs (1 row)"]);
  });

  test("refuses more rows than a print table holds, and builds nothing", async ({ page }) => {
    await page.evaluate(() => {
      window.__answer = (answer) => JSON.stringify({ ...JSON.parse(answer), total_count: 10001 });
    });
    const outcome = await prepare(page);
    expect(outcome.code).toBe("too_many_rows");
    expect(outcome.error).toContain("10001");
    expect(await printed(page)).toBeNull();
    // The look, the widths and the pins: no print sheet was added.
    expect((await screen(page)).sheets).toBe(3);
  });

  test("rejects with the provider's error, and takes an earlier table away", async ({ page }) => {
    await prepare(page);
    expect(await printed(page)).not.toBeNull();
    await page.evaluate(() => {
      window.__answer = () => Promise.reject(new Error("the source is gone"));
    });
    const outcome = await prepare(page);
    expect(outcome.error).toContain("the source is gone");
    expect(await printed(page)).toBeNull();
  });

  test("answers null where there is no query to print", async ({ page }) => {
    const outcome = await page.evaluate(async () => {
      const module = window.__opengridModule;
      const grid = document.createElement("opengrid-grid");
      document.body.append(grid);
      const table = document.createElement("opengrid-table");
      document.body.append(table);
      return [await module.prepare_print(grid), await module.prepare_print(table)];
    });
    expect(outcome).toEqual([null, null]);
  });

  test("without preparing, the grid prints as it stands — in black on white", async ({ page }) => {
    await page.emulateMedia({ media: "print" });
    const onPaper = await layout(page);
    expect(onPaper.host).toBe(320);
    expect(onPaper.grid).toBe(320);
    expect(onPaper.liveInk).toBe("rgb(0, 0, 0)");
    expect(onPaper.hostGround).toBe("rgb(255, 255, 255)");
  });
});

test("a tree is printed whole and flat, indented, its context rows named", async ({ page }) => {
  await open(page, "/tests/e2e/fixtures/grid-tree.html");
  expect(await prepare(page)).toEqual({ rows: 10 });
  let table = await printed(page);
  expect(table.caption).toBe("Organisation · 10 matches");
  // Every node, not only the open ones; an em space per level under the root.
  expect(table.rows.map((row) => row[0])).toEqual([
    "th[col]:name",
    "td:Orphan",
    "td:Partners",
    "td: Dave",
    "td:Sales",
    "td: North",
    "td:  Alice",
    "td:   Eve",
    "td:  Bob",
    "td: South",
    "td:  Carol",
  ]);

  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    const module = window.__opengridModule;
    module.set_view(grid, { ...module.get_view(grid), filters: [{ column: "name", op: "eq", value: "Eve" }] });
  });
  await expect
    .poll(() =>
      page.evaluate(
        () => document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tr[data-context]").length,
      ),
    )
    .toBeGreaterThan(0);
  expect(await prepare(page)).toEqual({ rows: 4 });
  table = await printed(page);
  expect(table.caption).toBe("Organisation · 1 match · name is Eve");
  expect(table.rows.slice(1).map((row) => row[0])).toEqual([
    "td:Sales (context)",
    "td: North (context)",
    "td:  Alice (context)",
    "td:   Eve",
  ]);
});

// The table and the pivot hold all their rows and are tables already: they
// print as they stand. What paper needs of them is the ink, and no toolbar.
for (const [name, url, selector] of [
  ["table", "/tests/e2e/fixtures/table-data.html", "opengrid-table"],
  ["pivot", "/tests/e2e/fixtures/pivot-rtl.html", "opengrid-pivot"],
]) {
  test(`a dark ${name} prints black on white, whole`, async ({ page }) => {
    await page.goto(url);
    await page.waitForFunction(
      (selector) => !!document.querySelector(selector).shadowRoot?.querySelector("tbody td, tbody th"),
      selector,
    );
    const facts = () =>
      page.evaluate((selector) => {
        const host = document.querySelector(selector);
        const root = host.shadowRoot;
        const toolbar = root.querySelector('[part~="toolbar"]');
        return {
          ink: getComputedStyle(root.querySelector("tbody td")).color,
          accent: getComputedStyle(host).getPropertyValue("--og-accent").trim(),
          rows: root.querySelectorAll("tbody tr").length,
          toolbar: toolbar ? toolbar.getBoundingClientRect().height > 0 : null,
        };
      }, selector);
    await page.evaluate((selector) => document.querySelector(selector).setAttribute("theme", "dark"), selector);
    const onScreen = await facts();
    expect(onScreen.ink).not.toBe("rgb(0, 0, 0)");

    await page.emulateMedia({ media: "print" });
    const onPaper = await facts();
    expect(onPaper.ink).toBe("rgb(0, 0, 0)");
    expect(onPaper.accent).toBe("#000");
    expect(onPaper.rows).toBe(onScreen.rows);
    // The pivot's field toolbar arranges the table; it is not on the paper.
    if (name === "pivot") {
      expect(onScreen.toolbar).toBe(true);
      expect(onPaper.toolbar).toBe(false);
    }
  });
}
