import { test, expect } from "@playwright/test";

// Per-column presentation (plan point 60): `set_columns`.
//
// The rule under test is one sentence: **the configuration narrows, it never
// widens.** A page may say how a column looks; it may not say something the
// type contradicts. And nothing is dropped in silence — a name the schema does
// not have is a typo somebody has to find.

async function setColumns(page, config) {
  await page.evaluate(
    (value) =>
      window.__opengridModule.set_columns(document.querySelector("opengrid-grid"), value),
    config,
  );
}

async function settled(page) {
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
}

async function status(page) {
  return page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="status"]')
      .textContent.trim(),
  );
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-view.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await settled(page);
});

test("alignment follows the type, and a page may override it", async ({ page }) => {
  // Numbers right, text left: digits of the same magnitude have to stand under
  // each other or the column cannot be read down. That is derivable, so it is
  // not configuration — but it is taste, so it is overridable.
  const before = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const of = (col) => ({
      align: root.querySelector(`td[data-col="${col}"]`).dataset.align,
      text: getComputedStyle(root.querySelector(`td[data-col="${col}"]`)).textAlign,
    });
    return { id: of(0), customer: of(1), amount: of(3) };
  });
  expect(before.amount).toEqual({ align: "end", text: "right" });
  expect(before.customer).toEqual({ align: "start", text: "left" });
  expect(before.id).toEqual({ align: "end", text: "right" });

  await setColumns(page, { amount: { align: "start" } });
  await settled(page);

  const after = await page.evaluate(
    () =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('td[data-col="3"]').dataset
        .align,
  );
  expect(after).toBe("start");
});

test("the header lines up with the values under it", async ({ page }) => {
  // Otherwise the column reads as two columns.
  const pairs = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [0, 1, 2, 3, 4].map((col) => [
      root.querySelector(`th[data-col="${col}"]`).dataset.align,
      root.querySelector(`td[data-col="${col}"]`).dataset.align,
    ]);
  });
  for (const [head, cell] of pairs) {
    expect(head).toBe(cell);
  }
});

test("mono, emphasis and muted reach the cell", async ({ page }) => {
  // The three that exist because no schema says them: the prototype's `id` is
  // monospaced and grey, its `customer` is bold.
  await setColumns(page, {
    id: { mono: true, muted: true },
    customer: { emphasis: true },
  });
  await settled(page);

  const seen = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const id = root.querySelector('td[data-col="0"]');
    const customer = root.querySelector('td[data-col="1"]');
    const layout = getComputedStyle(root.querySelector('[part="layout"]'));
    return {
      idFont: getComputedStyle(id).fontFamily,
      idColor: getComputedStyle(id).color,
      inkColor: layout.color,
      customerWeight: getComputedStyle(customer).fontWeight,
    };
  });
  expect(seen.idFont.toLowerCase()).toMatch(/mono|menlo/);
  // Muted is a different ink from the ordinary one — not merely declared.
  expect(seen.idColor).not.toBe(seen.inkColor);
  expect(Number(seen.customerWeight)).toBeGreaterThan(500);
});

test("the values stand under their header when only some columns have a width", async ({ page }) => {
  // Every body row is its own table (it is absolutely positioned), so a width
  // written only onto the header left the rows splitting evenly: header and
  // values drifted apart as soon as one column had a width and the others did
  // not. Found on the project page's demo.
  const edges = () =>
    page.evaluate(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const left = (node) => Math.round(node.getBoundingClientRect().left);
      const row = [...root.querySelectorAll("tbody tr")].find((tr) => tr.querySelector("td[data-row]"));
      return {
        header: [...root.querySelectorAll("thead th[data-col]")].map(left),
        values: [...row.querySelectorAll("td[data-col]")].map(left),
      };
    });
  await setColumns(page, { id: { width: 64 }, amount: { width: 220 } });
  await settled(page);
  await expect.poll(async () => (await edges()).header[1]).toBeLessThan(200);
  const seen = await edges();
  expect(seen.values).toEqual(seen.header);

  // And after a resize by the reader, which writes the same way.
  await page.evaluate(() => {
    const th = document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="1"]');
    th.focus();
    th.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", ctrlKey: true, shiftKey: true, bubbles: true, composed: true }));
  });
  const resized = await edges();
  expect(resized.values).toEqual(resized.header);
});

test("a configured width starts the column, and a resize still wins", async ({ page }) => {
  await setColumns(page, { amount: { width: 200 } });
  await settled(page);

  const started = await page.evaluate(() =>
    Math.round(
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('th[data-col="3"]')
        .getBoundingClientRect().width,
    ),
  );
  expect(started).toBe(200);

  // Ctrl+Shift+Right resizes the focused column (point 36). The reader leads.
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const th = root.querySelector('th[data-col="3"]');
    th.focus();
    th.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "ArrowRight",
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
        composed: true,
      }),
    );
  });

  await expect
    .poll(() =>
      page.evaluate(() =>
        Math.round(
          document
            .querySelector("opengrid-grid")
            .shadowRoot.querySelector('th[data-col="3"]')
            .getBoundingClientRect().width,
        ),
      ),
    )
    .not.toBe(200);
});

test("a sum over a text column is refused and named", async ({ page }) => {
  // The whole point of the layer: summing text is not a preference, it is a
  // type error, and the configuration cannot buy what the schema forbids.
  await setColumns(page, { customer: { aggregate: "sum" } });

  const said = await status(page);
  expect(said).toContain("customer");
  expect(said).toContain("sum");
});

test("a facet that does not fit the type is refused", async ({ page }) => {
  await setColumns(page, { customer: { facet: "range" } });
  expect(await status(page)).toContain("range");

  await setColumns(page, { amount: { facet: "list" } });
  expect(await status(page)).toContain("list");
});

test("a column name the schema does not have is named, not ignored", async ({ page }) => {
  // The mistake this catches most often, and the one silence hides best.
  await setColumns(page, { amuont: { width: 100 } });
  expect(await status(page)).toContain("amuont");
});

test("an aggregate the type does allow is accepted", async ({ page }) => {
  // The mirror of the refusals: the layer narrows, it does not forbid.
  await setColumns(page, {
    amount: { aggregate: "sum", facet: "range" },
    customer: { aggregate: "count", facet: "list" },
    qty: { aggregate: "max" },
  });
  await settled(page);
  expect(await status(page)).not.toContain("not");
});

test("every problem is reported at once, not one per call", async ({ page }) => {
  // Finding the mistakes one release apart is the failure this avoids.
  await setColumns(page, {
    nope: { width: 10 },
    customer: { aggregate: "sum", align: "sideways" },
  });
  const said = await status(page);
  expect(said).toContain("nope");
  expect(said).toContain("sum");
  expect(said).toContain("sideways");
});
