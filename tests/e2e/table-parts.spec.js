import { test, expect } from "@playwright/test";

// `<opengrid-table>` and `<opengrid-pivot>` are styled through their parts, and
// the table shows its values the way the grid does (issue #29).

/** The table's shadow root, evaluated in the page. */
const cells = (page, column) =>
  page.evaluate(
    (column) =>
      [
        ...document
          .querySelector("opengrid-table")
          .shadowRoot.querySelectorAll(`tbody tr td:nth-child(${column})`),
      ].map((td) => td.textContent),
    column,
  );

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/table-data.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-table")?.shadowRoot?.querySelector("tbody tr"),
  );
});

test("page CSS reaches the table through its parts", async ({ page }) => {
  await page.addStyleTag({
    content: `
      opengrid-table::part(cell) { padding-left: 17px; }
      opengrid-table::part(header) { padding-left: 13px; }
      opengrid-table::part(caption) { letter-spacing: 3px; }
    `,
  });
  const styles = await page.evaluate(() => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    const style = (selector, property) =>
      getComputedStyle(root.querySelector(selector)).getPropertyValue(property);
    return {
      cell: style("td", "padding-left"),
      header: style("th", "padding-left"),
      caption: style("caption", "letter-spacing"),
    };
  });
  expect(styles).toEqual({ cell: "17px", header: "13px", caption: "3px" });
});

test("the table applies the page's formats and column presentation", async ({ page }) => {
  // The plain notation first: decimals exact, numbers right-aligned by type.
  expect((await cells(page, 2))[0]).toBe("30.00");

  await page.evaluate(() => {
    const host = document.querySelector("opengrid-table");
    window.__module.set_formats(host, { amount: (text) => `${text} €` });
    window.__module.set_columns(host, {
      customer: { title: "Kunde", emphasis: true },
      qty: { align: "center" },
    });
  });
  await expect.poll(async () => (await cells(page, 2))[0]).toBe("30.00 €");

  const facts = await page.evaluate(() => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    const first = root.querySelector("tbody tr");
    return {
      title: root.querySelector('th[data-column="customer"] button > span').textContent,
      weight: getComputedStyle(first.children[0]).fontWeight,
      amountAlign: getComputedStyle(first.children[1]).textAlign,
      qtyAlign: getComputedStyle(first.children[2]).textAlign,
    };
  });
  expect(facts).toEqual({
    title: "Kunde",
    weight: "600",
    amountAlign: "right",
    qtyAlign: "center",
  });
});

test("a grid-only option is refused, not ignored", async ({ page }) => {
  await page.evaluate(() =>
    window.__module.set_columns(document.querySelector("opengrid-table"), {
      amount: { width: 120 },
    }),
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          document.querySelector("opengrid-table").shadowRoot.querySelector('[role="alert"]')
            ?.textContent ?? "",
      ),
    )
    .toContain("width belongs to the grid");
});

test("page CSS reaches the pivot through its parts", async ({ page }) => {
  // The worker fixture: the parts do not depend on where the pivot ran.
  await page.goto("/tests/e2e/fixtures/pivot-worker.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-pivot tbody tr").first()).toBeVisible();
  await page.addStyleTag({
    content: `
      opengrid-pivot::part(header) { padding-left: 11px; }
      opengrid-pivot::part(row-header) { padding-left: 13px; }
      opengrid-pivot::part(cell) { padding-left: 17px; }
      opengrid-pivot::part(total-row) { letter-spacing: 2px; }
    `,
  });
  const styles = await page.evaluate(() => {
    const root = document.querySelector("opengrid-pivot").shadowRoot;
    const style = (selector, property) =>
      getComputedStyle(root.querySelector(selector)).getPropertyValue(property);
    return {
      header: style('thead th[scope="col"]', "padding-left"),
      rowHeader: style('tbody th[scope="row"]', "padding-left"),
      cell: style("tbody td", "padding-left"),
      total: style("tbody tr[data-total]", "letter-spacing"),
    };
  });
  expect(styles).toEqual({ header: "11px", rowHeader: "13px", cell: "17px", total: "2px" });
});
