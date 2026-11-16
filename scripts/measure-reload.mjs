// What a page's memory does when the engine in a worker loads a million rows
// again and again (issue #459). Serves the repository, opens
// tests/bench/reload.html in Chromium and runs one scenario per fresh browser:
//
//   same-bytes    LOADS loads under one name with `load` (the CSV's bytes)
//   same-stream   LOADS loads under one name with `loadStream`
//   names-stream  LOADS loads under LOADS names with `loadStream`
//   restart       LOADS loads as same-stream, then `terminate()` and a new
//                 provider: the memory handed back, and the time to the first
//                 answer (start, load, query) against a load into a warm worker
//
// After every load: the engine's linear memory (`stats().memory`) and the
// memory of Chromium's whole process tree (its physical footprint on macOS),
// minus what it held after the page had made its CSV and before the engine
// started. Numbers, not a pass/fail; the performance guide quotes them.
//
//   just wasm-build packages/opengrid/engine && just wasm-build-components
//   node scripts/measure-reload.mjs [--rows 1000000] [--loads 10] [--scenarios same-bytes,…]

import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { chromium } from "@playwright/test";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const ROWS = Number(option("rows", 1_000_000));
const LOADS = Number(option("loads", 10));
const SCENARIOS = option("scenarios", "same-bytes,same-stream,names-stream,restart").split(",");
const PORT = Number(option("port", 8131));
const URL = `http://127.0.0.1:${PORT}/tests/bench/reload.html`;

// The CSV, written once into target/ (not in git): the page only fetches it.
const FILE = `target/reload-orders-${ROWS}.csv`;
if (!existsSync(FILE)) {
  const countries = ["DE", "FR", "PL", "NL", "ES", "IT", "AT", "CH"];
  const lines = ["id,customer,country,amount,qty,ratio,flag,ordered_on,created_at,note"];
  for (let i = 1; i <= ROWS; i++) {
    const day = String((i % 28) + 1).padStart(2, "0");
    lines.push(
      `${i},Customer ${i % 500},${countries[i % 8]},${(i % 997) + 0.25},${i % 40},` +
        `${(i % 100) / 10},${i % 2 === 0},2026-03-${day},2026-03-${day}T12:00:00Z,`,
    );
  }
  mkdirSync("target", { recursive: true });
  writeFileSync(FILE, lines.join("\n"));
}

const server = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1"], {
  stdio: "ignore",
});
await new Promise((done) => setTimeout(done, 800));

const MiB = (bytes) => (bytes / 1024 / 1024).toFixed(0);
const wait = (ms) => new Promise((done) => setTimeout(done, ms));

/**
 * The memory of a process and all its descendants, in bytes. On macOS the
 * physical footprint (`footprint`, what Activity Monitor shows): under memory
 * pressure macOS compresses pages, and the resident size then drops although
 * nothing was freed. Elsewhere the resident size (`ps`).
 */
function treeMemory(root) {
  const rows = execFileSync("ps", ["-A", "-o", "pid=,ppid=,rss="], { encoding: "utf8" })
    .trim()
    .split("\n")
    .map((line) => line.trim().split(/\s+/).map(Number));
  const children = new Map();
  for (const [pid, ppid] of rows) children.set(ppid, [...(children.get(ppid) ?? []), pid]);
  const tree = [];
  const stack = [root];
  while (stack.length) {
    const pid = stack.pop();
    tree.push(pid);
    stack.push(...(children.get(pid) ?? []));
  }
  if (process.platform === "darwin") {
    const out = execFileSync("footprint", ["--format", "bytes", ...tree.flatMap((pid) => ["-p", String(pid)])], {
      encoding: "utf8",
    });
    return [...out.matchAll(/phys_footprint: (\d+) B/g)].reduce((sum, [, bytes]) => sum + Number(bytes), 0);
  }
  const rss = new Map(rows.map(([pid, , kib]) => [pid, kib * 1024]));
  return tree.reduce((sum, pid) => sum + (rss.get(pid) ?? 0), 0);
}

async function engineMemory(page) {
  const stats = await page.evaluate(() => window.bench.stats());
  return (typeof stats === "string" ? JSON.parse(stats) : stats).memory;
}

/** After a forced collection on the page and a moment for the browser to settle. */
async function reading(page, pid) {
  for (let i = 0; i < 3; i++) {
    await page.evaluate(() => globalThis.gc?.());
    await wait(1000);
  }
  return treeMemory(pid);
}

async function run(scenario) {
  const browserServer = await chromium.launchServer({ args: ["--js-flags=--expose-gc"] });
  const pid = browserServer.process().pid;
  const browser = await chromium.connect(browserServer.wsEndpoint());
  const page = await browser.newPage();
  page.on("pageerror", (error) => console.error(`page: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") console.error(`page: ${message.text()}`);
  });
  await page.goto(URL);
  await page.waitForFunction(() => document.title === "ready");
  await page.evaluate((file) => window.bench.prepare(file), `/${FILE}`);
  const base = await reading(page, pid);
  await page.evaluate(() => window.bench.start());

  const how = scenario === "same-bytes" ? "bytes" : "stream";
  console.log(`\n${scenario}: ${ROWS.toLocaleString("en")} rows, ${LOADS} loads (MiB; time per load)`);
  console.log("| Load | Engine | Browser, minus the page | Load time |");
  console.log("|---|---|---|---|");
  for (let i = 1; i <= LOADS; i++) {
    const name = scenario === "names-stream" ? `orders_${i}` : "orders";
    const time = await page.evaluate(([n, h]) => window.bench.load(n, h), [name, how]);
    const engine = await engineMemory(page);
    const rss = await reading(page, pid);
    console.log(`| ${i} | ${MiB(engine)} | ${MiB(rss - base)} | ${(time / 1000).toFixed(2)} s |`);
  }

  if (scenario === "restart") {
    const warm = await page.evaluate(async () => {
      const load = await window.bench.load("orders", "stream");
      return load + (await window.bench.query("orders"));
    });
    await page.evaluate(() => window.bench.terminate());
    const after = await reading(page, pid);
    const cold = await page.evaluate(async () => {
      const started = performance.now();
      window.bench.start();
      await window.bench.load("orders", "stream");
      await window.bench.query("orders");
      return performance.now() - started;
    });
    const engine = await engineMemory(page);
    const again = await reading(page, pid);
    console.log(`\nAfter terminate(): browser ${MiB(after - base)} MiB minus the page`);
    console.log(`A new provider, loaded and asked once: ${(cold / 1000).toFixed(2)} s (a warm worker: ${(warm / 1000).toFixed(2)} s); engine ${MiB(engine)} MiB, browser ${MiB(again - base)} MiB`);
  }
  await browser.close();
  await browserServer.close();
}

try {
  console.log(`Chromium, median-free single runs, page baseline excludes its own CSV`);
  for (const scenario of SCENARIOS) await run(scenario);
} finally {
  server.kill();
}
