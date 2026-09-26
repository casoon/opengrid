import { test, expect } from "@playwright/test";

// The lifecycle of an element (plan point 74).
//
// A page with routing mounts and unmounts grids all the time; a framework
// moves them around in keyed lists and parks them in caches like Vue's
// `<KeepAlive>`. Two promises follow, and they pull in opposite directions:
//
// - a grid that leaves for good is **collected** — its shadow tree, its loaded
//   window, and whatever the page handed only to it (the provider, a format
//   function);
// - a grid that is only moved, or put back later, is **the same grid** — the
//   view, the selection, the active cell, the scroll position.
//
// Freeing on `disconnectedCallback` would break the second; never freeing
// broke the first. The garbage collector is the only one who knows which case
// it is, so the collection tests force it (`--js-flags=--expose-gc`).

test.use({ launchOptions: { args: ["--js-flags=--expose-gc"] } });

// Firefox and WebKit ignore that flag and have no `window.gc`, so there the
// collection tests are skipped; what they check besides collection still runs.
const NO_GC = "window.gc is Chromium's (--expose-gc); no other engine can force a collection";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-lifecycle.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
});

/**
 * Forces collections until every `window.__weak[name]` is gone, and answers
 * the names still alive. A WeakRef target survives the job that created or
 * read it, so each round is its own `evaluate`, with a pause for the
 * finalization callbacks to run in between.
 */
async function collect(page, names) {
  let alive = names;
  for (let round = 0; round < 20 && alive.length > 0; round += 1) {
    await page.evaluate(() => window.gc());
    await page.waitForTimeout(50);
    alive = await page.evaluate(
      (names) => names.filter((name) => window.__weak[name].deref() !== undefined),
      names,
    );
  }
  return alive;
}

/** Waits until the grid in `#<slot>` shows rows. */
async function settled(page, slot) {
  await page.waitForFunction(
    (slot) =>
      !!document.querySelector(`#${slot} opengrid-grid`)?.shadowRoot?.querySelector("td[data-row]"),
    slot,
  );
}

/** A grid in `#a` with a provider, showing rows. */
async function mounted(page) {
  await page.evaluate(() => {
    const grid = window.__grid();
    document.getElementById("a").append(grid);
    window.__opengridModule.set_provider(grid, window.__provider());
  });
  await settled(page, "a");
}

/** What a reader would notice about the grid, wherever it sits now. */
async function snapshot(page) {
  return page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    const root = grid.shadowRoot;
    const viewport = root.querySelector('[part="viewport"]');
    const active = root.querySelector('[tabindex="0"]');
    return {
      view: window.__opengridModule.get_view(grid),
      selected: [...root.querySelectorAll("tbody tr")]
        .filter((tr) => tr.getAttribute("aria-selected") === "true")
        .map((tr) => tr.getAttribute("aria-rowindex"))
        .sort(),
      active: active && [active.getAttribute("data-row"), active.getAttribute("data-col")],
      scrollTop: viewport.scrollTop,
      // The lowest row shown, not the first in DOM order: a pool that was
      // rebuilt need not hand its slots out in the order the old one had.
      firstRow: Math.min(
        ...[...root.querySelectorAll("tbody tr[aria-rowindex]")].map((tr) =>
          Number(tr.getAttribute("aria-rowindex")),
        ),
      ),
      tables: root.querySelectorAll("table").length,
      filterValues: [...root.querySelectorAll('[part~="filter-value"]')].map((input) => input.value),
    };
  });
}

/**
 * Focuses the second row fully inside the viewport — one the focus does not
 * have to scroll to, so the active row stays inside the loaded window.
 */
async function focusVisibleRow(page) {
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const band = root.querySelector('[part="viewport"]').getBoundingClientRect();
    const header = root.querySelector("thead").getBoundingClientRect();
    const visible = [...root.querySelectorAll("tbody tr[aria-rowindex]")]
      .filter((tr) => {
        const rect = tr.getBoundingClientRect();
        return rect.top >= header.bottom && rect.bottom <= band.bottom;
      })
      .sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top);
    visible[1].querySelector("td[data-row]").focus();
  });
}

