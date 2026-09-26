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

  test.describe("in one request, through the provider's export", () => {
    /** The server's requests of this page, by path. */
    function watch(page) {
      const paths = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.port === "8082" && request.method() === "POST") paths.push(url.pathname);
      });
      return paths;
    }

    /** `exportRows` over `window.__rest` itself, which has `export`. */
    function exportDirect(page, { query, total, options = {} }) {
      return page.evaluate(
        async ({ query, total, options }) => {
          const progress = [];
          const blob = await window.__exportRows(window.__rest, query, {
            ...options,
            onProgress: (step) => progress.push(step),
          });
          const text = new TextDecoder("utf-8", { ignoreBOM: true }).decode(await blob.arrayBuffer());
          const whole = window.__engine.execute(JSON.stringify({ ...query, sort: total }));
          const { format = "csv", ...csv } = options;
          const expected =
            format === "json"
              ? `[${window.__module.export_json(whole, true)}]`
              : window.__module.export_csv(whole, csv, true);
          return { same: text === expected, length: text.length, type: blob.type, progress };
        },
        { query, total, options },
      );
    }

    test("the server's file is the pieces' file, byte for byte, in one request", async ({ page }) => {
      const paths = watch(page);
      const csv = await exportDirect(page, {
        query: BY_DAY,
        total: BY_DAY_TOTAL,
        options: { delimiter: ";", bom: false, protectFormulas: false, null: "\\N" },
      });
      expect(csv.same).toBe(true);
      expect(csv.type).toBe("text/csv;charset=utf-8");
      // Progress once, at the end — the count came before the rows.
      expect(csv.progress).toEqual([{ rows: 100_000, total: 100_000 }]);

      const json = await exportDirect(page, {
        query: { ...BY_DAY, select: ["id", "note"], sort: [{ field: "note", direction: "asc" }] },
        total: [
          { field: "note", direction: "asc" },
          { field: "id", direction: "asc" },
        ],
        options: { format: "json" },
      });
      expect(json.same).toBe(true);
      expect(json.type).toBe("application/json");
      // Two exports, two requests, and not one piece over `/query`.
      expect(paths).toEqual(["/export/export", "/export/export"]);
    });

    test("the file name and the row count reach a page on another origin", async ({ page }) => {
      const headers = await page.evaluate(async () => {
        const response = await fetch("http://127.0.0.1:8082/export/export?format=json", {
          method: "POST",
          headers: { Authorization: "Bearer e2e-token", "Content-Type": "application/json" },
          body: JSON.stringify({ source: "export", select: ["id"], filter: { field: "id", op: "lte", value: 3 } }),
        });
        return {
          status: response.status,
          name: response.headers.get("Content-Disposition"),
          rows: response.headers.get("X-Total-Count"),
          body: await response.text(),
        };
      });
      expect(headers).toEqual({
        status: 200,
        name: "attachment; filename=\"export.json\"; filename*=UTF-8''export.json",
        rows: "3",
        body: '[{"id":1},{"id":2},{"id":3}]',
      });
    });

    test("an abort during the download stops it, and there is no Blob", async ({ page }) => {
      // Whether the network layer still reports the request as failed depends
      // on how much of 7 MB over loopback it had already taken; what the page
      // gets does not. The server's side — a client that leaves ends the
      // query — is opengrid-server's `export_postgres` test.
      const outcome = await page.evaluate(async () => {
        const controller = new AbortController();
        // Aborted once the answer has begun — the status and the count are
        // in, the rows are on their way.
        const fetchBefore = window.fetch;
        let started = false;
        window.fetch = async (...args) => {
          const response = await fetchBefore(...args);
          started = true;
          controller.abort();
          return response;
        };
        const progress = [];
        try {
          const blob = await window.__exportRows(window.__rest, {
            source: "export",
            select: ["id", "region", "amount", "day", "note"],
            sort: [{ field: "note", direction: "desc" }],
          }, { signal: controller.signal, onProgress: (step) => progress.push(step) });
          return { blob: blob instanceof Blob, started, progress };
        } catch (error) {
          return { name: error.name, started, progress };
        } finally {
          window.fetch = fetchBefore;
        }
      });
      // Aborted after the answer began, and the end was never reached.
      expect(outcome).toEqual({ name: "AbortError", started: true, progress: [] });
    });

    test("a missing or unreadable row count is an error, not zero", async ({ page }) => {
      const outcome = await page.evaluate(async () => {
        const fetchBefore = window.fetch;
        const answer = async (count) => {
          window.fetch = async () =>
            new Response("id\r\n1\r\n2\r\n", {
              status: 200,
              headers: count === undefined ? {} : { "X-Total-Count": count },
            });
          try {
            await window.__rest.export({ source: "export", select: ["id"] }, { maxRows: 1 });
            return "a Blob";
          } catch (error) {
            return error.message;
          } finally {
            window.fetch = fetchBefore;
          }
        };
        return { missing: await answer(undefined), empty: await answer(""), text: await answer("many") };
      });
      for (const message of Object.values(outcome)) {
        expect(message).toContain("X-Total-Count");
      }
    });

    test("no request follows a redirect, so the token stays where it was sent", async ({ page }) => {
      const redirects = await page.evaluate(async () => {
        const fetchBefore = window.fetch;
        const seen = {};
        window.fetch = async (url, init) => {
          seen[new URL(url).pathname.split("/")[1]] = init?.redirect;
          return fetchBefore(url, init);
        };
        try {
          const query = { source: "export", select: ["id"], filter: { field: "id", op: "lte", value: 1 } };
          await window.__rest.describe();
          await window.__rest.execute(JSON.stringify(query), "");
          await window.__rest.export(query);
          await window.__pivot.execute(
            JSON.stringify({ source: "export", rows: ["region"], values: [{ fn: "count", as: "n" }] }),
            "",
          );
        } finally {
          window.fetch = fetchBefore;
        }
        return seen;
      });
      expect(redirects).toEqual({ source: "error", query: "error", export: "error", pivot: "error" });
    });

    test("maxRows refuses before the rows, and the server's sentences arrive", async ({ page }) => {
      const paths = watch(page);
      const outcome = await page.evaluate(async () => {
        const refused = async (query, options) => {
          try {
            await window.__exportRows(window.__rest, query, options);
            return "accepted";
          } catch (error) {
            return error.message;
          }
        };
        const query = { source: "export", select: ["id"], sort: [] };
        return {
          maxRows: await refused(query, { maxRows: 99_999 }),
          // Checked in the tab, before any request, as for pieces.
          delimiter: await refused(query, { delimiter: "ab" }),
          unknown: await refused(query, { filename: "x.csv" }),
          // The server's own: a column that does not exist for this token.
          field: await refused({ ...query, select: ["id", "no_such_column"] }, {}),
          direct: await window.__rest.export(query, { delimiter: "ab" }).then(
            () => "accepted",
            (error) => error.message,
          ),
        };
      });
      expect(outcome.maxRows).toContain("100000 rows match");
      expect(outcome.maxRows).toContain("maxRows");
      expect(outcome.delimiter).toContain("delimiter");
      expect(outcome.unknown).toContain('unknown option "filename"');
      expect(outcome.field).toContain("no_such_column");
      expect(outcome.direct).toContain("delimiter: one character");
      // maxRows and the two bad ones from the server; the tab's refusals asked nothing.
      expect(paths).toEqual(["/export/export", "/export/export", "/export/export"]);
    });
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

