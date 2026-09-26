import { test, expect } from "@playwright/test";

// The export notation in the browser (plan point 83): the element module's
// `export_csv` and `export_json` — the same Rust as the server's — over a real
// engine result. Internal to `exportRows` (point 84), tested here for what
// crosses the WASM boundary: the result JSON in, the options object read.

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-query.html");
  await page.waitForFunction(() => window.__opengridReady && window.__engine);
});

/** Runs a query on the engine and hands its result JSON to `fn`. */
function withResult(page, query, fn) {
  return page.evaluate(
    ({ query, fn }) => {
      const result = window.__engine.execute(JSON.stringify(query));
      // eslint-disable-next-line no-new-func
      return new Function("module", "result", fn)(window.__opengridModule, result);
    },
    { query, fn },
  );
}

const QUERY = {
  source: "orders",
  select: ["id", "customer", "amount"],
  sort: [{ field: "id", direction: "asc" }],
  limit: 2,
};

test("a CSV piece: header with the mark, then rows in CRLF", async ({ page }) => {
  const csv = await withResult(page, QUERY, "return module.export_csv(result, {}, true);");
  const lines = csv.split("\r\n");
  expect(lines[0]).toBe("﻿id,customer,amount");
  expect(lines).toHaveLength(4); // header, two rows, the empty rest after the last CRLF
  expect(lines[1]).toMatch(/^1,[A-Za-z]+,-?\d+\.\d{2}$/);
});

test("the options are read: semicolon, no mark, NULL spelled out, no header", async ({ page }) => {
  const csv = await withResult(
    page,
    QUERY,
    'return module.export_csv(result, { delimiter: ";", bom: false, null: "\\\\N" }, false);',
  );
  expect(csv.startsWith("1;")).toBe(true);
  expect(csv).not.toContain("﻿");
  const refused = await withResult(
    page,
    QUERY,
    'try { module.export_csv(result, { delimiter: "ab" }, true); return "no"; } catch (e) { return String(e.message); }',
  );
  expect(refused).toContain("delimiter");
  // A key of the wrong type is refused, not read as the default.
  const wrongType = await withResult(
    page,
    QUERY,
    'try { module.export_csv(result, { bom: "false" }, true); return "no"; } catch (e) { return String(e.message); }',
  );
  expect(wrongType).toContain("bom");
  const badNull = await withResult(
    page,
    QUERY,
    'try { module.export_csv(result, { null: "a,b" }, true); return "no"; } catch (e) { return String(e.message); }',
  );
  expect(badNull).toContain("null");
});

test("a JSON piece: row objects, a leading comma after the first piece", async ({ page }) => {
  const pieces = await withResult(
    page,
    QUERY,
    "return [module.export_json(result, true), module.export_json(result, false)];",
  );
  const rows = JSON.parse(`[${pieces[0]}${pieces[1]}]`);
  expect(rows).toHaveLength(4);
  expect(Object.keys(rows[0])).toEqual(["id", "customer", "amount"]);
  expect(typeof rows[0].amount).toBe("string");
  // An empty first piece: `first` stays true until a row is written.
  const afterEmpty = await withResult(
    page,
    { ...QUERY, limit: 0 },
    "return module.export_json(result, true);",
  );
  expect(afterEmpty).toBe("");
});
