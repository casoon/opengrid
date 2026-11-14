// What the engine costs on its own: load a CSV, then filter, sort, group and pivot it —
// the WebAssembly module of `just wasm-build`, run in Node, so no browser and no page
// around it. Numbers, not a pass/fail; the page that quotes them says when and where.
//
//   just wasm-build && node scripts/measure-queries.mjs [--rows 100000,1000000] [--runs 9]
//
// Every query runs RUNS times after one that is thrown away; the median and the spread
// (the fastest and the slowest) are printed. The memory is what the module holds after the
// load (`stats()`), which is the number a page pays for the engine.

import { readFileSync } from "node:fs";
import os from "node:os";
import { initSync, Engine } from "../examples/engine-demo/pkg/opengrid_wasm.js";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const ROWS = option("rows", "100000,1000000").split(",").map(Number);
const RUNS = Number(option("runs", 9));

initSync({ module: readFileSync(new URL("../examples/engine-demo/pkg/opengrid_wasm_bg.wasm", import.meta.url)) });
const data = new URL("../crates/opengrid-conformance/data/", import.meta.url);
const schema = readFileSync(new URL("orders.schema.json", data), "utf8");

const countries = ["DE", "FR", "PL", "NL", "ES", "IT", "AT", "CH"];
function csv(rows) {
  const lines = ["id,customer,country,amount,qty,ratio,flag,ordered_on,created_at,note"];
  for (let i = 1; i <= rows; i++) {
    const day = String((i % 28) + 1).padStart(2, "0");
    lines.push(
      `${i},Customer ${i % 500},${countries[i % 8]},${(i % 997) + 0.25},${i % 40},` +
        `${(i % 100) / 10},${i % 2 === 0},2026-03-${day},2026-03-${day}T12:00:00Z,`,
    );
  }
  return new TextEncoder().encode(lines.join("\n"));
}

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
function time(run) {
  run();
  const samples = [];
  for (let i = 0; i < RUNS; i++) {
    const start = performance.now();
    run();
    samples.push(performance.now() - start);
  }
  return { median: median(samples), low: Math.min(...samples), high: Math.max(...samples) };
}
const ms = (t) => `${t.median.toFixed(t.median < 10 ? 1 : 0)} ms (${t.low.toFixed(t.low < 10 ? 1 : 0)}–${t.high.toFixed(t.high < 10 ? 1 : 0)})`;

console.log(`${os.cpus()[0].model}, ${os.arch()}, Node ${process.version}, median of ${RUNS} after one warm-up`);
for (const rows of ROWS) {
  const bytes = csv(rows);
  const engine = new Engine();
  const loadStart = performance.now();
  engine.load_csv("orders", bytes, schema);
  const load = performance.now() - loadStart;
  const held = JSON.parse(engine.stats());
  const memory = held.memory;
  const columns = held.sources[0]?.bytes;
  console.log(`\n${rows.toLocaleString("en")} rows, CSV ${(bytes.length / 1e6).toFixed(1)} MB`);
  console.log(`  load            ${load.toFixed(0)} ms`);
  console.log(`  module memory   ${(memory / 2 ** 20).toFixed(1)} MiB (what the page pays; it only grows, and holds the room the load needed)`);
  console.log(`  the columns     ${(columns / 2 ** 20).toFixed(1)} MiB (the data itself, typed)`);
  const run = (query) => () => engine.execute_columns(JSON.stringify({ source: "orders", ...query }));
  const rowsOut = { select: ["id", "customer", "country", "amount"], limit: 100 };
  const scenarios = {
    "filter (country = DE and amount > 500), 100 rows": run({ ...rowsOut, filter: { and: [{ field: "country", op: "eq", value: "DE" }, { field: "amount", op: "gt", value: "500" }] }, sort: [{ field: "id" }] }),
    "sort by amount, top 100": run({ ...rowsOut, sort: [{ field: "amount", direction: "desc" }, { field: "id" }] }),
    "text search (customer contains \"er 4\"), 100 rows": run({ ...rowsOut, filter: { field: "customer", op: "icontains", value: "er 4" }, sort: [{ field: "id" }] }),
    "group by country, sum and count": run({ select: ["country"], group: ["country"], aggregate: [{ field: "amount", fn: "sum", as: "total" }, { fn: "count", as: "n" }], sort: [{ field: "country" }] }),
    "group by customer (500 groups), sum": run({ select: ["customer"], group: ["customer"], aggregate: [{ field: "amount", fn: "sum", as: "total" }], sort: [{ field: "customer" }] }),
  };
  for (const [name, query] of Object.entries(scenarios)) {
    console.log(`  ${name.padEnd(52)} ${ms(time(query))}`);
  }
  const pivot = () => engine.pivot_columns(JSON.stringify({ source: "orders", rows: ["country", "customer"], columns: [], values: [{ field: "amount", fn: "sum", as: "total" }, { fn: "count", as: "n" }] }));
  console.log(`  ${"pivot (country, customer) with sum and count".padEnd(52)} ${ms(time(pivot))}`);
  engine.free();
}