/** Whether the viewport shows rows, and whether its active cell is among them. */
async function onScreen(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const band = root.querySelector('[part="viewport"]').getBoundingClientRect();
    const inside = (rect) => rect.bottom > band.top && rect.top < band.bottom;
    const active = root.querySelector('td[tabindex="0"]');
    return {
      rows: [...root.querySelectorAll("tbody tr[aria-rowindex]")].filter((tr) =>
        inside(tr.getBoundingClientRect()),
      ).length,
      active: !!active && inside(active.getBoundingClientRect()),
    };
  });
}

/** A grid with a state worth keeping: sorted, filtered, scrolled, one row selected. */
async function worked(page) {
  await mounted(page);
  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    window.__opengridModule.set_view(grid, {
      sort: [{ field: "amount", direction: "desc" }],
      filters: [{ column: "country", op: "eq", value: "DE" }],
    });
  });
  await settled(page, "a");
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('[part="viewport"]').scrollTop = 600;
  });
  await page.waitForTimeout(150);
  await focusVisibleRow(page);
  await page.keyboard.press(" ");
  await expect
    .poll(async () => (await snapshot(page)).selected.length)
    .toBe(1);
  // The focus goes back to the page, so nothing below depends on where it was.
  await page.evaluate(() => document.activeElement?.blur());
  return snapshot(page);
}

test("a removed grid is collected, and with it what only the grid held", async ({
  page,
  browserName,
}) => {
  test.skip(browserName !== "chromium", NO_GC);
  await page.evaluate(async () => {
    const grid = window.__grid();
    // Both close over the grid — the everyday case of a callback defined next
    // to a component's ref. Held from WASM, that cycle would never be collected.
    const inner = window.__provider();
    const provider = { execute: (query) => (grid.isConnected, inner.execute(query)) };
    const format = (text) => (grid ? text : "");
    document.getElementById("a").append(grid);
    window.__opengridModule.set_formats(grid, { amount: format });
    window.__opengridModule.set_provider(grid, provider);
    await new Promise((resolve) => {
      const check = () =>
        grid.shadowRoot?.querySelector("td[data-row]") ? resolve() : requestAnimationFrame(check);
      check();
    });
    grid.remove();
    window.__weak = {
      grid: new WeakRef(grid),
      provider: new WeakRef(provider),
      format: new WeakRef(format),
    };
  });

  // The grid itself: nothing in the module may hold its shadow tree. The
  // provider and the format function: the page handed them to this grid only,
  // so once it is gone the module has to let go of them too.
  expect(await collect(page, ["grid", "provider", "format"])).toEqual([]);
});

test("a result that arrives after the grid left changes nothing and throws nothing", async ({
  page,
  browserName,
}) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.evaluate(async () => {
    const grid = window.__grid();
    const inner = window.__provider();
    let release;
    const gate = new Promise((resolve) => {
      release = resolve;
    });
    // Answers only when the test says so — after the grid is gone.
    const provider = { execute: (query) => gate.then(() => inner.execute(query)) };
    document.getElementById("a").append(grid);
    window.__opengridModule.set_provider(grid, provider);
    grid.remove();
    await new Promise((resolve) => setTimeout(resolve, 50));
    release();
    await new Promise((resolve) => setTimeout(resolve, 100));
    window.__weak = { grid: new WeakRef(grid) };
  });

  expect(errors).toEqual([]);
  if (browserName === "chromium") {
    expect(await collect(page, ["grid"])).toEqual([]);
  }
});

test("a removed table is collected too", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", NO_GC);
  await page.evaluate(async () => {
    const table = document.createElement("opengrid-table");
    table.setAttribute("datasource", "orders");
    table.setAttribute("columns", "id,customer");
    document.getElementById("a").append(table);
    window.__opengridModule.set_provider(table, window.__provider());
    await new Promise((resolve) => {
      const check = () =>
        table.shadowRoot?.querySelector("tbody td") ? resolve() : requestAnimationFrame(check);
      check();
    });
    table.remove();
    window.__weak = { table: new WeakRef(table) };
  });

  expect(await collect(page, ["table"])).toEqual([]);
});

test("a replaced provider is let go while the grid stays", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", NO_GC);
  await mounted(page);
  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    const first = window.__provider();
    window.__opengridModule.set_provider(grid, first);
    window.__weak = { first: new WeakRef(first) };
  });
  await settled(page, "a");
  await page.evaluate(() => {
    const grid = document.querySelector("opengrid-grid");
    window.__opengridModule.set_provider(grid, window.__provider());
  });
  await settled(page, "a");
  await page.waitForTimeout(100);

  expect(await collect(page, ["first"])).toEqual([]);
});

