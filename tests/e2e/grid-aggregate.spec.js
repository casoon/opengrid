import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Aggregates and the grand total (plan point 63).
//
// The fixture groups five rows by region and configures `amount: sum`,
// `qty: avg`, `day: max` through `set_columns`. The rows are chosen for what
// they break: North sums past 2^53, South has a NULL amount, one row has no
// region at all.

/** The drawn rows in display order, with each cell's text and spoken name. */
async function drawn(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.style.display)
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .map((tr) => ({
        kind: tr.dataset.kind ?? "row",
        index: Number(tr.getAttribute("aria-rowindex")),
        expanded: tr.getAttribute("aria-expanded"),
        cells: [...tr.querySelectorAll("td[data-col]")].map((td) => ({
          text: td.textContent,
          said: td.getAttribute("aria-label"),
          aggregate: td.dataset.aggregate ?? null,
        })),
      }));
  });
}

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-aggregate.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(async () => (await drawn(page)).at(-1)?.kind).toBe("total");
});

test("a sum past 2^53 is exact to the last digit", async ({ page }) => {
  // S8: a decimal stays exact. 9007199254740993.01 + 1.01 through an f64 would
  // come out as 9007199254740994 — the cents gone, and the units possibly too.
  const north = (await drawn(page))[0];
  expect(north.cells[0].text).toBe("region: North (2 rows)");
  expect(north.cells[2]).toEqual({
    text: "9007199254740994.02",
    said: "Sum: 9007199254740994.02",
    aggregate: "sum",
  });
});

test("NULL is skipped by a sum and an average (S11)", async ({ page }) => {
  const south = (await drawn(page))[1];
  // Its amounts are 10.00 and NULL: the sum is 10.00, not NULL and not an error.
  expect(south.cells[2].text).toBe("10.00");
  // qty 3 and 4 — the average of two values.
  expect(south.cells[3].text).toBe("3.5");
});

test("an aggregate over nothing says so in words, and draws no glyph", async ({ page }) => {
  // The group without a region has one row whose qty is NULL: the average of
  // nothing is NULL (S11). Empty on screen — a "⌀" alone would read as zero —
  // and named for a screen reader, not "Average: " and silence.
  const none = (await drawn(page))[2];
  expect(none.cells[0].text).toBe("region: (no value) (1 row)");
  expect(none.cells[3]).toEqual({ text: "", said: "Average: (no value)", aggregate: null });
});

test("the grand total is the last row, over every matching row", async ({ page }) => {
  const rows = await drawn(page);
  const total = rows.at(-1);
  expect(total.kind).toBe("total");
  expect(total.cells[0].text).toBe("Total (5 rows)");
  // 9007199254740994.02 + 10.00 + 5.00 — exact.
  expect(total.cells[2].text).toBe("9007199254741009.02");
  // qty 1, 2, 3, 4 and one NULL: the average of four values.
  expect(total.cells[3].text).toBe("2.5");
  expect(total.cells[4].text).toBe("2026-03-01");

  // It is a row of the list — counted, and at the end of it.
  const rowcount = await page.evaluate(() =>
    Number(document.querySelector("opengrid-grid").shadowRoot.querySelector("table").getAttribute("aria-rowcount")),
  );
  expect(total.index).toBe(rowcount);
  // It opens nothing, and does not claim it could.
  expect(total.expanded).toBeNull();
});

test("the keyboard reaches the total like any row", async ({ page }) => {
  // A sticky footer outside the list would need a second keyboard model; a
  // row at the end of the list needs none.
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-row="0"][data-col="0"]').focus();
  });
  await page.keyboard.press("Control+End");
  const focused = await page.evaluate(() => {
    const cell = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    return { kind: cell?.closest("tr")?.dataset.kind, said: cell?.getAttribute("aria-label") };
  });
  expect(focused).toEqual({ kind: "total", said: "Maximum: 2026-03-01" });

  // And Enter on it toggles nothing.
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  expect((await drawn(page)).length).toBe(4);
});

