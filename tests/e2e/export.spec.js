import { test, expect } from "@playwright/test";

// `exportRows` (issue #1): every match of a query, through any provider, in
// pieces, as a Blob — with progress and cancellation.
//
// The claim is that the pieces add up to the engine's own answer, row for row.
// So the expected file is not written by hand: the engine in the tab runs the
// whole query at once, under the total order the export promises (the query's
// sort, then every other selected column ascending), and the element module
// writes it in one piece. The export, fetched in pieces through a provider, has
// to be that text exactly. Both sources hold the same 100 000 rows
// (fixtures/export-data.js).

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/export.html");
  await page.waitForFunction(() => window.__ready);
  await page.evaluate(() => window.__ready);
});

const ALL = ["id", "region", "amount", "day", "note"];

/** Every column, sorted by a key with ties (1 500 distinct days). */
const BY_DAY = {
  source: "export",
  select: ALL,
  sort: [{ field: "day", direction: "desc" }],
};
/** The total order the export promises for {@link BY_DAY}, spelled out. */
const BY_DAY_TOTAL = [
  { field: "day", direction: "desc" },
  { field: "id", direction: "asc" },
  { field: "region", direction: "asc" },
  { field: "amount", direction: "asc" },
  { field: "note", direction: "asc" },
];

/**
 * Exports `query` through the provider named `provider` (`__tab`, `__rest`)
 * and compares the text with the engine's answer under `total`. Answers what
 * a test needs to see, not the megabytes.
 */
function exportAndCompare(page, { provider, query, total, options = {} }) {
  return page.evaluate(
    async ({ provider, query, total, options }) => {
      const calls = [];
      const counted = {
        execute(json, mode, extra) {
          calls.push(JSON.parse(json));
          return window[provider].execute(json, mode, extra);
        },
      };
      const blob = await window.__exportRows(counted, query, options);
      // `Blob.text()` would drop the byte order mark; the file has it.
      const text = new TextDecoder("utf-8", { ignoreBOM: true }).decode(await blob.arrayBuffer());

      const whole = window.__engine.execute(JSON.stringify({ ...query, sort: total }));
      const { format = "csv", chunkSize, maxRows, onProgress, signal, ...csv } = options;
      const expected =
        format === "json"
          ? `[${window.__module.export_json(whole, true)}]`
          : window.__module.export_csv(whole, csv, true);
      let at = -1;
      if (text !== expected) {
        at = 0;
        while (text[at] === expected[at]) at += 1;
      }
      const around = (value) => value.slice(Math.max(0, at - 80), at + 80);
      return {
        type: blob.type,
        same: text === expected,
        // Where the two part, when they do: the export's text, then the engine's.
        near: at < 0 ? "" : `${around(text)}\n--- the engine's ---\n${around(expected)}`,
        length: text.length,
        pieces: calls.map(({ offset, limit }) => [offset, limit]),
        sorts: calls.map((call) => call.sort),
      };
    },
    { provider, query, total, options },
  );
}

test("100 000 rows over the tab, in pieces, are the engine's rows exactly", async ({ page }) => {
  const outcome = await exportAndCompare(page, {
    provider: "__tab",
    query: BY_DAY,
    total: BY_DAY_TOTAL,
  });
  expect(outcome.near).toBe("");
  expect(outcome.same).toBe(true);
  expect(outcome.type).toBe("text/csv;charset=utf-8");
  // Ten pieces of the default 10 000, one after the other.
  expect(outcome.pieces).toEqual(Array.from({ length: 10 }, (_, index) => [index * 10_000, 10_000]));
  // Every piece asks under the same total order: the tie-breaker is the
  // selected columns not yet in the sort, ascending, in column order.
  for (const sort of outcome.sorts) {
    expect(sort).toEqual(BY_DAY_TOTAL);
  }
});

test("the CSV options reach the notation, and nothing else does", async ({ page }) => {
  const outcome = await exportAndCompare(page, {
    provider: "__tab",
    query: BY_DAY,
    total: BY_DAY_TOTAL,
    options: { chunkSize: 9_000, delimiter: ";", bom: false, protectFormulas: false, null: "\\N" },
  });
  expect(outcome.near).toBe("");
  expect(outcome.same).toBe(true);
  expect(outcome.pieces).toHaveLength(12);
});

