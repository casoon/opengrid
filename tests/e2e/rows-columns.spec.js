import { test, expect } from "@playwright/test";

// Rows as packs of columns (issue #445): `loadRows` of an array of objects makes the table the
// JSON route makes of the same rows — the same bytes in every answer, or the same refusal — for
// plain rows, for odd ones that take the JSON route, and for tables cut into several packs.

const SCHEMA = {
  fields: [
    { name: "id", type: "int64", nullable: false },
    { name: "flag", type: "bool", nullable: true },
    { name: "note", type: "utf8", nullable: true },
    { name: "ratio", type: "float64", nullable: true },
    { name: "amount", type: { decimal: { precision: 12, scale: 2 } }, nullable: true },
    { name: "day", type: "date", nullable: true },
    { name: "made", type: "timestamp", nullable: true },
    { name: "day_year", type: "int64", nullable: true, from: { part: "year", field: "day" } },
    { name: "made_month", type: "int64", nullable: true, from: { part: "month", field: "made" } },
  ],
};

async function open(page) {
  await page.goto("/tests/e2e/fixtures/rows-columns.html");
  await page.waitForFunction(() => window.__ready === true);
}

/**
 * Loads `rows` over `kind` of provider by the columns route and by the JSON route (the same
 * rows as text), and answers with what each made: every answer as base64, or the refusal.
 */
const compare = (page, kind, makeRows) =>
  page.evaluate(
    async ([kind, source]) => {
      const rows = new Function(`return (${source})()`)();
      const { Engine, createLocalProvider, createWorkerProvider } = window.__lab;
      const schema = JSON.stringify(window.__schema);
      const answers = async (provider) => {
        const out = [];
        for (const query of [
          { select: ["id", "flag", "note", "ratio", "amount", "day", "made", "day_year", "made_month"], sort: [{ field: "id" }] },
          { select: ["id", "ratio", "day_year"], sort: [{ field: "ratio", direction: "desc" }, { field: "id" }], limit: 40 },
        ]) {
          const bytes = await provider.execute(JSON.stringify({ source: "t", ...query }));
          // The whole answer, as a length and a hash of every byte.
          const view = new Uint8Array(bytes);
          let hash = 2166136261;
          for (let i = 0; i < view.length; i += 1) {
            hash = Math.imul(hash ^ view[i], 16777619) >>> 0;
          }
          out.push(`${view.length}:${hash}`);
        }
        const stats = await provider.stats();
        out.push(JSON.stringify(stats.sources?.find((s) => s.name === "t")));
        return out;
      };
      const run = async (route) => {
        const sent = [];
        let provider;
        if (kind === "worker") {
          provider = createWorkerProvider();
          await provider.stats(); // starts the worker, so that its messages can be watched
          const post = provider.worker.postMessage.bind(provider.worker);
          provider.worker.postMessage = (message, ...rest) => (sent.push(message.type), post(message, ...rest));
        } else {
          const engine = new Engine();
          for (const name of ["begin_columns", "load_rows"]) {
            const original = engine[name].bind(engine);
            engine[name] = (...args) => (sent.push(name), original(...args));
          }
          provider = createLocalProvider(engine);
        }
        try {
          await provider.loadRows("t", route === "json" ? JSON.stringify(rows) : rows, schema);
          return { ok: await answers(provider), sent };
        } catch (error) {
          return { error: String(error.message ?? error), sent };
        } finally {
          provider.terminate();
        }
      };
      const columns = await run("columns");
      const json = await run("json");
      // The way the rows went: over the columns, or — for what is not plain — by the JSON route.
      const via = (sent) => (sent.some((type) => /columns/.test(type)) && !sent.some((type) => /rows/.test(type)) ? "columns" : "json");
      return { columns: { ...columns, sent: undefined }, json: { ...json, sent: undefined }, via: via(columns.sent) };
    },
    [kind, makeRows.toString()],
  );