test("the header says which aggregate its column shows", async ({ page }) => {
  const marks = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("thead th[data-col]")].map(
      (th) => th.dataset.aggregate ?? null,
    ),
  );
  // id, region, amount, qty, day
  expect(marks).toEqual([null, null, "sum", "avg", "max"]);
});

test("nothing is summed that nobody chose", async ({ page }) => {
  // No default "sum every number": `id` is a number, and summing ids would put
  // a meaningless total in the first row a reader sees.
  const north = (await drawn(page))[0];
  expect(north.cells[1]).toEqual({ text: "", said: null, aggregate: null });
});

test("a data row carries no aggregate from a group row drawn in its slot before", async ({
  page,
}) => {
  // The pool recycles `<tr>`s: a slot that drew a group row a frame ago would
  // otherwise keep its `aria-label` and say "Sum: …" about an ordinary value.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-row="0"][data-col="1"]').click(),
  );
  await expect.poll(async () => (await drawn(page))[1]?.kind).toBe("row");
  for (const row of (await drawn(page)).filter((row) => row.kind === "row")) {
    for (const cell of row.cells) {
      expect(cell.said).toBeNull();
      expect(cell.aggregate).toBeNull();
    }
  }
});

test("the reader's choice leads over the configuration, and travels in the view", async ({
  page,
}) => {
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  expect(view.aggregates).toEqual({});

  await page.evaluate(
    (view) =>
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        ...view,
        aggregates: { amount: "count" },
      }),
    view,
  );
  await expect.poll(async () => (await drawn(page))[0]?.cells[2]?.said).toBe("Count: 2");

  const after = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  expect(after.aggregates).toEqual({ amount: "count" });
});

test("a range shows the dates a group spans, in one query", async ({ page }) => {
  // F7, decided 2026-09-24: "from – to", asked as `min` and `max` of the column
  // inside the same group query — not a query of its own.
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  await page.evaluate(() => {
    window.__queries.length = 0;
  });
  await page.evaluate(
    (view) =>
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        ...view,
        aggregates: { day: "range" },
      }),
    view,
  );
  await expect.poll(async () => (await drawn(page))[0]?.cells[4]?.text).toBe("2026-01-01 – 2026-01-05");
  const rows = await drawn(page);
  expect(rows[0].cells[4]).toEqual({
    text: "2026-01-01 – 2026-01-05",
    said: "Range: 2026-01-01 – 2026-01-05",
    aggregate: "range",
  });
  // One day only: said once, not "2026-03-01 – 2026-03-01".
  expect(rows[2].cells[4].text).toBe("2026-03-01");
  // The total spans every matching row.
  expect(rows.at(-1).cells[4].text).toBe("2026-01-01 – 2026-03-01");

  const grouped = await page.evaluate(() =>
    window.__queries.map((q) => JSON.parse(q)).filter((q) => q.group?.[0] === "region"),
  );
  expect(grouped.length).toBe(1);
  expect(grouped[0].aggregate.filter((a) => a.field === "day").map((a) => a.fn)).toEqual(["min", "max"]);

  const header = await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="4"]').dataset.aggregate,
  );
  expect(header).toBe("range");
  expect(
    (await page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")))).aggregates,
  ).toEqual({ day: "range" });
});

test("a range over text is refused like any aggregate the type does not allow", async ({ page }) => {
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  await page.evaluate(
    (view) =>
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        ...view,
        aggregates: { region: "range" },
      }),
    view,
  );
  await expect.poll(() => status(page)).toContain("region: range is not an aggregate for this type");
});

test("a chosen aggregate the type does not allow is named", async ({ page }) => {
  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  await page.evaluate(
    (view) =>
      window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
        ...view,
        aggregates: { region: "sum" },
      }),
    view,
  );
  await expect.poll(() => status(page)).toContain("region: sum is not an aggregate for this type");
});

test("has no axe violations with aggregates and the total", async ({ page }) => {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-row="0"][data-col="1"]').click(),
  );
  await expect.poll(async () => (await drawn(page))[1]?.kind).toBe("row");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
