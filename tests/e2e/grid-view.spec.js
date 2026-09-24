import { test, expect } from "@playwright/test";

// The view as a value (plan point 59).
//
// A "saved view" is nothing but serialized view state, so the element hands its
// view out and takes one back and the page does the naming, storing and
// deleting. These tests hold that contract: the round trip, the single query,
// the refusal of a view from somewhere else, and the deliberate absence of the
// selection.

/** `get_view(host)` through the loaded module. */
async function getView(page) {
  return page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
}

/** `set_view(host, view)` through the loaded module. */
async function setView(page, view) {
  await page.evaluate(
    (value) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), value),
    view,
  );
}

/** Waits until the grid has rows again. */
async function settled(page) {
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-view.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await settled(page);
});

test("a view read back and set again changes nothing", async ({ page }) => {
  // The whole module rests on this: if a view does not survive the round trip,
  // "save a view" is a guess.
  const before = await getView(page);
  expect(before).toMatchObject({
    sort: expect.any(Array),
    filters: expect.any(Array),
    columns: expect.any(Object),
    density: "normal",
  });

  const queries = await page.evaluate(() => window.__queries.length);
  await setView(page, before);
  const after = await getView(page);

  expect(after).toEqual(before);
  // Setting the view it already has is not a change, so it costs no query.
  expect(await page.evaluate(() => window.__queries.length)).toBe(queries);
});

test("a view carries sort, filters, columns and density", async ({ page }) => {
  await setView(page, {
    sort: [{ field: "amount", direction: "desc" }],
    filters: [{ column: "country", op: "eq", value: "DE" }],
    columns: { order: ["customer", "id"], hidden: ["qty"], widths: { amount: 180 } },
    density: "compact",
  });
  await settled(page);

  const view = await getView(page);
  expect(view.sort).toEqual([{ field: "amount", direction: "desc" }]);
  expect(view.filters).toEqual([{ column: "country", op: "eq", value: "DE" }]);
  expect(view.columns.hidden).toEqual(["qty"]);
  expect(view.columns.widths).toEqual({ amount: 180 });
  expect(view.density).toBe("compact");

  // And it is not only recorded — it is what the grid is actually showing.
  const shown = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      headers: [...root.querySelectorAll("thead th")].map((th) =>
        th.querySelector("span")?.textContent.trim(),
      ),
      density: document.querySelector("opengrid-grid").getAttribute("density"),
      rowHeight: Math.round(root.querySelector("tbody td").getBoundingClientRect().height),
    };
  });
  expect(shown.headers).not.toContain("qty");
  expect(shown.headers[0]).toBe("customer");
  expect(shown.density).toBe("compact");
  expect(shown.rowHeight).toBe(34);
});

test("restoring a view costs one query, not one per field", async ({ page }) => {
  // Field by field, a restore would flash through five intermediate results and
  // announce each of them — the reader would hear four states that never
  // existed. This is the same failure class announcements.spec.js was written
  // for, caught here at its source.
  await page.evaluate(() => {
    window.__queries.length = 0;
  });

  await setView(page, {
    sort: [{ field: "amount", direction: "desc" }],
    filters: [{ column: "country", op: "eq", value: "DE" }],
    columns: { order: [], hidden: ["qty"], widths: {} },
    density: "comfortable",
  });
  await settled(page);
  // Give any stray follow-up query a chance to show up before counting.
  await page.waitForTimeout(250);

  expect(await page.evaluate(() => window.__queries.length)).toBe(1);
});

test("a view without a sort still pages under a total order", async ({ page }) => {
  // S6: an offset without a sort names different rows on every run. A saved
  // view need not carry a sort; the grid falls back to its first column, the
  // same as clearing the last sort by hand.
  await setView(page, { sort: [{ field: "amount", direction: "desc" }] });
  await settled(page);
  await page.evaluate(() => {
    window.__queries.length = 0;
  });
  await setView(page, { filters: [{ column: "country", op: "eq", value: "DE" }] });
  await settled(page);
  const sent = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
  expect(sent.length).toBeGreaterThan(0);
  for (const query of sent) expect(query.sort?.length).toBeGreaterThan(0);
  expect((await getView(page)).sort).toEqual([{ field: "id", direction: "asc" }]);
});