test("a source that changes during the export is refused, not exported", async ({ page }) => {
  // Rows that come or go between two pieces shift the windows: a row would
  // repeat or go missing. The tie-breaker cannot help there, so the export
  // says so instead of writing a file that is wrong without saying it.
  const outcome = await page.evaluate(async () => {
    const attempt = async (provider) => {
      let calls = 0;
      const counted = {
        execute(json, mode, options) {
          calls += 1;
          return provider(calls, JSON.parse(window.__tab.execute(json, mode, options)));
        },
      };
      try {
        await window.__exportRows(counted, { source: "export", select: ["id"], sort: [] });
        return { outcome: "a Blob", calls };
      } catch (error) {
        return { outcome: error.message, calls };
      }
    };
    return {
      // The count moves at the third piece.
      grown: await attempt((call, result) =>
        JSON.stringify({ ...result, total_count: call >= 3 ? 100_001 : result.total_count }),
      ),
      // The count holds at 150 000, the rows end at 100 000.
      ended: await attempt((call, result) => JSON.stringify({ ...result, total_count: 150_000 })),
    };
  });
  expect(outcome.grown.outcome).toContain("the source changed during the export");
  expect(outcome.grown.outcome).toContain("100000 rows matched at first, 100001");
  expect(outcome.grown.calls).toBe(3);
  expect(outcome.ended.outcome).toContain("the source changed during the export");
  expect(outcome.ended.outcome).toContain("ended at 100000 of 150000");
  expect(outcome.ended.calls).toBe(11);
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

test("without a unique column, the rows are the engine's rows all the same", async ({ page }) => {
  // `region, day` repeats: the tie-breaker cannot make the order total, and
  // the provider keeps shuffling what is left of a tie by `id`, a column the
  // export does not have. Those rows are equal in every exported column, so
  // the file is the same whichever copy comes — the multiset of lines is the
  // engine's.
  const outcome = await page.evaluate(async () => {
    let calls = 0;
    const shuffling = {
      execute(json, mode, options) {
        const query = JSON.parse(json);
        calls += 1;
        // A sort names output columns: `id` is fetched, ordered by, and
        // dropped again, the way a database orders by what it does not return.
        query.select = [...query.select, "id"];
        query.sort = [...query.sort, { field: "id", direction: calls % 2 ? "asc" : "desc" }];
        const result = JSON.parse(window.__tab.execute(JSON.stringify(query), mode, options));
        result.columns = result.columns.filter((column) => column.name !== "id");
        return JSON.stringify(result);
      },
    };
    const query = {
      source: "export",
      select: ["region", "day"],
      sort: [{ field: "region", direction: "asc" }],
    };
    const blob = await window.__exportRows(shuffling, query, { chunkSize: 7_000, bom: false });
    const exported = (await blob.text()).split("\r\n").slice(1, -1);
    const whole = window.__engine.execute(JSON.stringify({ source: "export", select: ["region", "day"] }));
    const engine = window.__module.export_csv(whole, { bom: false }, false).split("\r\n").slice(0, -1);
    const pairs = new Set(engine);
    return {
      calls,
      repeated: engine.length - pairs.size,
      same: JSON.stringify([...exported].sort()) === JSON.stringify([...engine].sort()),
    };
  });
  expect(outcome.calls).toBe(15);
  // There are ties the selected columns cannot break.
  expect(outcome.repeated).toBeGreaterThan(1_000);
  expect(outcome.same).toBe(true);
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
      // Its output is `region` and an alias `select` does not name; the
      // tie-breaker could not order by it.
      grouped: await refused(
        { source: "export", select: ["region", "rows"], group: ["region"], aggregate: [{ fn: "count", as: "rows" }], sort: [] },
        {},
      ),
      aggregated: await refused(
        { source: "export", select: ["rows"], aggregate: [{ fn: "count", as: "rows" }], sort: [] },
        {},
      ),
      calls,
    };
  });
  expect(outcome.unknown).toContain('unknown option "filename"');
  expect(outcome.csvOnJson).toContain('"delimiter" is a CSV option');
  expect(outcome.format).toContain("xlsx");
  expect(outcome.delimiter).toContain("delimiter");
  expect(outcome.nullQuery).toContain("get_query");
  expect(outcome.windowed).toContain("window");
  expect(outcome.grouped).toContain("groups or aggregates");
  expect(outcome.aggregated).toContain("groups or aggregates");
  expect(outcome.calls).toBe(0);
});