test("JSON over the tab: one array of row objects, the engine's", async ({ page }) => {
  const outcome = await exportAndCompare(page, {
    provider: "__tab",
    query: BY_DAY,
    total: BY_DAY_TOTAL,
    options: { format: "json", chunkSize: 9_000 },
  });
  expect(outcome.near).toBe("");
  expect(outcome.same).toBe(true);
  expect(outcome.type).toBe("application/json");
  // The last piece asks only for what is left of the total.
  expect(outcome.pieces).toEqual([
    ...Array.from({ length: 11 }, (_, index) => [index * 9_000, 9_000]),
    [99_000, 1_000],
  ]);
});

test.describe("over a real server", () => {
  test("100 000 rows over REST, in pieces, are the engine's rows exactly", async ({ page }) => {
    const outcome = await exportAndCompare(page, {
      provider: "__rest",
      query: BY_DAY,
      total: BY_DAY_TOTAL,
    });
    expect(outcome.near).toBe("");
    expect(outcome.same).toBe(true);
    expect(outcome.pieces).toHaveLength(10);
  });

  test("JSON over REST is the engine's too", async ({ page }) => {
    const outcome = await exportAndCompare(page, {
      provider: "__rest",
      query: { ...BY_DAY, select: ["id", "note"], sort: [{ field: "note", direction: "asc" }] },
      total: [
        { field: "note", direction: "asc" },
        { field: "id", direction: "asc" },
      ],
      options: { format: "json", chunkSize: 8_000 },
    });
    expect(outcome.near).toBe("");
    expect(outcome.same).toBe(true);
    expect(outcome.pieces).toHaveLength(13);
  });

  test("an abort stops the request in flight, and there is no Blob", async ({ page }) => {
    const failed = [];
    page.on("requestfailed", (request) => {
      if (request.url().includes("/query/export")) failed.push(request.failure()?.errorText);
    });
    const outcome = await page.evaluate(async () => {
      const controller = new AbortController();
      const requests = [];
      const provider = {
        execute(json, mode, options) {
          const request = window.__rest.execute(json, mode, options);
          const seen = { settled: "pending" };
          requests.push(seen);
          request.then(
            () => (seen.settled = "answered"),
            (error) => (seen.settled = error.name),
          );
          // The second piece is aborted while it is on its way.
          if (requests.length === 2) setTimeout(() => controller.abort(), 0);
          return request;
        },
      };
      let result;
      try {
        result = await window.__exportRows(provider, {
          source: "export",
          select: ["id", "region", "amount", "day", "note"],
          sort: [{ field: "day", direction: "desc" }],
        }, { signal: controller.signal });
      } catch (error) {
        result = error;
      }
      // Give a request that was not stopped the time to answer anyway.
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      return {
        name: result?.name,
        blob: result instanceof Blob,
        requests: requests.map((seen) => seen.settled),
      };
    });
    expect(outcome.blob).toBe(false);
    expect(outcome.name).toBe("AbortError");
    // The first piece was answered; the second was stopped, not left to finish;
    // no third was asked.
    expect(outcome.requests).toEqual(["answered", "AbortError"]);
    expect(failed).toEqual(["net::ERR_ABORTED"]);
  });

  test("the REST, pivot and hybrid providers hand the signal to fetch", async ({ page }) => {
    const names = await page.evaluate(async () => {
      const query = JSON.stringify({ source: "export", select: ["id"], limit: 1 });
      const pivot = JSON.stringify({ source: "export", rows: ["region"], values: [] });
      const stopped = AbortSignal.abort();
      const outcome = async (provider, json) => {
        try {
          await provider.execute(json, "", { signal: stopped });
          return "answered";
        } catch (error) {
          return error.name;
        }
      };
      return {
        rest: await outcome(window.__rest, query),
        pivot: await outcome(window.__pivot, pivot),
        hybrid: await outcome(window.__hybrid, query),
        // A provider asked with two arguments, as the grid asks, still answers.
        twoArguments: typeof (await window.__rest.execute(query, "")),
      };
    });
    expect(names).toEqual({
      rest: "AbortError",
      pivot: "AbortError",
      hybrid: "AbortError",
      twoArguments: "string",
    });
  });
});

