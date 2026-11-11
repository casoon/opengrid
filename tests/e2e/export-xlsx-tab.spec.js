import { test, expect } from "@playwright/test";

import { unzip } from "./helpers.js";

// A workbook written in the tab (issue #332): `exportRows(…, { format: "xlsx" })`
// over the engine in the tab, with the module of its own (`xlsx/`) that is
// loaded the first time.
//
// The file is opened here, not only received: the archive is read with
// `node:zlib`, the sheet's cells are taken from its XML, and every row is held
// against the engine's own answer to the same query.

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/export.html");
  await page.waitForFunction(() => window.__ready);
  await page.evaluate(() => window.__ready);
});

const SHEET_TYPE = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/** 2 500 rows in `id` order: three pieces of 1 000, 1 000 and 500. */
const SMALL = {
  source: "export",
  select: ["id", "region", "amount", "day"],
  filter: { field: "id", op: "lte", value: 2500 },
  sort: [{ field: "id", direction: "asc" }],
};

const unescape = (text) =>
  text
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&amp;", "&");

/** The first sheet as rows of `{ type, value }` by column letter, from the archive's XML. */
function readSheet(base64) {
  const files = unzip(Buffer.from(base64, "base64"));
  // A workbook written in memory keeps its texts in the shared table.
  const shared = [...(files.get("xl/sharedStrings.xml")?.toString() ?? "").matchAll(/<si>(.*?)<\/si>/gs)].map(
    (item) => unescape(item[1].match(/<t[^>]*>(.*?)<\/t>/s)?.[1] ?? ""),
  );
  const xml = files.get("xl/worksheets/sheet1.xml").toString();
  const rows = [];
  for (const row of xml.matchAll(/<row [^>]*>(.*?)<\/row>/gs)) {
    const cells = {};
    for (const cell of row[1].matchAll(/<c r="([A-Z]+)\d+"([^>]*?)(?:\/>|>(.*?)<\/c>)/gs)) {
      const [, column, attributes, body = ""] = cell;
      const type = attributes.match(/ t="([^"]*)"/)?.[1] ?? "n";
      const raw = body.match(/<v>(.*?)<\/v>/s)?.[1] ?? body.match(/<t[^>]*>(.*?)<\/t>/s)?.[1] ?? "";
      cells[column] = { type, value: type === "s" ? shared[Number(raw)] : unescape(raw) };
    }
    rows.push(cells);
  }
  return { rows, files };
}

/** Exports `query` through the tab, and answers the file as base64 with what the provider saw. */
function exportXlsx(page, query, options = {}) {
  return page.evaluate(
    async ({ query, options }) => {
      const calls = [];
      const progress = [];
      const counted = {
        execute(json, mode, extra) {
          calls.push(JSON.parse(json));
          return window.__tab.execute(json, mode, extra);
        },
      };
      const blob = await window.__exportRows(counted, query, {
        format: "xlsx",
        onProgress: (step) => progress.push(step),
        ...options,
      });
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let binary = "";
      for (let at = 0; at < bytes.length; at += 8192) {
        binary += String.fromCharCode(...bytes.subarray(at, at + 8192));
      }
      const whole = JSON.parse(window.__engine.execute(JSON.stringify(query)));
      return {
        type: blob.type,
        size: blob.size,
        base64: btoa(binary),
        progress,
        pieces: calls.map(({ offset, limit }) => [offset, limit]),
        columns: whole.columns,
      };
    },
    { query, options },
  );
}

test("the rows of the engine in the tab, in pieces, are the rows of the sheet", async ({ page }) => {
  const outcome = await exportXlsx(page, SMALL, { chunkSize: 1000 });
  expect(outcome.type).toBe(SHEET_TYPE);
  // Three pieces, asked for as the CSV asks for them, and the progress after each.
  expect(outcome.pieces).toEqual([
    [0, 1000],
    [1000, 1000],
    [2000, 500],
  ]);
  expect(outcome.progress.at(-1)).toEqual({ rows: 2500, total: 2500 });

  const { rows } = readSheet(outcome.base64);
  // One header, and every row once.
  expect(rows).toHaveLength(2501);
  expect(Object.values(rows[0]).map((cell) => cell.value)).toEqual(["id", "region", "amount", "day"]);
  const [id, region, amount, day] = outcome.columns;
  for (let at = 0; at < 2500; at++) {
    const cells = rows[at + 1];
    const spot = `row ${at + 1}`;
    expect(Number(cells.A.value), `${spot}: id`).toBe(Number(id.values[at]));
    if (region.values[at] === null) {
      expect(cells.B?.value ?? "", `${spot}: region`).toBe("");
    } else {
      expect(cells.B.value, `${spot}: region`).toBe(region.values[at]);
    }
    expect(Number(cells.C.value), `${spot}: amount`).toBe(Number(amount.values[at]));
    const [year, month, date] = day.values[at].split("-").map(Number);
    // An Excel date is the days since 1899-12-30.
    expect(Number(cells.D.value), `${spot}: day`).toBe(Date.UTC(year, month - 1, date) / 86_400_000 + 25_569);
  }
});

