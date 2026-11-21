import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow } from "./helpers.js";

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
