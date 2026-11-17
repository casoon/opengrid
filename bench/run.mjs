// Runs every grid's page in a fresh browser page, RUNS times, and prints one table: the median
// of each figure and the range in brackets. Results are written to results/ and stay there.
//
//   pnpm bench [--grids opengrid,aggrid] [--rows 100000] [--runs 5] [--throttle 1]
//              [--baseline results/<file>.json [--tolerance 20] [--fail-on-regress]]
//
// `--baseline` compares the medians with an earlier run's **from the same machine** (a figure
// from another computer says nothing about this one) and marks every figure more than
// `--tolerance` percent worse — and worse by more than a frame, 16.7 ms, 10 MiB or 10 KiB, so that
// noise on a small figure is not a finding. `--fail-on-regress` exits 1 when there is one.
//
// `--throttle 4` slows the CPU four times (Chrome's own emulation), for a mid-range laptop.
// Memory is the browser's whole process tree, resident, minus an empty page's: what the page
// costs the machine, a Worker and WebAssembly included, measured from outside at two points
// (rows loaded, and after sorting, filtering and scrolling).
// A figure that is `—` was not reached: the grid did not show the right answer within 30 s,
// and that is reported as that, not left out.

import { execSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { brotliCompressSync } from "node:zlib";
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

// A grid whose name has `server` in it asks a database through `opengrid-server`: started once
// for these rows (the first time it writes the table, which takes minutes for a million rows),
// asked on the loopback, stopped at the end. `--db postgres` uses the database `DATABASE_URL` names.
const SERVER_PORT = 8099;
let backend = null;
if (GRIDS.some((g) => g.includes("server"))) {
  const kind = option("db", "sqlite");
  backend = spawn("cargo", ["run", "--release", "-q", "-p", "opengrid-server", "--example", "bench_server", "--", kind, String(ROWS), String(SERVER_PORT)], {
    cwd: resolve(here, ".."),
    stdio: ["ignore", "pipe", "inherit"],
  });
  await new Promise((ready, fail) => {
    backend.stdout.on("data", (chunk) => String(chunk).includes("listening") && ready());
    backend.on("exit", (code) => fail(new Error(`the server stopped (${code}) before it listened`)));
  });
  backend.removeAllListeners("exit");
  console.log(`server: opengrid-server on ${kind}, ${ROWS.toLocaleString("en")} rows, port ${SERVER_PORT}`);
}
const backendMiB = () => (backend ? Number(execSync(`ps -o rss= -p ${backend.pid}`, { encoding: "utf8" }).trim()) / 1024 : null);

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

/**
 * What a page costs to fetch: every file it requested, as built — raw, and brotli-compressed as a
 * server would send it — and the WebAssembly among them (what the browser then compiles). From
 * the files, not from the wire, so that a server that compresses or not changes nothing.
 */
function sizeOf(paths) {
  const total = { files: 0, raw: 0, brotli: 0, wasmRaw: 0, wasmBrotli: 0 };
  for (const path of paths) {
    const file = resolve(here, "dist", `.${path === "/" ? "/index.html" : path}`);
    if (!existsSync(file) || !readFileSync(file).length) continue;
    const bytes = readFileSync(file);
    const packed = brotliCompressSync(bytes).length;
    total.files++;
    total.raw += bytes.length;
    total.brotli += packed;
    if (path.endsWith(".wasm")) {
      total.wasmRaw += bytes.length;
      total.wasmBrotli += packed;
    }
  }
  return total;
}

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
    // What the page asked for, to size afterwards from the built files (see `sizeOf`).
    const requested = new Set();
    page.on("response", (response) => {
      if (response.url().startsWith(base)) requested.add(new URL(response.url()).pathname);
    });
    // The requests to the server, counted from outside: how many and how many bytes of answer,
    // between the marks the page sets (`mark()` in harness.js).
    const asked = { requests: 0, bytes: 0 };
    const marks = {};
    if (backend) {
      page.on("requestfinished", async (request) => {
        if (!request.url().startsWith(`http://127.0.0.1:${SERVER_PORT}`)) return;
        asked.requests++;
        asked.bytes += (await request.sizes()).responseBodySize;
      });
      await page.exposeFunction("__mark", async (name) => {
        await settle(100);
        marks[name] = { ...asked };
      });
    }
    await page.goto("about:blank");
    await settle(300);
    const baseline = treeMiB();
    await page.goto(`${base}/pages/${grid}.html?rows=${ROWS}&sync=1&server=${SERVER_PORT}`);
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
      const between = (from, to) => (marks[from] && marks[to] ? { requests: marks[to].requests - marks[from].requests, bytes: marks[to].bytes - marks[from].bytes } : null);
      results[grid].push({ ...result, memory, bytes: sizeOf(requested), server: backend && grid.includes("server") ? { windows: between("windows:start", "windows:end"), scroll: between("scroll:start", "scroll:end"), asked, backendMiB: backendMiB() } : null });
    } catch (error) {
      results[grid].push({ grid, error: `no result: ${error.message.split("\n")[0]}`, memory });
    }
    await context.close();
  }
}
await browser.close();
backend?.kill();
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
  ["group ms", (r) => (r.group?.ok ? r.group.ms : null), 0],
  ["ungroup ms", (r) => (r.ungroup?.ok ? r.ungroup.ms : null), 0],
  ["step p50 ms", (r) => r.windows?.steps?.p50 ?? null, 0],
  ["step p95 ms", (r) => r.windows?.steps?.p95 ?? null, 0],
  ["jump p50 ms", (r) => r.windows?.jumps?.p50 ?? null, 0],
  ["jump p95 ms", (r) => r.windows?.jumps?.p95 ?? null, 0],
  ["middle ms", (r) => r.windows?.middle ?? null, 0],
  ["requests", (r) => r.server?.windows?.requests ?? null, 0],
  ["KiB answered", (r) => (r.server?.windows ? r.server.windows.bytes / 1024 : null), 0],
  ["server MiB", (r) => r.server?.backendMiB ?? null, 0],
  ["KiB fetched (br)", (r) => (r.bytes ? r.bytes.brotli / 1024 : null), 0],
  ["KiB wasm (br)", (r) => (r.bytes ? r.bytes.wasmBrotli / 1024 : null), 0],
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