test("a view applied from the page leaves the focus on the page", async ({ page }) => {
  // A page applies views from its own controls (the tabs of point 69); taking
  // the focus away would break their keyboard pattern.
  await page.evaluate(() => {
    const button = Object.assign(document.createElement("button"), { id: "outside", textContent: "Apply" });
    document.body.prepend(button);
    button.focus();
  });
  await setView(page, { sort: [{ field: "amount", direction: "desc" }] });
  await settled(page);
  expect(await page.evaluate(() => document.activeElement.id)).toBe("outside");
});

test("a view applied while the grid has the focus keeps it in the grid", async ({ page }) => {
  await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector("th[tabindex='0']").focus());
  await setView(page, { sort: [{ field: "amount", direction: "desc" }] });
  await settled(page);
  expect(await page.evaluate(() => !!document.querySelector("opengrid-grid").shadowRoot.activeElement)).toBe(true);
});

test("a view from another data source is refused, not half-applied", async ({ page }) => {
  const before = await getView(page);

  await setView(page, {
    sort: [{ field: "nope", direction: "asc" }],
    filters: [{ column: "country", op: "eq", value: "DE" }],
    density: "compact",
  });

  const status = await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent,
  );
  expect(status).toContain("nope");
  // Nothing of it stuck: the good half of a bad view is not applied either.
  expect(await getView(page)).toEqual(before);
});

test("the view event reports what has already happened", async ({ page }) => {
  // The contract of point 35: on the host, bubbles, composed, not cancelable.
  const seen = await page.evaluate(async () => {
    const host = document.querySelector("opengrid-grid");
    const events = [];
    // Listened for on `document`, not on the host: that only works because the
    // event is composed and bubbling.
    document.addEventListener("opengrid-view-change", (event) =>
      events.push({
        bubbles: event.bubbles,
        composed: event.composed,
        cancelable: event.cancelable,
        density: event.detail.view.density,
        target: event.target.tagName.toLowerCase(),
      }),
    );
    host.setAttribute("density", "comfortable");
    await new Promise((resolve) => setTimeout(resolve, 200));
    return events;
  });

  expect(seen.length).toBeGreaterThan(0);
  expect(seen.at(-1)).toMatchObject({
    bubbles: true,
    composed: true,
    cancelable: false,
    density: "comfortable",
    target: "opengrid-grid",
  });
});

test("sorting and hiding a column each report a new view", async ({ page }) => {
  const seen = await page.evaluate(async () => {
    const host = document.querySelector("opengrid-grid");
    const root = host.shadowRoot;
    const views = [];
    host.addEventListener("opengrid-view-change", (event) => views.push(event.detail.view));

    root.querySelector('th[data-col="0"]').focus();
    root.querySelector('th[data-col="0"]').dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true, composed: true }),
    );
    await new Promise((resolve) => setTimeout(resolve, 200));
    return views;
  });

  expect(seen.length).toBeGreaterThan(0);
  expect(seen.at(-1).sort.length).toBeGreaterThan(0);
});

test("a view has no selection, and applying one drops it", async ({ page }) => {
  // A selection names positions; the grid has no key column; sorting or
  // filtering puts different records in those positions. A restored selection
  // would not be incomplete, it would be wrong.
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const cell = root.querySelector('td[data-row="0"][data-col="0"]');
    cell.focus();
    cell.dispatchEvent(
      new KeyboardEvent("keydown", { key: " ", bubbles: true, composed: true }),
    );
  });
  await page.waitForFunction(
    () =>
      !!document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('tr[data-selected="true"]'),
  );

  const view = await getView(page);
  expect(view.selection).toBeUndefined();

  await setView(page, { ...view, density: "compact" });
  await settled(page);

  const stillSelected = await page.evaluate(
    () =>
      !!document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('tr[data-selected="true"]'),
  );
  expect(stillSelected).toBe(false);
});

test("the later fields are already in the shape", async ({ page }) => {
  // Points 62 and 66 add content to these, not structure. A view stored today
  // has to still read after they land.
  const view = await getView(page);
  expect(view.group).toEqual([]);
  expect(view.expanded).toEqual([]);
  expect(view.facets).toEqual({});
});