const ok = (i) => ({ id: i, flag: i % 2 === 0, note: `n${i}`, ratio: i / 4, amount: `${i}.25`, day: "2026-03-01", made: "2026-03-01T12:00:00Z" });
const ROWS = {
  plain: () => [1, 2, 3].map((i) => ({ id: i, flag: i % 2 === 0, note: `n${i}`, ratio: i / 4, amount: `${i}.25`, day: "2026-03-01", made: "2026-03-01T12:00:00Z" })),
  "nulls and missing keys": () => [{ id: 1, flag: true, note: "a" }, { id: 2 }, { id: 3, flag: null, note: null, ratio: null, amount: null, day: null, made: null }, { id: 4, ratio: 2 }],
  "text of every kind": () => [{ id: 1, note: "" }, { id: 2, note: "x".repeat(5000) }, { id: 3, note: "😀 ä 日本語" }, { id: 4, note: 'say "hi"\n\t\\' }, { id: 5, note: "\u0000" }],
  "odd numbers": () => [{ id: -0, ratio: -0 }, { id: 2, ratio: 1e300 }, { id: 3, ratio: 5e-324 }, { id: 4, ratio: 0.1 + 0.2 }, { id: 5, amount: 7 }, { id: 6, amount: -3 }],
  "dates and timestamps": () => [{ id: 1, day: "1970-01-01", made: "1969-12-31T23:59:59.999999Z" }, { id: 2, day: "2024-02-29", made: "2026-03-01T12:00:00.123456Z" }],
  "an unknown key": () => [{ id: 1 }, { id: 2, extra: 1 }],
  "a key with an undefined value": () => [{ id: 1, note: undefined }],
  "a derived column given": () => [{ id: 1, day_year: 2026 }],
  "a required column missing": () => [{ id: 1 }, { note: "x" }],
  "an integer that is not one": () => [{ id: 1.5 }],
  "an integer too big for a double": () => [{ id: 2 ** 53 }, { id: 2 ** 60 }],
  "a float that is not finite": () => [{ id: 1, ratio: NaN }, { id: 2, ratio: Infinity }],
  "a bool as a number": () => [{ id: 1, flag: 1 }],
  "a decimal as a float": () => [{ id: 1, amount: 7.5 }],
  "a decimal with too many places": () => [{ id: 1, amount: "1.005" }],
  "a date that does not exist": () => [{ id: 1, day: "2026-02-30" }],
  "a Date object": () => [{ id: 1, day: new Date(0) }],
  "text that is not well formed": () => [{ id: 1, note: "a\ud800b" }],
  "a row that is not a plain object": () => [{ id: 1 }, Object.assign(new (class Row {})(), { id: 2 })],
  "a row that is null": () => [{ id: 1 }, null],
  "a bigint": () => [{ id: 5n }],
  "a table of 150,000 rows in three packs": () =>
    Array.from({ length: 150000 }, (_, i) => ({ id: i + 1, flag: i % 3 === 0 ? null : i % 2 === 0, note: i % 7 === 0 ? null : `row ${i % 913}`, ratio: ((i * 7919) % 100003) / 7, amount: i % 5 === 0 ? null : `${(i * 31) % 10007}.${String(i % 100).padStart(2, "0")}`, day: `2026-${String((i % 12) + 1).padStart(2, "0")}-${String((i % 28) + 1).padStart(2, "0")}`, made: i % 11 === 0 ? null : `2025-${String((i % 12) + 1).padStart(2, "0")}-15T10:20:30Z` })),
  "a bad value in the second pack": () => Array.from({ length: 150000 }, (_, i) => ({ id: i === 70000 ? 1.5 : i + 1 })),
  "an unknown key in the third pack": () => Array.from({ length: 150000 }, (_, i) => (i === 140000 ? { id: i + 1, extra: 1 } : { id: i + 1 })),
};

// What goes over the columns: the rows that are plain. The rest takes the JSON route, which says
// what is wrong with them; a table that went the wrong way would be the same table, so the way is
// checked too.
const BY_COLUMNS = new Set([
  "plain",
  "nulls and missing keys",
  "text of every kind",
  "odd numbers",
  "dates and timestamps",
  "a table of 150,000 rows in three packs",
]);

for (const kind of ["local", "worker"]) {
  for (const [label, makeRows] of Object.entries(ROWS)) {
    test(`${kind}: ${label} — the table is the JSON route's`, async ({ page }) => {
      await open(page);
      await page.evaluate((schema) => (window.__schema = schema), SCHEMA);
      const { columns, json, via } = await compare(page, kind, makeRows);
      expect(columns).toEqual(json);
      expect(via).toBe(BY_COLUMNS.has(label) ? "columns" : "json");
      // A refusal is the JSON route's own sentence, with its row and field.
      if (json.error) {
        expect(json.error.length).toBeGreaterThan(0);
      }
    });
  }
}

test("plain rows go over the columns, and an engine without the calls takes the JSON route", async ({ page }) => {
  await open(page);
  const used = await page.evaluate(async (schema) => {
    const { Engine, createLocalProvider } = window.__lab;
    const rows = [{ id: 1, note: "a" }, { id: 2, note: "b" }];
    const calls = [];
    const engine = new Engine();
    for (const name of ["begin_columns", "push_columns", "end_columns", "load_rows"]) {
      const original = engine[name].bind(engine);
      engine[name] = (...args) => (calls.push(name), original(...args));
    }
    await createLocalProvider(engine).loadRows("t", rows, JSON.stringify(schema));
    const plain = [...calls];
    // An engine module from before the calls: the rows still load, by the JSON route.
    const old = new Engine();
    old.begin_columns = undefined;
    const calls2 = [];
    const original = old.load_rows.bind(old);
    old.load_rows = (...args) => (calls2.push("load_rows"), original(...args));
    await createLocalProvider(old).loadRows("t", rows, JSON.stringify(schema));
    return { plain, calls2 };
  }, SCHEMA);
  expect(used.plain).toEqual(["begin_columns", "push_columns", "end_columns"]);
  expect(used.calls2).toEqual(["load_rows"]);
});

test("two loads at once do not mix their packs", async ({ page }) => {
  await open(page);
  // The engine keeps one set of rows in the making: the packs of one load go in a row, then the
  // packs of the other. (A mix would not stay a mix — the engine refuses packs that do not fit and
  // the loads fall back to the JSON route — but it would cost the loads they are meant to save.)
  const { sequence, counts } = await page.evaluate(async (schema) => {
    const { createWorkerProvider } = window.__lab;
    const provider = createWorkerProvider();
    await provider.stats();
    const sequence = [];
    const post = provider.worker.postMessage.bind(provider.worker);
    provider.worker.postMessage = (message, ...rest) => (sequence.push(message.type), post(message, ...rest));
    const make = (offset) => Array.from({ length: 140000 }, (_, i) => ({ id: offset + i, note: `n${offset + i}` }));
    const sch = JSON.stringify(schema);
    await Promise.all([provider.loadRows("a", make(0), sch), provider.loadRows("b", make(1000000), sch)]);
    const stats = await provider.stats();
    provider.terminate();
    return { sequence, counts: stats.sources.map((s) => [s.name, s.rows]).sort() };
  }, SCHEMA);
  const load = ["columns_begin", "columns_chunk", "columns_chunk", "columns_chunk", "columns_end"];
  expect(sequence.filter((type) => type !== "stats")).toEqual([...load, ...load]);
  expect(counts).toEqual([["a", 140000], ["b", 140000]]);
});