test("a short piece ends the export, whatever the total said", async ({ page }) => {
  // The rows shrank after the first piece counted them: the total says 150 000,
  // the source has 100 000. The export ends with the first short piece.
  const outcome = await page.evaluate(async () => {
    let calls = 0;
    const shrinking = {
      execute(json, mode, options) {
        calls += 1;
        const result = JSON.parse(window.__tab.execute(json, mode, options));
        return JSON.stringify({ ...result, total_count: 150_000 });
      },
    };
    const seen = [];
    await window.__exportRows(shrinking, { source: "export", select: ["id"], sort: [] }, {
      onProgress: (step) => seen.push(step),
    });
    return { calls, last: seen.at(-1) };
  });
  expect(outcome).toEqual({ calls: 11, last: { rows: 100_000, total: 150_000 } });
});

test("a sort with many ties exports every row exactly once", async ({ page }) => {
  // PostgreSQL may order the rows of a tie differently in every statement, so
  // two `OFFSET` pieces can overlap and miss rows. This provider does that on
  // purpose — each piece breaks ties by `id` the other way round — unless the
  // query's order is already total over `id`.
  const outcome = await page.evaluate(async () => {
    let calls = 0;
    const unstable = {
      execute(json, mode, options) {
        const query = JSON.parse(json);
        calls += 1;
        if (!query.sort.some((key) => key.field === "id")) {
          query.sort = [...query.sort, { field: "id", direction: calls % 2 ? "asc" : "desc" }];
        }
        return window.__tab.execute(JSON.stringify(query), mode, options);
      },
    };
    const blob = await window.__exportRows(
      unstable,
      { source: "export", select: ["region", "id"], sort: [{ field: "region", direction: "asc" }] },
      { chunkSize: 7_000, bom: false },
    );
    const lines = (await blob.text()).split("\r\n").slice(1, -1);
    const ids = lines.map((line) => Number(line.split(",")[1]));
    const regions = lines.map((line) => line.split(",")[0]);
    return { calls, count: ids.length, unique: new Set(ids).size, regions: [...new Set(regions)] };
  });
  expect(outcome.calls).toBe(15);
  expect(outcome.count).toBe(100_000);
  expect(outcome.unique).toBe(100_000);
  // The ties stay together, in the sort's order; NULL last.
  expect(outcome.regions).toEqual(["centre", "east", "north", "south", "west", ""]);
});

test("progress after every piece, against the first piece's total", async ({ page }) => {
  const progress = await page.evaluate(async () => {
    const seen = [];
    await window.__exportRows(window.__tab, { source: "export", select: ["id"], sort: [] }, {
      chunkSize: 9_000,
      onProgress: (step) => seen.push(step),
    });
    return seen;
  });
  expect(progress).toEqual(
    Array.from({ length: 12 }, (_, index) => ({
      rows: Math.min((index + 1) * 9_000, 100_000),
      total: 100_000,
    })),
  );
});

test("an abort over the tab rejects at once and asks for nothing more", async ({ page }) => {
  const outcome = await page.evaluate(async () => {
    const controller = new AbortController();
    let calls = 0;
    const counted = {
      execute: (json, mode, options) => {
        calls += 1;
        return window.__tab.execute(json, mode, options);
      },
    };
    try {
      await window.__exportRows(counted, { source: "export", select: ["id"], sort: [] }, {
        signal: controller.signal,
        onProgress: () => controller.abort(),
      });
      return { name: "resolved", calls };
    } catch (error) {
      return { name: error.name, calls };
    }
  });
  expect(outcome).toEqual({ name: "AbortError", calls: 1 });
});

test("an abort during the last piece's progress still gives no Blob", async ({ page }) => {
  const name = await page.evaluate(async () => {
    const controller = new AbortController();
    try {
      await window.__exportRows(
        window.__tab,
        { source: "export", select: ["id"], filter: { field: "id", op: "lte", value: 5 }, sort: [] },
        { signal: controller.signal, onProgress: () => controller.abort() },
      );
      return "resolved";
    } catch (error) {
      return error.name;
    }
  });
  expect(name).toBe("AbortError");
});

