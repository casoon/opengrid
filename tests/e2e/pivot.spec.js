import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-pivot>` against a real `opengrid-server` (plan points 32 and 53).
//
// Table Mode: a native <table>, a two-level column header, a row header per row
// and subtotals that say so. The structure is asserted, not just the numbers —
// a pivot whose headers do not connect to its cells is unreadable to a screen
// reader even when every value is right.

/** The pivot's shadow root, as facts. */
async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-pivot").shadowRoot;
    const headerRows = [...root.querySelectorAll("thead tr")];
    return {
      caption: root.querySelector("caption")?.textContent ?? null,
      ariaLabel: root.querySelector("table")?.getAttribute("aria-label") ?? null,
      headerRows: headerRows.length,
      colgroups: headerRows[0]
        ? [...headerRows[0].querySelectorAll('th[scope="colgroup"]')].map((th) => ({
            text: th.textContent,
            colspan: th.getAttribute("colspan"),
          }))
        : [],
      measureHeaders: headerRows[1]
        ? [...headerRows[1].querySelectorAll('th[scope="col"]')].map((th) => th.textContent)
        : [],
      rowHeaders: [...root.querySelectorAll('tbody th[scope="row"]')].map((th) => th.textContent),
      levels: [...root.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-level")),
      totals: [...root.querySelectorAll("tbody tr[data-total]")].length,
      status: root.querySelector('[part="status"]')?.textContent ?? null,
      state: root.querySelector('[part="status"]')?.getAttribute("data-state") ?? null,
    };
  });
}

async function open(page) {
  await page.goto("/tests/e2e/fixtures/pivot.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-pivot tbody tr").first()).toBeVisible();
}

test.describe("pivot", () => {
  test("a late answer to an earlier request is not drawn over the newest", async ({ page }) => {
    await open(page);
    // Two requests in a row, and the server's answer to the first comes last —
    // as it can when the first pivot is the more expensive one.
    const settled = await page.evaluate(async () => {
      const { createPivotProvider } = await import("/packages/opengrid/loader.js");
      const real = createPivotProvider({
        url: "http://127.0.0.1:8082",
        source: "orders",
        token: "e2e-token",
      });
      const pivot = document.querySelector("opengrid-pivot");
      let calls = 0;
      const settled = [];
      window.__opengridModule.set_provider(pivot, {
        async execute(json, mode) {
          const call = ++calls;
          const answer = await real.execute(json, mode);
          if (call === 1) await new Promise((resolve) => setTimeout(resolve, 500));
          settled.push(call);
          return answer;
        },
      });
      pivot.setAttribute("rows", "customer");
      await new Promise((resolve) => setTimeout(resolve, 1500));
      return settled;
    });
    expect(settled).toEqual([2, 1]);
    const shown = await page.evaluate(() =>
      [...document.querySelector("opengrid-pivot").shadowRoot.querySelectorAll("thead th")].map(
        // The name, without the direction mark beside it.
        (th) => th.querySelector("button > span:first-child")?.textContent ?? th.textContent,
      ),
    );
    expect(shown).toContain("customer");
    expect(shown).not.toContain("country");
  });

  test("renders a native table with a two-level column header", async ({ page }) => {
    await open(page);
    const seen = await facts(page);

    expect(seen.caption).toBe("Orders by country and year");
    expect(seen.ariaLabel).toBe("Orders by country and year");
    expect(seen.headerRows).toBe(2);
    // One group per column value that occurs, each spanning its two measures.
    // The NULL year is a group like any other and it has a **name**: an empty
    // header cell is silence to a screen reader.
    expect(seen.colgroups.map((group) => group.text)).toEqual(["2025", "2026", "(no value)"]);
    expect(seen.colgroups.every((group) => group.colspan === "2")).toBe(true);
    expect(seen.measureHeaders).toEqual(["total", "n", "total", "n", "total", "n"]);
    expect(seen.state).toBe("ready");
  });

  test("every row has a header and the grand total says that it is one", async ({ page }) => {
    await open(page);
    const seen = await facts(page);

    // One header per row, and the last row is the grand total.
    expect(seen.rowHeaders.length).toBe(seen.levels.length);
    expect(seen.levels.at(-1)).toBe("0");
    expect(seen.rowHeaders.at(-1)).toBe("Total");
    expect(seen.totals).toBe(1);
    // A real NULL country group is a row of its own, *not* the total — that is
    // the whole reason levels exist (S10/P2) — and it is named, not blank.
    expect(seen.levels.filter((level) => level === "1").length).toBeGreaterThan(1);
    expect(seen.rowHeaders).toContain("(no value)");
    expect(seen.rowHeaders.every((text) => text.trim().length > 0)).toBe(true);
  });

  test("the numbers are the server's, and they add up", async ({ page }) => {
    await open(page);
    const rows = await page.evaluate(() => {
      const root = document.querySelector("opengrid-pivot").shadowRoot;
      return [...root.querySelectorAll("tbody tr")].map((tr) =>
        [...tr.querySelectorAll("td")].map((td) => td.textContent),
      );
    });

    // Column 1 is `n` for 2025, column 3 `n` for 2026, column 5 `n` for NULL.
    const counts = (row) => [1, 3, 5].map((index) => Number(row[index] || 0));
    const total = counts(rows.at(-1));
    const summed = rows
      .slice(0, -1)
      .reduce((acc, row) => counts(row).map((value, i) => value + acc[i]), [0, 0, 0]);
    expect(summed).toEqual(total);
  });

  test("a broken measure list is reported, not swallowed", async ({ page }) => {
    await open(page);
    await page.evaluate(() =>
      document.querySelector("opengrid-pivot").setAttribute("values", "sum(qty)"),
    );
    await expect
      .poll(async () => (await facts(page)).state)
      .toBe("error");
    const seen = await facts(page);
    expect(seen.status).toContain("values");
  });

  test("has no axe violations", async ({ page }) => {
    await open(page);
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });

  // Issue #104: the page's titles and formats, as at the grid and the table.
  test("titles and formats are the page's, and the export stays raw", async ({ page }) => {
    await open(page);
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      window.__opengridModule.set_formats(pivot, {
        ordered_year: (text) => `FY ${text}`,
        n: { kind: "number", locale: "de-DE", minimumFractionDigits: 1 },
      });
    });
    await expect
      .poll(async () => (await facts(page)).colgroups.map((group) => group.text))
      .toEqual(["FY 2025", "FY 2026", "(no value)"]);
    // Once the formats show, so that its own rerun is what shows the titles.
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      window.__opengridModule.set_columns(pivot, {
        country: { title: "Country of order" },
        total: { title: "Quantity" },
        n: { title: "Orders" },
        // Not shown now — the dimensions change with the attributes.
        customer: { title: "Customer" },
      });
    });
    await expect
      .poll(async () => (await facts(page)).measureHeaders)
      .toEqual(["Quantity", "Orders", "Quantity", "Orders", "Quantity", "Orders"]);
    const seen = await facts(page);
    expect(seen.state).toBe("ready");
    // NULL keeps its word: a format never blanks a header.
    expect(seen.colgroups.map((group) => group.text)).toEqual(["FY 2025", "FY 2026", "(no value)"]);
    const shown = await page.evaluate(() => {
      const root = document.querySelector("opengrid-pivot").shadowRoot;
      return {
        dimension: root.querySelector('thead th[rowspan="2"] button > span').textContent,
        total: [...root.querySelectorAll("tbody tr:last-child td")].map((td) => td.textContent),
      };
    });
    expect(shown.dimension).toBe("Country of order");
    // `n` is formatted, `total` is not.
    expect(shown.total[1]).toMatch(/^\d+,0$/);
    expect(shown.total[0]).toMatch(/^\d+$/);

    const csv = await page.evaluate(() =>
      window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot")),
    );
    const header = csv.split(/\r?\n/)[0];
    expect(header).toContain("country");
    expect(header).toContain("2025");
    expect(header).not.toContain("Quantity");
    expect(csv).not.toContain("FY ");

    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
  });

  test("an option a pivot cannot honour is reported", async ({ page }) => {
    await open(page);
    await page.evaluate(() =>
      window.__opengridModule.set_columns(document.querySelector("opengrid-pivot"), {
        country: { title: "Country", width: 120 },
      }),
    );
    await expect
      .poll(async () => (await facts(page)).state)
      .toBe("error");
    expect((await facts(page)).status).toContain("width");
  });
});

// Issue #106: the pivot's view is its rows, columns and values — read and set
// as one value, reported as `opengrid-view-change`, and every answer measured
// as `opengrid-query`, as at the grid.
test.describe("pivot view", () => {
  const FIRST = {
    rows: ["country"],
    columns: ["ordered_year"],
    values: [
      { field: "qty", fn: "sum", as: "total" },
      { fn: "count", as: "n" },
    ],
    sort: [],
    collapsed: [],
  };

  /** Counts the events on the pivot from here on. */
  async function listen(page) {
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      window.__views = [];
      window.__queries = [];
      pivot.addEventListener("opengrid-view-change", (event) => window.__views.push(event.detail.view));
      pivot.addEventListener("opengrid-query", (event) => window.__queries.push(event.detail));
    });
  }

  const views = (page) => page.evaluate(() => window.__views);
  const queries = (page) => page.evaluate(() => window.__queries);
  const getView = (page) =>
    page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-pivot")));
  const setView = (page, view) =>
    page.evaluate(
      (view) => window.__opengridModule.set_view(document.querySelector("opengrid-pivot"), view),
      view,
    );

  test("the view is the attributes", async ({ page }) => {
    await open(page);
    expect(await getView(page)).toEqual(FIRST);
    // A pivot not in the document has no view.
    expect(
      await page.evaluate(() =>
        window.__opengridModule.get_view(document.createElement("opengrid-pivot")),
      ),
    ).toBeNull();
  });

  test("set_view writes all three at once, in one query, and says so once", async ({ page }) => {
    await open(page);
    await listen(page);
    const next = {
      rows: ["customer"],
      columns: [],
      values: [{ fn: "count", as: "n" }],
      sort: [],
      collapsed: [],
    };
    await setView(page, next);

    // The event is the report of the change, there as set_view returns.
    expect(await views(page)).toEqual([next]);
    expect(await getView(page)).toEqual(next);
    expect(
      await page.evaluate(() => {
        const pivot = document.querySelector("opengrid-pivot");
        return ["rows", "columns", "values"].map((name) => pivot.getAttribute(name));
      }),
    ).toEqual(["customer", "", '[{"fn":"count","as":"n"}]']);
    await expect.poll(async () => (await facts(page)).state).toBe("ready");
    const header = await page.evaluate(
      () =>
        document.querySelector("opengrid-pivot").shadowRoot.querySelector("thead th button > span")
          .textContent,
    );
    expect(header).toBe("customer");
    // One query, not one per attribute.
    expect((await queries(page)).length).toBe(1);

    // Setting what it has costs nothing and says nothing.
    await setView(page, next);
    await page.waitForTimeout(200);
    expect((await views(page)).length).toBe(1);
    expect((await queries(page)).length).toBe(1);
  });

  test("a view that does not hold is refused whole", async ({ page }) => {
    await open(page);
    await listen(page);
    await setView(page, { rows: "customer", columns: ["ordered_year"], values: [{ fn: "count" }] });
    const seen = await facts(page);
    expect(seen.status).toContain("rows");
    expect(seen.status).toContain("values");
    // Nothing of it applied: the pivot shown stays, and nothing changed.
    expect(await getView(page)).toEqual(FIRST);
    expect(seen.measureHeaders.length).toBe(6);
    expect(await views(page)).toEqual([]);
  });

  test("an attribute the page sets is a view change", async ({ page }) => {
    await open(page);
    await listen(page);
    await page.evaluate(() =>
      document.querySelector("opengrid-pivot").setAttribute("columns", ""),
    );
    expect(await views(page)).toEqual([{ ...FIRST, columns: [] }]);
    // The same value again is no change.
    await page.evaluate(() =>
      document.querySelector("opengrid-pivot").setAttribute("columns", ""),
    );
    expect((await views(page)).length).toBe(1);
  });

  // The adapters hand the view to `connect`, which is the same for every
  // element: a controlled view is written, the reader's change reported, and
  // writing back what was reported asks nothing.
  test("connect controls a pivot's view like a grid's", async ({ page }) => {
    await open(page);
    await listen(page);
    await page.evaluate(async () => {
      const { connect } = await import("/packages/opengrid/loader.js");
      const pivot = document.querySelector("opengrid-pivot");
      window.__reported = [];
      window.__connection = connect(pivot, {
        view: { rows: ["customer"], columns: [], values: [{ fn: "count", as: "n" }] },
        onViewChange: (view) => {
          window.__reported.push(view);
          // Written back, as a framework's controlled prop is.
          window.__connection.update({ view });
        },
      });
      await window.__connection.ready;
    });
    expect((await getView(page)).rows).toEqual(["customer"]);
    // connect's own write is not the reader's change.
    expect(await page.evaluate(() => window.__reported)).toEqual([]);
    await expect.poll(async () => (await queries(page)).length).toBe(1);

    await page.evaluate(() =>
      document.querySelector("opengrid-pivot").setAttribute("rows", "country"),
    );
    expect(await page.evaluate(() => window.__reported.map((view) => view.rows))).toEqual([
      ["country"],
    ]);
    await expect.poll(async () => (await queries(page)).length).toBe(2);
    await page.waitForTimeout(200);
    expect((await queries(page)).length).toBe(2);
  });

  test("every answer is measured as opengrid-query", async ({ page }) => {
    await open(page);
    await listen(page);
    await page.evaluate(() =>
      document.querySelector("opengrid-pivot").setAttribute("rows", "customer"),
    );
    await expect.poll(async () => (await queries(page)).length).toBe(1);
    const [query] = await queries(page);
    const rows = await page.evaluate(
      () => document.querySelector("opengrid-pivot").shadowRoot.querySelectorAll("tbody tr").length,
    );
    expect(query).toMatchObject({ rows, total: rows });
    expect(["json", "binary"]).toContain(query.form);
    expect(query.bytes).toBeGreaterThan(0);
    expect(query.ms).toBeGreaterThanOrEqual(0);
    expect(query.memory).toBeGreaterThan(0);
  });
});

// Issue #108: sorting — each level among its siblings (rule P9), from the
// headers, with the keyboard, and the focus kept on the pressed button.
test.describe("pivot sorting", () => {
  const shadowOf = (page, fn) =>
    page.evaluate(
      (fn) => new Function("root", `return (${fn})(root)`)(document.querySelector("opengrid-pivot").shadowRoot),
      fn.toString(),
    );
  const countries = (page) =>
    shadowOf(page, (root) =>
      [...root.querySelectorAll('tbody tr[data-level="1"] th')].map((th) => th.textContent),
    );
  const ariaSort = (page) =>
    shadowOf(page, (root) =>
      [...root.querySelectorAll("thead th[aria-sort]")].map((th) => [
        th.querySelector("button")?.textContent.replace(/[\u00a0▲▼]/g, ""),
        th.getAttribute("aria-sort"),
      ]),
    );
  const focused = (page) =>
    shadowOf(page, (root) => root.activeElement?.dataset.sortField ?? root.activeElement?.dataset.sortBy ?? null);

  test("a row-dimension header orders its level, from the keyboard", async ({ page }) => {
    await open(page);
    // Ascending by value is the default, and the header says so. With a
    // column dimension a measure header is no sort button.
    expect(await ariaSort(page)).toEqual([["country", "ascending"]]);
    expect(await countries(page)).toEqual(["(empty)", "DE", "FR", "GB", "US", "(no value)"]);

    await shadowOf(page, (root) => root.querySelector('button[data-sort-field="country"]').focus());
    await page.keyboard.press("Enter");
    // NULL stays last either way (S3).
    await expect.poll(() => countries(page)).toEqual(["US", "GB", "FR", "DE", "(empty)", "(no value)"]);
    expect(await ariaSort(page)).toEqual([["country", "descending"]]);
    // The answer replaced every node; the reader is still on the button.
    expect(await focused(page)).toBe("country");
    expect((await facts(page)).rowHeaders.at(-1)).toBe("Total");
    expect(
      await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-pivot")).sort),
    ).toEqual([{ field: "country", direction: "desc" }]);

    await page.keyboard.press("Space");
    await expect.poll(() => countries(page)).toEqual(["(empty)", "DE", "FR", "GB", "US", "(no value)"]);
    // Ascending is the default: the attribute goes.
    expect(await page.evaluate(() => document.querySelector("opengrid-pivot").hasAttribute("sort"))).toBe(false);
  });

  test("without a column dimension a measure header orders every level", async ({ page }) => {
    await open(page);
    await page.evaluate(() => document.querySelector("opengrid-pivot").setAttribute("columns", ""));
    await expect.poll(() => ariaSort(page)).toEqual([
      ["country", "ascending"],
      ["total", "none"],
      ["n", "none"],
    ]);
    await shadowOf(page, (root) => root.querySelector('button[data-sort-by="n"]').click());
    await expect.poll(() => ariaSort(page)).toEqual([
      ["country", "none"],
      ["total", "none"],
      ["n", "ascending"],
    ]);
    const counts = await shadowOf(page, (root) =>
      [...root.querySelectorAll('tbody tr[data-level="1"]')].map((tr) => Number(tr.querySelectorAll("td")[1].textContent)),
    );
    expect(counts).toEqual([...counts].sort((a, b) => a - b));
    expect(await focused(page)).toBe("n");
    expect((await facts(page)).rowHeaders.at(-1)).toBe("Total");

    await shadowOf(page, (root) => root.querySelector('button[data-sort-by="n"]').click());
    await expect.poll(() => ariaSort(page)).toContainEqual(["n", "descending"]);
    const descending = await shadowOf(page, (root) =>
      [...root.querySelectorAll('tbody tr[data-level="1"]')].map((tr) => Number(tr.querySelectorAll("td")[1].textContent)),
    );
    expect(descending).toEqual([...descending].sort((a, b) => b - a));
  });

  test("a sort that is not the wire's JSON is said", async ({ page }) => {
    await open(page);
    await page.evaluate(() => document.querySelector("opengrid-pivot").setAttribute("sort", "country desc"));
    await expect.poll(async () => (await facts(page)).state).toBe("error");
    expect((await facts(page)).status).toContain("sort");
  });

  test("a sorted pivot has no axe violations", async ({ page }) => {
    await open(page);
    await shadowOf(page, (root) => root.querySelector('button[data-sort-field="country"]').click());
    await expect.poll(() => ariaSort(page)).toEqual([["country", "descending"]]);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

// Issue #110: folding a group to its subtotal and opening it again — a button
// with `aria-expanded`, no new query, said in the status line.
test.describe("pivot folding", () => {
  const inside = (page, fn, arg) =>
    page.evaluate(
      ({ fn, arg }) =>
        new Function("root", "arg", `return (${fn})(root, arg)`)(
          document.querySelector("opengrid-pivot").shadowRoot,
          arg,
        ),
      { fn: fn.toString(), arg },
    );
  const deRows = (page) =>
    inside(page, (root) =>
      [...root.querySelectorAll('tbody tr[data-level="2"]')].filter(
        (tr) => tr.querySelector("th").textContent === "DE",
      ).length,
    );
  const toggle = (page) =>
    inside(page, (root) => {
      const button = root.querySelector('button[part="group-toggle"][data-path=\'["DE"]\']');
      return {
        name: [...button.childNodes]
          .filter((node) => node.getAttribute?.("aria-hidden") !== "true")
          .map((node) => node.textContent)
          .join(""),
        expanded: button.getAttribute("aria-expanded"),
        focused: root.activeElement === button,
      };
    });

  async function twoLevels(page) {
    await open(page);
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      pivot.setAttribute("columns", "");
      pivot.setAttribute("rows", "country,customer");
    });
    await expect.poll(() => deRows(page)).toBeGreaterThan(1);
    await page.evaluate(() => {
      window.__queries = 0;
      document
        .querySelector("opengrid-pivot")
        .addEventListener("opengrid-query", () => (window.__queries += 1));
    });
  }

  test("a group folds to its subtotal and opens again, from the keyboard", async ({ page }) => {
    await twoLevels(page);
    const open = await deRows(page);
    expect(await toggle(page)).toEqual({ name: "Total DE", expanded: "true", focused: false });
    // The grand total and the innermost level have no fold button.
    expect(
      await inside(page, (root) =>
        [...root.querySelectorAll('button[part="group-toggle"]')].every(
          (button) => button.closest("tr").dataset.level === "1",
        ),
      ),
    ).toBe(true);

    await inside(page, (root) =>
      root.querySelector('button[part="group-toggle"][data-path=\'["DE"]\']').focus(),
    );
    await page.keyboard.press("Enter");
    expect(await deRows(page)).toBe(0);
    expect(await toggle(page)).toEqual({ name: "Total DE", expanded: "false", focused: true });
    expect((await facts(page)).status).toBe("DE collapsed");
    expect(
      await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-pivot")).collapsed),
    ).toEqual([["DE"]]);

    await page.keyboard.press("Enter");
    expect(await deRows(page)).toBe(open);
    expect(await toggle(page)).toMatchObject({ expanded: "true", focused: true });
    expect((await facts(page)).status).toBe(`DE expanded, ${open} rows`);
    expect(await page.evaluate(() => document.querySelector("opengrid-pivot").hasAttribute("collapsed"))).toBe(false);
    // Folding asks nothing: the pivot draws what it holds.
    await page.waitForTimeout(200);
    expect(await page.evaluate(() => window.__queries)).toBe(0);
  });

  test("a folded view comes back, and the export stays whole", async ({ page }) => {
    await twoLevels(page);
    const before = await page.evaluate(() =>
      window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot")),
    );
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      const view = window.__opengridModule.get_view(pivot);
      window.__opengridModule.set_view(pivot, { ...view, collapsed: [["DE"]] });
    });
    // Zero rows of DE is also what a pivot shows while it loads.
    await expect.poll(async () => (await facts(page)).state).toBe("ready");
    expect(await deRows(page)).toBe(0);
    expect((await toggle(page)).expanded).toBe("false");
    expect(
      await page.evaluate(() => window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot"))),
    ).toBe(before);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

// Issue #112: the reader chooses the fields from a toolbar — menus, chips,
// moving and removing, all from the keyboard, each change one view.
test.describe("pivot field toolbar", () => {
  const inside = (page, fn, arg) =>
    page.evaluate(
      ({ fn, arg }) =>
        new Function("root", "arg", `return (${fn})(root, arg)`)(
          document.querySelector("opengrid-pivot").shadowRoot,
          arg,
        ),
      { fn: fn.toString(), arg },
    );
  const chips = (page) =>
    inside(page, (root) =>
      Object.fromEntries(
        [...root.querySelectorAll('[part="field-group"]')].map((group) => [
          group.dataset.axis,
          [...group.querySelectorAll('[part="chip"]')].map((chip) => chip.dataset.field),
        ]),
      ),
    );
  const focused = (page) =>
    inside(page, (root) => {
      const active = root.activeElement;
      return active
        ? {
            part: active.getAttribute("part"),
            role: active.getAttribute("role"),
            name: active.getAttribute("aria-label") ?? active.textContent,
          }
        : null;
    });
  const focus = (page, selector) => inside(page, (root, selector) => root.querySelector(selector).focus(), selector);

  async function withToolbar(page) {
    await open(page);
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      pivot.setAttribute("fields", "country,customer,ordered_year");
      pivot.setAttribute(
        "measures",
        '[{"fn":"count","as":"n"},{"field":"qty","fn":"sum","as":"total"},{"field":"qty","fn":"max","as":"most"}]',
      );
      pivot.setAttribute("toolbar", "");
    });
    await expect.poll(() => chips(page)).toEqual({
      rows: ["country"],
      columns: ["ordered_year"],
      values: ["total", "n"],
    });
    await expect.poll(async () => (await facts(page)).state).toBe("ready");
    await page.evaluate(() => {
      window.__views = [];
      document
        .querySelector("opengrid-pivot")
        .addEventListener("opengrid-view-change", (event) => window.__views.push(event.detail.view));
    });
  }

  test("the add buttons say what they can do", async ({ page }) => {
    await withToolbar(page);
    const buttons = await inside(page, (root) =>
      [...root.querySelectorAll('button[part="add-field"]')].map((button) => [
        button.textContent,
        button.getAttribute("aria-haspopup"),
        button.getAttribute("aria-disabled"),
        button.getAttribute("aria-label"),
      ]),
    );
    expect(buttons).toEqual([
      ["+ Row", "menu", null, null],
      // One column field in V1, and there is one.
      ["+ Column", "menu", "true", "One column field already"],
      ["+ Measure", "menu", null, null],
    ]);
  });

  test("a field is added from the menu, from the keyboard", async ({ page }) => {
    await withToolbar(page);
    await focus(page, 'button[data-add="rows"]');
    await page.keyboard.press("Enter");
    // The menu offers what is offered and not in use, and has the focus.
    const menu = await inside(page, (root) => {
      const menu = root.querySelector('[part="field-menu"]');
      return {
        role: menu.getAttribute("role"),
        items: [...menu.querySelectorAll('[role="menuitem"]')].map((item) => item.textContent),
        expanded: root.querySelector('button[data-add="rows"]').getAttribute("aria-expanded"),
      };
    });
    expect(menu).toEqual({ role: "menu", items: ["customer"], expanded: "true" });
    expect(await focused(page)).toMatchObject({ role: "menuitem", name: "customer" });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

    // Escape closes and goes back to the button.
    await page.keyboard.press("Escape");
    expect(await focused(page)).toMatchObject({ part: "add-field", name: "+ Row" });
    expect(await inside(page, (root) => root.querySelector('[part="field-menu"]'))).toBeNull();

    await page.keyboard.press("Enter");
    await page.keyboard.press("Enter");
    await expect.poll(() => chips(page)).toMatchObject({ rows: ["country", "customer"] });
    await expect.poll(async () => (await facts(page)).state).toBe("ready");
    expect(await page.evaluate(() => document.querySelector("opengrid-pivot").getAttribute("rows"))).toBe(
      "country,customer",
    );
    // One change, one report; the focus is back on the button — which says
    // now that every field is in use.
    expect(await page.evaluate(() => window.__views.map((view) => view.rows))).toEqual([["country", "customer"]]);
    await expect.poll(() => focused(page)).toMatchObject({ part: "add-field", name: "Nothing left to add" });
  });

  test("a field moves and goes, and the focus stays with it", async ({ page }) => {
    await withToolbar(page);
    await page.evaluate(() => document.querySelector("opengrid-pivot").setAttribute("rows", "country,customer"));
    await expect.poll(() => chips(page)).toMatchObject({ rows: ["country", "customer"] });

    await focus(page, 'button[aria-label="Move country later"]');
    await page.keyboard.press("Enter");
    await expect.poll(() => chips(page)).toMatchObject({ rows: ["customer", "country"] });
    // At the edge now, so the move back has the focus.
    await expect.poll(() => focused(page)).toMatchObject({ part: "chip-move", name: "Move country earlier" });

    await focus(page, 'button[aria-label="Remove customer"]');
    await page.keyboard.press("Enter");
    await expect.poll(() => chips(page)).toMatchObject({ rows: ["country"] });
    await expect.poll(() => focused(page)).toMatchObject({ part: "add-field", name: "+ Row" });

    // A measure goes the same way, and its column with it.
    await focus(page, 'button[aria-label="Remove total"]');
    await page.keyboard.press("Space");
    await expect.poll(() => chips(page)).toMatchObject({ values: ["n"] });
    await expect.poll(async () => (await facts(page)).measureHeaders).toEqual(["n", "n", "n"]);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

// Issue #28: the pivot in the browser. The engine in a worker answers the very
// pivot the server answers — same data, same element — so the two tables have
// to be the same, cell for cell.
test.describe("pivot in the browser", () => {
  /** Every cell of the pivot, header and body, as text. */
  async function cells(page, fixture) {
    await page.goto(`/tests/e2e/fixtures/${fixture}`);
    await page.waitForFunction(() => window.__ready === true);
    await expect(page.locator("opengrid-pivot tbody tr").first()).toBeVisible();
    return page.evaluate(() => {
      const root = document.querySelector("opengrid-pivot").shadowRoot;
      return [...root.querySelectorAll("tr")].map((row) =>
        [...row.querySelectorAll("th, td")].map((cell) => cell.textContent),
      );
    });
  }

  test("the worker's pivot is the server's pivot", async ({ page }) => {
    const server = await cells(page, "pivot.html");
    const worker = await cells(page, "pivot-worker.html");
    expect(worker.length).toBeGreaterThan(3);
    expect(worker).toEqual(server);
  });

  test("a pivot from the worker has no axe violations", async ({ page }) => {
    await cells(page, "pivot-worker.html");
    const { violations } = await new AxeBuilder({ page }).include("opengrid-pivot").analyze();
    expect(violations).toEqual([]);
  });
});