test("the sheet is named after the source and its header row is frozen", async ({ page }) => {
  const outcome = await exportXlsx(page, { ...SMALL, filter: { field: "id", op: "lte", value: 3 } });
  const { files } = readSheet(outcome.base64);
  expect(files.get("xl/workbook.xml").toString()).toContain('name="export"');
  expect(files.get("xl/worksheets/sheet1.xml").toString()).toMatch(/<pane [^>]*ySplit="1"/);
});

test("the module is fetched when a workbook is asked for, not before", async ({ page }) => {
  const fetched = [];
  page.on("request", (request) => {
    if (request.url().includes("/xlsx/")) fetched.push(new URL(request.url()).pathname.split("/").pop());
  });
  // A CSV and a JSON export do not need it.
  await page.evaluate((query) => window.__exportRows(window.__tab, query, { format: "csv" }), SMALL);
  await page.evaluate((query) => window.__exportRows(window.__tab, query, { format: "json" }), SMALL);
  expect(fetched).toEqual([]);
  // The first workbook loads it; the second reuses it.
  await exportXlsx(page, { ...SMALL, filter: { field: "id", op: "lte", value: 10 } });
  expect(fetched.sort()).toEqual(["opengrid_xlsx.js", "opengrid_xlsx_bg.wasm"]);
  await exportXlsx(page, { ...SMALL, filter: { field: "id", op: "lte", value: 10 } });
  expect(fetched).toHaveLength(2);
});

test("more rows than maxRows is an error with a sentence, before a workbook is begun", async ({ page }) => {
  const failure = await page.evaluate(async (query) => {
    try {
      await window.__exportRows(window.__tab, query, { format: "xlsx", maxRows: 100 });
      return null;
    } catch (error) {
      return { code: error.code, message: error.message };
    }
  }, SMALL);
  expect(failure.code).toBe("too_many_rows");
  expect(failure.message).toContain("2500 rows match, more than the 100");
});

test("more rows than an Excel sheet holds is an error, never a shortened file", async ({ page }) => {
  // A source that says it has 2 000 000 rows, with the first piece's one row.
  const failure = await page.evaluate(async (query) => {
    const provider = {
      execute: () =>
        JSON.stringify({
          total_count: 2_000_000,
          row_count: 1,
          columns: [{ name: "id", type: "int64", nullable: false, values: [1] }],
        }),
    };
    try {
      await window.__exportRows(provider, { source: "export", select: ["id"], sort: [] }, { format: "xlsx", maxRows: 5_000_000 });
      return null;
    } catch (error) {
      return { code: error.code, message: error.message };
    }
  });
  expect(failure.code).toBe("too_many_rows");
  expect(failure.message).toContain("more than the 1048575 an Excel sheet holds");
});

test("an abort gives no file, and a module that did not load is said, and tried again later", async ({ page }) => {
  const aborted = await page.evaluate(async (query) => {
    const controller = new AbortController();
    controller.abort();
    try {
      await window.__exportRows(window.__tab, query, { format: "xlsx", signal: controller.signal });
      return null;
    } catch (error) {
      return error.name;
    }
  }, SMALL);
  expect(aborted).toBe("AbortError");

  await page.route("**/xlsx/opengrid_xlsx_bg.wasm", (route) => route.abort());
  const failed = await page.evaluate(async (query) => {
    try {
      await window.__exportRows(window.__tab, query, { format: "xlsx" });
      return null;
    } catch (error) {
      return { code: error.code, message: error.message };
    }
  }, SMALL);
  expect(failed.code).toBe("module_not_loaded");
  expect(failed.message).toContain("the XLSX module did not load");

  // The failure is not kept: with the module reachable again the same call works.
  await page.unroute("**/xlsx/opengrid_xlsx_bg.wasm");
  const outcome = await exportXlsx(page, { ...SMALL, filter: { field: "id", op: "lte", value: 5 } });
  expect(readSheet(outcome.base64).rows).toHaveLength(6);
});

test("a workbook in the tab has a lower default maxRows, which the page may raise", async ({ page }) => {
  const outcome = await page.evaluate(async () => {
    const provider = {
      execute: () =>
        JSON.stringify({
          total_count: 300_000,
          row_count: 1,
          columns: [{ name: "id", type: "int64", nullable: false, values: [1] }],
        }),
    };
    const query = { source: "export", select: ["id"], sort: [] };
    try {
      await window.__exportRows(provider, query, { format: "xlsx" });
      return { refused: false };
    } catch (error) {
      return { refused: true, code: error.code, message: error.message };
    }
  });
  expect(outcome.refused).toBe(true);
  expect(outcome.code).toBe("too_many_rows");
  expect(outcome.message).toContain("300000 rows match, more than the 250000");
});