test("an abort does not wait for a provider that cannot stop", async ({ page }) => {
  // Like the worker: the answer comes when it comes, whatever the signal says.
  const outcome = await page.evaluate(async () => {
    const controller = new AbortController();
    const deaf = {
      execute: (json) =>
        new Promise((resolve) => setTimeout(() => resolve(window.__tab.execute(json)), 3_000)),
    };
    const started = performance.now();
    setTimeout(() => controller.abort(), 50);
    try {
      await window.__exportRows(deaf, { source: "export", select: ["id"], sort: [] }, {
        signal: controller.signal,
      });
      return { name: "resolved" };
    } catch (error) {
      return { name: error.name, waited: performance.now() - started };
    }
  });
  expect(outcome.name).toBe("AbortError");
  expect(outcome.waited).toBeLessThan(1_000);
});

test("an empty result is a header, or an empty array", async ({ page }) => {
  const outcome = await page.evaluate(async () => {
    const query = {
      source: "export",
      select: ["id", "region"],
      filter: { field: "id", op: "lt", value: 0 },
      sort: [{ field: "id", direction: "asc" }],
    };
    let calls = 0;
    const counted = {
      execute: (json, mode, options) => {
        calls += 1;
        return window.__tab.execute(json, mode, options);
      },
    };
    const bytes = await (await window.__exportRows(counted, query)).arrayBuffer();
    const csv = new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes);
    const json = await (await window.__exportRows(counted, query, { format: "json" })).text();
    return { csv, json, calls };
  });
  expect(outcome.csv).toBe("﻿id,region\r\n");
  expect(outcome.json).toBe("[]");
  expect(outcome.calls).toBe(2);
});

test("more rows than maxRows is an error with a sentence, not a shorter file", async ({ page }) => {
  const outcome = await page.evaluate(async () => {
    let calls = 0;
    const counted = {
      execute: (json, mode, options) => {
        calls += 1;
        return window.__tab.execute(json, mode, options);
      },
    };
    const query = { source: "export", select: ["id"], sort: [] };
    let message;
    try {
      await window.__exportRows(counted, query, { maxRows: 99_999 });
    } catch (error) {
      message = error.message;
    }
    const fits = await window.__exportRows(counted, query, { maxRows: 100_000 });
    return { message, calls, fits: fits instanceof Blob };
  });
  expect(outcome.message).toContain("100000 rows match");
  expect(outcome.message).toContain("maxRows");
  // One piece to learn the total, then nothing; then ten for the one that fits.
  expect(outcome.calls).toBe(11);
  expect(outcome.fits).toBe(true);
});

test("a wrong option is refused before anything is asked", async ({ page }) => {
  const outcome = await page.evaluate(async () => {
    let calls = 0;
    const counted = {
      execute: (json, mode, options) => {
        calls += 1;
        return window.__tab.execute(json, mode, options);
      },
    };
    const query = { source: "export", select: ["id"], sort: [] };
    const refused = async (q, options) => {
      try {
        await window.__exportRows(counted, q, options);
        return "accepted";
      } catch (error) {
        return error.message;
      }
    };
    return {
      unknown: await refused(query, { filename: "orders.csv" }),
      csvOnJson: await refused(query, { format: "json", delimiter: ";" }),
      format: await refused(query, { format: "xlsx" }),
      delimiter: await refused(query, { delimiter: "ab" }),
      nullQuery: await refused(null, {}),
      windowed: await refused({ ...query, limit: 10 }, {}),
      calls,
    };
  });
  expect(outcome.unknown).toContain('unknown option "filename"');
  expect(outcome.csvOnJson).toContain('"delimiter" is a CSV option');
  expect(outcome.format).toContain("xlsx");
  expect(outcome.delimiter).toContain("delimiter");
  expect(outcome.nullQuery).toContain("get_query");
  expect(outcome.windowed).toContain("window");
  expect(outcome.calls).toBe(0);
});
