import { test, expect } from "@playwright/test";

// A scaled integer column in `<opengrid-table>` (issue #537): `scale: 2` makes
// `price_cent` an int64 that stores 612 for what a reader reads as 6,12. The
// table shows the scaled number, in the page's notation and through the
// column's format, while what the page and the wire see stays the stored integer.
// Columns: id, price_cent (scale 2), qty (a plain integer), rating (a float).
// Rows 0 to 7 hold 612, 500, 499, 123450, 5, 250, NULL and 1000 cents.

const SHOWN = {
  de: ["6,12", "5,00", "4,99", "1.234,50", "0,05", "2,50", "", "10,00"],
  en: ["6.12", "5.00", "4.99", "1,234.50", "0.05", "2.50", "", "10.00"],
};
const WIRE = ["6.12", "5.00", "4.99", "1234.50", "0.05", "2.50", "", "10.00"];

/** The texts of a column's cells, in the order of the rows. */
const column = (page, name) =>
  page.evaluate((name) => {
    const root = document.querySelector("opengrid-table").shadowRoot;
    const index = [...root.querySelectorAll("thead th")].findIndex((th) => th.getAttribute("data-column") === name);
    return [...root.querySelectorAll("tbody tr")].map((tr) => tr.children[index].textContent);
  }, name);
const alert = (page) =>
  page.evaluate(() => document.querySelector("opengrid-table").shadowRoot.querySelector('[role="alert"]')?.textContent ?? "");

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/table-scaled.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(async () => (await column(page, "price_cent")).length).toBe(8);
}

for (const lang of ["de", "en"]) {
  test(`the cells show the scaled number in the page's notation (lang ${lang})`, async ({ page }) => {
    await open(page, `?lang=${lang}`);
    expect(await column(page, "price_cent")).toEqual(SHOWN[lang]);
    // The plain integer beside it is as it was.
    expect(await column(page, "qty")).toEqual(["1", "2", "3", "4", "5", "6", "7", "8"]);
  });
}

test("without a format the cell shows the scaled number in the wire notation", async ({ page }) => {
  await open(page, "?lang=de&format=none");
  expect(await column(page, "price_cent")).toEqual(WIRE);
});

test("a function format is given the scaled number", async ({ page }) => {
  await open(page, "?format=function");
  expect(await column(page, "price_cent")).toEqual(WIRE.map((text) => `[${text}]`));
  const seen = await page.evaluate(() => window.__seen);
  // The text of the scaled number, and the number as a decimal's JSON: a string.
  expect(seen[0]).toEqual(["6.12", "6.12"]);
  expect(seen.map(([text]) => text)).toContain("1234.50");
});

test("a sort and the query stay on the stored integer", async ({ page }) => {
  await open(page, "?lang=de");
  await page.evaluate(() =>
    document.querySelector("opengrid-table").shadowRoot.querySelector('th[data-column="price_cent"] button').click(),
  );
  await expect.poll(async () => (await column(page, "price_cent")).slice(0, 3)).toEqual(["0,05", "2,50", "4,99"]);
  // The order of the integers is the order of the numbers; a NULL is last.
  expect(await column(page, "price_cent")).toEqual(["0,05", "2,50", "4,99", "5,00", "6,12", "10,00", "1.234,50", ""]);
  // Nothing in what the table asked for is scaled: the field, its name, no value.
  const query = await page.evaluate(() => window.__queries.at(-1));
  expect(query.select).toContain("price_cent");
  expect(query.sort).toEqual([{ field: "price_cent", direction: "asc" }]);
  expect(JSON.stringify(query)).not.toMatch(/6[.,]12|\.05|scale/);
});

test("an export of the table's query is the stored integer", async ({ page }) => {
  await open(page, "?lang=de");
  const csv = await page.evaluate(async () => {
    const { exportRows } = await import("/packages/opengrid/loader.js");
    const blob = await exportRows(window.__provider, window.__queries.at(-1), { bom: false });
    return await blob.text();
  });
  const lines = csv.split("\r\n");
  expect(lines.slice(0, 5)).toEqual([
    "id,price_cent,qty,rating",
    "1,612,1,0.5",
    "2,500,2,0.5",
    "3,499,3,0.5",
    "4,123450,4,0.5",
  ]);
  expect(lines[7]).toBe("7,,7,0.5");
});

test("a scale the table cannot apply is reported, and none of the call applied", async ({ page }) => {
  await open(page, "?lang=de");
  const call = (columns) => page.evaluate((columns) => window.__module.set_columns(window.__host, columns), columns);

  // qty is a valid integer; rating is a float: the whole call is refused.
  await call({ qty: { scale: 2 }, rating: { scale: 2 } });
  await expect.poll(() => alert(page)).toContain("rating: a scale needs an int64 column");
  // A scale that is not a whole number of places.
  await call({ qty: { scale: 2.5 } });
  await expect.poll(() => alert(page)).toContain("qty: scale is not a whole number from 0 to 18");
  // A grid-only option is still refused beside it.
  await call({ price_cent: { scale: 2, aggregate: "sum" } });
  await expect.poll(() => alert(page)).toContain("price_cent: aggregate belongs to the grid, not to a table");

  // 0 is no scale: the stored integer as it is, and a larger one moves the point.
  await call({ price_cent: { scale: 0 } });
  await expect.poll(async () => (await column(page, "price_cent"))[0]).toBe("612,00");
  await call({ price_cent: { scale: 3 } });
  await expect.poll(async () => (await column(page, "price_cent"))[0]).toBe("0,61");
  await call({ qty: { scale: 1 } });
  await expect.poll(async () => (await column(page, "qty"))[0]).toBe("0.1");
});