test("a grid moved within the page is the same grid", async ({ page }) => {
  // A keyed list moves a node with one insert: disconnect and connect in the
  // same task. Nothing about the grid may change, and nothing is asked again.
  const before = await worked(page);
  const queries = await page.evaluate(() => window.__queries.length);

  await page.evaluate(() => {
    document.getElementById("b").append(document.querySelector("opengrid-grid"));
  });
  await page.waitForTimeout(250);

  expect(await snapshot(page)).toEqual(before);
  expect((await onScreen(page)).rows).toBeGreaterThan(3);
  expect(await page.evaluate(() => window.__queries.length)).toBe(queries);
});

test("a grid put away and brought back is the same grid", async ({ page }) => {
  // Vue's `<KeepAlive>`, a tab panel that is detached rather than hidden: the
  // grid is out of the document long enough to let go of its DOM, and comes
  // back with everything the reader left it with — once, not twice.
  const before = await worked(page);
  const queries = await page.evaluate(() => window.__queries.length);

  await page.evaluate(() => {
    window.__parked = document.querySelector("opengrid-grid");
    window.__parked.remove();
  });
  await page.waitForTimeout(300);
  await page.evaluate(() => {
    document.getElementById("b").append(window.__parked);
    window.__parked = null;
  });
  await settled(page, "b");
  await page.waitForTimeout(250);

  expect(await snapshot(page)).toEqual(before);
  expect((await onScreen(page)).rows).toBeGreaterThan(3);
  expect(await page.evaluate(() => window.__queries.length)).toBe(queries);
});

test("a grid put back after its active row was scrolled away goes to that row", async ({
  page,
}) => {
  // The pin kept the active row in its old DOM slot while the reader scrolled
  // on; a new skeleton has no such slot and no data for that row. The grid goes
  // where Tab would take the reader anyway — its active cell — and loads that
  // window: one quiet query, and a viewport that is not empty.
  await mounted(page);
  await focusVisibleRow(page);
  await page.keyboard.press(" ");
  await page.evaluate(() => document.activeElement?.blur());
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('[part="viewport"]').scrollTop = 3000;
  });
  await page.waitForTimeout(250);
  const { view, selected } = await snapshot(page);
  const queries = await page.evaluate(() => window.__queries.length);

  await page.evaluate(() => {
    window.__parked = document.querySelector("opengrid-grid");
    window.__parked.remove();
  });
  await page.waitForTimeout(300);
  await page.evaluate(() => {
    document.getElementById("b").append(window.__parked);
    window.__parked = null;
  });
  await page.waitForTimeout(400);

  expect(await onScreen(page)).toEqual({ rows: expect.any(Number), active: true });
  expect((await onScreen(page)).rows).toBeGreaterThan(3);
  const after = await snapshot(page);
  expect(after.view).toEqual(view);
  expect(after.selected).toEqual(selected);
  expect(await page.evaluate(() => window.__queries.length)).toBe(queries + 1);
});

test("grids keep their column settings to themselves", async ({ page }) => {
  // Two stores used to share one id slot on the host with two counters: a grid
  // configured with `set_columns` before it was connected and a later grid
  // without could end up with the same id — and one layout between them.
  await page.evaluate(() => {
    const module = window.__opengridModule;
    const [a, b, c] = ["A", "B", "C"].map((label) => window.__grid(label));
    module.set_columns(a, { amount: { align: "start" } });
    module.set_columns(b, { amount: { align: "start" } });
    document.getElementById("a").append(a, b);
    document.getElementById("b").append(c);
    for (const grid of [a, b, c]) module.set_provider(grid, window.__provider());
  });
  await page.waitForFunction(() =>
    [...document.querySelectorAll("opengrid-grid")].every((grid) =>
      grid.shadowRoot?.querySelector("td[data-row]"),
    ),
  );

  await page.evaluate(() => {
    const a = document.querySelector('opengrid-grid[label="A"]');
    window.__opengridModule.set_view(a, { columns: { order: [], hidden: ["qty"], widths: {} } });
  });
  await page.waitForTimeout(250);

  const hidden = await page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll("opengrid-grid")].map((grid) => [
        grid.getAttribute("label"),
        window.__opengridModule.get_view(grid).columns.hidden,
      ]),
    ),
  );
  expect(hidden).toEqual({ A: ["qty"], B: [], C: [] });
});
