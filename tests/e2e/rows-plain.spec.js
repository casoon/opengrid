import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// Plain rows to a table without a schema (issue #460): `fromRows` infers one, loads the rows into
// the worker and connects the grid; `inferSchema` refuses what it cannot type, naming row and field.

const column = (page, col) =>
  shadow(page, (root, col) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.querySelector('td[data-col="0"]')?.textContent.trim())
      .map((tr) => tr.querySelector(`td[data-col="${col}"]`).textContent), col);

async function open(page) {
  const warnings = [];
  page.on("console", (message) => {
    if (message.type() === "warning") {
      warnings.push(message.text());
    }
  });
  await page.goto("/tests/e2e/fixtures/rows-plain.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid")).toBeVisible();
  return warnings;
}

const refusal = (page, rows) =>
  page.evaluate((rows) => {
    try {
      window.__inferSchema(rows);
      return null;
    } catch (error) {
      return { code: error.code, message: error.message };
    }
  }, rows);

test("plain objects become a grid, with the schema it inferred handed back", async ({ page }) => {
  const warnings = await open(page);
  await expect.poll(() => column(page, 1)).toEqual(["Ada", "Grace", "Edsger"]);
  expect(await column(page, 2)).toEqual(["1815-12-10", "1906-12-09", ""]);

  const shown = await page.evaluate(() => ({
    schema: JSON.parse(window.__shown.schema),
    datasource: document.querySelector("opengrid-grid").getAttribute("datasource"),
    columns: document.querySelector("opengrid-grid").getAttribute("columns"),
  }));
  expect(shown.schema.fields).toEqual([
    { name: "id", type: "int64", nullable: false },
    { name: "name", type: "utf8", nullable: false },
    { name: "born", type: "date", nullable: true },
    { name: "active", type: "bool", nullable: false },
    { name: "score", type: "float64", nullable: false },
  ]);
  expect(shown.datasource).toBe("rows");
  expect(shown.columns).toBe("id,name,born,active,score");

  // A field with fractions is float64, and the warning points to a decimal.
  expect(warnings.some((text) => text.includes("field score") && text.includes("decimal"))).toBe(true);
  expect(warnings.some((text) => text.includes("field id"))).toBe(false);

  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("a schema given wins over inference", async ({ page }) => {
  await open(page);
  const shown = await page.evaluate(async () => {
    const { fromRows } = await import("/packages/opengrid/loader.js");
    const grid = document.createElement("opengrid-grid");
    grid.setAttribute("label", "Codes");
    document.querySelector("main").append(grid);
    const schema = JSON.stringify({
      fields: [
        { name: "id", type: "utf8", nullable: false },
        { name: "name", type: "utf8", nullable: false },
      ],
    });
    const result = await fromRows(grid, [{ id: "a-1", name: "Barbara" }], { schema });
    return { same: result.schema === schema, columns: grid.getAttribute("columns") };
  });
  expect(shown).toEqual({ same: true, columns: "id,name" });
  await expect
    .poll(() =>
      page.evaluate(() =>
        [...document.querySelectorAll("opengrid-grid")[1].shadowRoot.querySelectorAll("tbody td")]
          .map((td) => td.textContent.trim())
          .filter(Boolean),
      ),
    )
    .toEqual(["a-1", "Barbara"]);
});

test("what inference cannot type is refused, naming row and field", async ({ page }) => {
  await open(page);
  const cases = [
    [[{ id: 1 }, { id: "x" }], "row 1, field id: a string where row 0 has a number"],
    [[{ id: 1 }, { id: 2 ** 53 }], "row 1, field id: 9007199254740992 is beyond 2^53"],
    [[{ tags: ["a"] }], "row 0, field tags: a list cannot be typed"],
    [[{ note: null }, {}], "field note is null or missing in every row"],
  ];
  for (const [rows, expected] of cases) {
    const error = await refusal(page, rows);
    expect(error?.code).toBe("cannot_infer");
    expect(error?.message).toContain(expected);
  }
  // A string that only looks like a day stays text; a field missing in a row is nullable.
  const schema = await page.evaluate(() =>
    JSON.parse(window.__inferSchema([{ day: "2026-02-30", n: 1 }, { day: "2026-02-28" }])),
  );
  expect(schema.fields).toEqual([
    { name: "day", type: "utf8", nullable: false },
    { name: "n", type: "int64", nullable: true },
  ]);
});

// `suggestColumns`: what the schema says about how the columns look, as the two objects a page hands
// on. A plain function — it counts nothing, the caller says how many values a field has.
const suggest = (page, schema, options) =>
  page.evaluate(
    ([schema, options]) => {
      try {
        return { ...window.__suggestColumns(schema, options) };
      } catch (error) {
        return { error: error.message };
      }
    },
    [schema, options],
  );

const schemaOf = (...fields) => JSON.stringify({ fields: fields.map(([name, type]) => ({ name, type, nullable: false })) });

test("suggestColumns reads the rules off the schema", async ({ page }) => {
  await open(page);
  const schema = schemaOf(
    ["id", "int64"],
    ["name", "utf8"],
    ["country", "utf8"],
    ["born", "date"],
    ["amount", { decimal: { precision: 12, scale: 2 } }],
    ["score", "float64"],
    ["qty", "int64"],
    ["active", "bool"],
    ["seen", "timestamp"],
  );
  const { columns, formats } = await suggest(page, schema, { rowKey: "id", distinct: { country: 5, name: 40, qty: 4 } });
  expect(columns).toEqual({
    id: { readonly: true, mono: true, muted: true, width: 96 },
    country: { facet: "list" },
    amount: { align: "end", aggregate: "sum" },
    score: { align: "end" },
    qty: { align: "end", aggregate: "sum", facet: "list" },
  });
  expect(formats).toEqual({ born: { kind: "date" } });
});

test("suggestColumns offers a facet only for a count it was handed, at most 12", async ({ page }) => {
  await open(page);
  const schema = schemaOf(["country", "utf8"], ["city", "utf8"], ["region", "utf8"]);
  const { columns } = await suggest(page, schema, { distinct: { country: 12, region: 13 } });
  expect(columns).toEqual({ country: { facet: "list" } });
  expect((await suggest(page, schema)).columns).toEqual({});
});

test("suggestColumns takes the key's rules and nothing else, and its result is a plain object to change", async ({ page }) => {
  await open(page);
  const schema = schemaOf(["id", "int64"], ["qty", "int64"]);
  const { columns } = await suggest(page, schema, { rowKey: "id", distinct: { id: 3 } });
  expect(columns.id).toEqual({ readonly: true, mono: true, muted: true, width: 96 });
  const changed = await page.evaluate((schema) => {
    const suggestions = window.__suggestColumns(schema, { rowKey: "id" });
    suggestions.columns.qty.aggregate = "avg";
    return window.__suggestColumns(schema, { rowKey: "id" }).columns.qty.aggregate;
  }, schema);
  expect(changed).toBe("sum");
});

test("suggestColumns names what is wrong with the call", async ({ page }) => {
  await open(page);
  const schema = schemaOf(["id", "int64"]);
  expect((await suggest(page, "not json")).error).toContain("not the JSON text of { fields");
  expect((await suggest(page, "{}")).error).toContain("not the JSON text of { fields");
  expect((await suggest(page, schema, { rowKey: "key" })).error).toContain("rowKey key is not a field");
  expect((await suggest(page, schema, { distinct: { nope: 1 } })).error).toContain("distinct names nope");
  expect((await suggest(page, schema, { distinct: { id: 1.5 } })).error).toContain("distinct id is 1.5");
  expect((await suggest(page, schema, { distinct: { id: -1 } })).error).toContain("distinct id is -1");
});

test("what suggestColumns gives is taken by a grid as it is", async ({ page }) => {
  await open(page);
  await expect.poll(() => column(page, 2)).toEqual(["1815-12-10", "1906-12-09", ""]);
  await page.evaluate(async () => {
    const loader = await window.__loadOpengrid();
    const grid = document.querySelector("opengrid-grid");
    const { columns, formats } = window.__suggestColumns(window.__shown.schema, { rowKey: "id", distinct: { active: 2 } });
    loader.module.set_columns(grid, columns);
    loader.module.set_formats(grid, formats);
  });
  // The date column is formatted now — the format the schema suggested — and nothing was refused.
  await expect.poll(() => column(page, 2)).not.toEqual(["1815-12-10", "1906-12-09", ""]);
  expect(await status(page)).not.toMatch(/not applied|unknown|refused|not a/i);
});
