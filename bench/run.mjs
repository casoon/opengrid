// Runs every grid's page in a fresh browser page, RUNS times, and prints one table: the median
// of each figure and the range in brackets. Results are written to results/ and stay there.
//
//   pnpm bench [--grids opengrid,aggrid] [--rows 100000] [--runs 5] [--throttle 1]
//
// `--throttle 4` slows the CPU four times (Chrome's own emulation), for a mid-range laptop.
// Memory is the browser's whole process tree, resident, minus an empty page's: what the page
// costs the machine, a Worker and WebAssembly included, measured from outside at two points
// (rows loaded, and after sorting, filtering and scrolling).
// A figure that is `—` was not reached: the grid did not show the right answer within 30 s,
// and that is reported as that, not left out.

import { execSync } from "node:child_process";
import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import os from "node:os";
import { resolve } from "node:path";
import { chromium } from "@playwright/test";
import { build, preview } from "vite";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const here = import.meta.dirname;
const pagesIn = (sub) => {
  try {
    return readdirSync(resolve(here, "pages", sub)).filter((f) => f.endsWith(".html")).map((f) => `${sub ? `${sub}/` : ""}${f.replace(/\.html$/, "")}`);
  } catch {
    return [];
  }
};
const ALL = [...pagesIn(""), ...pagesIn("local")];
const GRIDS = option("grids", ALL.join(",")).split(",");
const ROWS = Number(option("rows", 100_000));
const RUNS = Number(option("runs", 5));
const THROTTLE = Number(option("throttle", 1));

await build({ root: here, logLevel: "warn" });
const server = await preview({ root: here, logLevel: "error" });
const base = `http://localhost:4173`;
const server_ = await chromium.launchServer({ args: ["--enable-precise-memory-info"] });
const browser = await chromium.connect(server_.wsEndpoint());
const rootPid = server_.process().pid;

/** What the browser holds for everything it runs: the resident memory of its whole process tree, MiB. */
function treeMiB() {
  const rows = execSync("ps -axo pid=,ppid=,rss=", { encoding: "utf8" })
    .trim()
    .split("\n")
    .map((line) => line.trim().split(/\s+/).map(Number));
  const children = new Map();
  for (const [pid, ppid] of rows) children.set(ppid, [...(children.get(ppid) ?? []), pid]);
  const rss = new Map(rows.map(([pid, , kib]) => [pid, kib]));
  let total = 0;
  const stack = [rootPid];
  while (stack.length) {
    const pid = stack.pop();
    total += rss.get(pid) ?? 0;
    stack.push(...(children.get(pid) ?? []));
  }
  return total / 1024;
}
const settle = (ms) => new Promise((done) => setTimeout(done, ms));

const median = (values) => {
  const sorted = values.filter((v) => v != null).sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};
const range = (values) => {
  const v = values.filter((x) => x != null);
  return v.length ? [Math.min(...v), Math.max(...v)] : null;
};

const results = {};
for (const grid of GRIDS) {
  results[grid] = [];
  for (let run = 0; run < RUNS; run++) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await context.newPage();
    if (THROTTLE > 1) {
      const cdp = await context.newCDPSession(page);
      await cdp.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE });
    }
    await page.goto("about:blank");
    await settle(300);
    const baseline = treeMiB();
    await page.goto(`${base}/pages/${grid}.html?rows=${ROWS}&sync=1`);
    const memory = {};
    try {
      for (;;) {
        await page.waitForFunction(() => window.__bench?.waiting || window.__bench?.done, null, { timeout: 180_000 });
        const waiting = await page.evaluate(() => window.__bench.waiting);
        if (!waiting) break;
        await settle(1000);
        memory[waiting] = treeMiB() - baseline;
        await page.evaluate(() => window.__bench.release());
      }
      const result = await page.evaluate(() => window.__bench.result);
      results[grid].push({ ...result, memory });
    } catch (error) {
      results[grid].push({ grid, error: `no result: ${error.message.split("\n")[0]}`, memory });
    }
    await context.close();
  }
}
await browser.close();
await server_.close();
await server.close();

const fmt = (value, digits = 0) => (value == null ? "—" : value.toFixed(value < 10 ? 1 : digits));
const cell = (runs, pick, digits) => {
  const values = runs.map(pick);
  const m = median(values);
  const r = range(values);
  return m == null ? "—" : r[0] === r[1] ? fmt(m, digits) : `${fmt(m, digits)} (${fmt(r[0], digits)}–${fmt(r[1], digits)})`;
};

console.log(`\n${os.cpus()[0].model}, ${os.arch()}, Chromium ${browser.version()}, ${ROWS.toLocaleString("en")} rows, CPU x${THROTTLE}, median of ${RUNS} (range)`);
const columns = [
  ["first rows ms", (r) => r.firstRows?.ok ? r.firstRows.ms : null, 0],
  ["sort ms", (r) => (r.sort?.ok ? r.sort.ms : null), 0],
  ["filter ms", (r) => (r.filter?.ok ? r.filter.ms : null), 0],
  ["undo ms", (r) => (r.clear?.ok ? r.clear.ms : null), 0],
  ["scroll p95 ms", (r) => r.scroll?.p95 ?? null, 1],
  ["scroll max ms", (r) => r.scroll?.max ?? null, 0],
  ["frames>33ms", (r) => r.scroll?.over33 ?? null, 0],
  ["MiB loaded", (r) => r.memory?.load ?? null, 0],
  ["MiB after", (r) => r.memory?.after ?? null, 0],
];
const header = ["grid", ...columns.map((c) => c[0])];
const lines = [header, ...Object.entries(results).map(([grid, runs]) => [grid, ...columns.map(([, pick, d]) => cell(runs, pick, d))])];
const widths = header.map((_, i) => Math.max(...lines.map((l) => l[i].length)));
for (const line of lines) console.log(line.map((c, i) => c.padEnd(widths[i])).join("  "));
for (const [grid, runs] of Object.entries(results)) {
  const errors = runs.filter((r) => r.error).map((r) => r.error.split("\n")[0]);
  if (errors.length) console.log(`! ${grid}: ${errors.length} run(s) failed: ${errors[0]}`);
}
mkdirSync(resolve(here, "results"), { recursive: true });
const file = resolve(here, "results", `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify({ machine: os.cpus()[0].model, rows: ROWS, throttle: THROTTLE, results }, null, 2));
console.log(`\nwritten: ${file}`);