const baselineFile = option("baseline", null);
if (baselineFile) {
  const earlier = JSON.parse(readFileSync(resolve(baselineFile), "utf8"));
  const tolerance = Number(option("tolerance", 20)) / 100;
  if (earlier.rows !== ROWS || earlier.throttle !== THROTTLE || earlier.machine !== os.cpus()[0].model) {
    console.log(`\n! the baseline was measured with ${earlier.rows} rows, CPU x${earlier.throttle}, on ${earlier.machine}: not comparable`);
    process.exit(2);
  }
  // Worse is more, for every figure here. The floor is what is too small to tell from noise.
  const floors = { ms: 16.7, mib: 10, kib: 10 };
  const compared = columns.map(([name, pick]) => [name, pick, name.startsWith("MiB") ? floors.mib : name.startsWith("KiB") ? floors.kib : name === "frames>33ms" ? 1 : floors.ms]);
  let regressions = 0;
  console.log(`\nagainst ${baselineFile} (marked: more than ${tolerance * 100} % worse and past the floor)`);
  for (const [grid, runs] of Object.entries(results)) {
    const before = earlier.results[grid];
    if (!before) continue;
    const parts = compared.map(([name, pick, floor]) => {
      const now = median(runs.map(pick));
      const then = median(before.map(pick));
      if (now == null || then == null) return `${name} ${now == null && then != null ? "LOST" : "—"}`;
      const worse = now - then > floor && now > then * (1 + tolerance);
      if (worse) regressions++;
      return `${name} ${fmt(then, 0)}→${fmt(now, 0)}${worse ? " !" : ""}`;
    });
    console.log(`${grid}: ${parts.join(" · ")}`);
  }
  console.log(regressions ? `${regressions} figure(s) worse` : "nothing worse");
  if (regressions && args.includes("--fail-on-regress")) process.exit(1);
}
