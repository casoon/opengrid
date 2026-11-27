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
// `--throttle 4` slows the page's main thread four times (Chrome's own emulation). Not a Worker:
// Chromium 153 refuses the emulation there ("only supported for pages, not workers"), so
// opengrid's engine, which runs in one, keeps the machine's full speed. A throttled figure is the
// page on a slow CPU with the engine on a fast one, not a mid-range laptop (#421).
// A limit that reaches every thread is the kernel's: on Linux, run the whole tool in a cgroup with
// a CPU quota (bench.yml's `cpu` input does). The runner reads the quota it runs under from
// /proc/self/cgroup and cpu.max, records it, and compares only with a run under the same one.
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
const PROBE = args.includes("--probe");
const RELEASE = args.includes("--release");
const GCTRACE = args.includes("--gctrace");

/** EXPERIMENT (#518): the garbage collector's trace events of the steps, by thread and name. */
async function gcSummary(bcdp, stream) {
  let text = "";
  for (;;) {
    const part = await bcdp.send("IO.read", { handle: stream, size: 1 << 20 });
    text += part.data;
    if (part.eof) break;
  }
  await bcdp.send("IO.close", { handle: stream });
  const events = JSON.parse(text).traceEvents ?? [];
  const threads = new Map();
  for (const e of events) if (e.ph === "M" && e.name === "thread_name") threads.set(`${e.pid}:${e.tid}`, e.args.name);
  const open = new Map();
  const sums = {};
  const big = [];
  const add = (e, dur) => {
    if (!/gc|scaven|compact|mark|sweep|MinorGC|MajorGC/i.test(e.name)) return;
    const key = `${threads.get(`${e.pid}:${e.tid}`) ?? e.tid} ${e.name}`;
    const t = (sums[key] ??= { n: 0, ms: 0, max: 0 });
    t.n++;
    t.ms += dur / 1000;
    t.max = Math.max(t.max, dur / 1000);
    if (dur > 8000 && /Main/.test(threads.get(`${e.pid}:${e.tid}`) ?? "")) big.push(`${e.name}@${Math.round((e.ts - first) / 1000)}:${(dur / 1000).toFixed(0)}`);
  };
  const first = Math.min(...events.filter((e) => e.ph !== "M").map((e) => e.ts));
  for (const e of events) {
    if (e.ph === "X" && e.dur != null) add(e, e.dur);
    else if (e.ph === "B") open.set(`${e.pid}:${e.tid}:${e.name}`, e);
    else if (e.ph === "E") {
      const b = open.get(`${e.pid}:${e.tid}:${e.name}`);
      if (b) add(b, e.ts - b.ts);
    }
  }
  const top = Object.entries(sums)
    .filter(([, v]) => v.ms >= 3)
    .sort((a, b) => b[1].ms - a[1].ms)
    .slice(0, 14)
    .map(([k, v]) => `${k} n=${v.n} ${v.ms.toFixed(0)}ms max=${v.max.toFixed(0)}`);
  return { top, big: big.slice(0, 30) };
}

/** EXPERIMENT (#518): CPU ticks of every thread of the browser's process tree, by thread name. */
function threadTicks(rootPid) {
  const out = {};
  try {
    const rows = execSync("ps -axo pid=,ppid=", { encoding: "utf8" }).trim().split("\n").map((l) => l.trim().split(/\s+/).map(Number));
    const kids = new Map();
    for (const [pid, ppid] of rows) kids.set(ppid, [...(kids.get(ppid) ?? []), pid]);
    const stack = [rootPid];
    while (stack.length) {
      const pid = stack.pop();
      stack.push(...(kids.get(pid) ?? []));
      let role = "";
      try {
        role = readFileSync(`/proc/${pid}/cmdline`, "utf8").match(/--type=([a-z-]+)/)?.[1] ?? "browser";
      } catch {
        continue;
      }
      for (const tid of readdirSync(`/proc/${pid}/task`)) {
        try {
          const stat = readFileSync(`/proc/${pid}/task/${tid}/stat`, "utf8");
          const comm = stat.slice(stat.indexOf("(") + 1, stat.lastIndexOf(")"));
          const f = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
          const key = `${role}/${pid}/${comm}${role === "renderer" && comm.startsWith("Dedicated") ? `#${tid}` : ""}`;
          out[`${key}`] = (out[key] ?? 0) + Number(f[11]) + Number(f[12]);
        } catch {
          // a thread that ended
        }
      }
    }
  } catch {
    // not Linux
  }
  try {
    const path = readFileSync("/proc/self/cgroup", "utf8").match(/^0::(.*)$/m)?.[1];
    const stat = readFileSync(`/sys/fs/cgroup${path}/cpu.stat`, "utf8");
    for (const line of stat.trim().split("\n")) {
      const [k, v] = line.split(" ");
      if (["nr_periods", "nr_throttled", "throttled_usec"].includes(k)) out[`cgroup ${k}`] = Number(v);
    }
  } catch {
    // no cgroup v2
  }
  return out;
}

/** The CPU time this process tree may use, in cores (0.25 is a quarter of one), from the cgroup
 * it runs in; null when there is no quota or no cgroup v2 (macOS). Read, not taken from an option,
 * so that a result says what the kernel enforced. */
function cpuLimit() {
  try {
    const path = readFileSync("/proc/self/cgroup", "utf8").match(/^0::(.*)$/m)?.[1];
    const [quota, period] = readFileSync(`/sys/fs/cgroup${path}/cpu.max`, "utf8").trim().split(" ");
    return quota === "max" ? null : Number(quota) / Number(period);
  } catch {
    return null;
  }
}
const CPU_LIMIT = cpuLimit();

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
    const cdp = THROTTLE > 1 || PROBE ? await context.newCDPSession(page) : null;
    if (THROTTLE > 1) {
      await cdp.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE });
    }
    // EXPERIMENT (#518): the page's own counters (layouts, style recalcs, script time) around the steps.
    const metrics = {};
    let bcdp = null;
    if (PROBE && GCTRACE) bcdp = await browser.newBrowserCDPSession();
    if (PROBE) {
      await cdp.send("Performance.enable");
      await page.exposeFunction("__metrics", async (name) => {
        if (bcdp && name === "steps:start") {
          await bcdp.send("Tracing.start", { traceConfig: { includedCategories: ["v8", "v8.gc", "blink_gc", "disabled-by-default-v8.gc"], recordMode: "recordAsMuchAsPossible" }, transferMode: "ReturnAsStream" });
        }
        if (bcdp && name === "steps:end") {
          const done = new Promise((resolve) => bcdp.once("Tracing.tracingComplete", resolve));
          await bcdp.send("Tracing.end");
          const { stream } = await done;
          try {
            metrics.gc = await gcSummary(bcdp, stream);
          } catch (error) {
            metrics.gc = { error: String(error) };
          }
        }
        const { metrics: list } = await cdp.send("Performance.getMetrics");
        metrics[name] = Object.fromEntries(list.map((m) => [m.name, m.value]));
        metrics[`${name}:threads`] = threadTicks(rootPid);
      });
      await page.exposeFunction("__gc", async () => {
        await cdp.send("HeapProfiler.enable");
        await cdp.send("HeapProfiler.collectGarbage");
      });
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
    await page.goto(`${base}/pages/${grid}.html?rows=${ROWS}&sync=1&server=${SERVER_PORT}${PROBE ? "&probe=1" : ""}${args.includes("--mid") ? "&mid=1" : ""}${args.includes("--micro3") ? "&micro3=1" : ""}${args.includes("--reuse") ? "&reuse=1" : ""}${args.includes("--micro") ? "&micro=1" : ""}${RELEASE ? "&release=1" : ""}`);
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
      results[grid].push({ ...result, metrics, memory, bytes: sizeOf(requested), server: backend && grid.includes("server") ? { windows: between("windows:start", "windows:end"), scroll: between("scroll:start", "scroll:end"), asked, backendMiB: backendMiB() } : null });
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
if (THROTTLE > 1) console.log(`! x${THROTTLE} slows the main thread only; a Worker (opengrid's engine) runs at full speed`);
if (CPU_LIMIT != null) console.log(`cgroup CPU quota: ${CPU_LIMIT} core(s) for the whole tool, browser and Worker included`);
const columns = [
  ["first rows ms", (r) => r.firstRows?.ok ? r.firstRows.ms : null, 0],
  ["sort ms", (r) => (r.sort?.ok ? r.sort.ms : null), 0],
  ["filter ms", (r) => (r.filter?.ok ? r.filter.ms : null), 0],
  ["undo ms", (r) => (r.clear?.ok ? r.clear.ms : null), 0],
  ["scroll p95 ms", (r) => r.scroll?.p95 ?? null, 1],
  ["scroll max ms", (r) => r.scroll?.max ?? null, 0],
  ["frames>33ms", (r) => r.scroll?.over33 ?? null, 0],
  ["jumps>33ms", (r) => r.scroll?.jumpsOver33 ?? null, 0],
  ["LoAF forced ms", (r) => (r.scroll?.loaf ? r.scroll.loaf.steps.forced + r.scroll.loaf.jumps.forced : null), 0],
  ["LoAF render ms", (r) => (r.scroll?.loaf ? r.scroll.loaf.steps.render + r.scroll.loaf.jumps.render : null), 0],
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
if (PROBE) {
  const pick = ["LayoutCount", "RecalcStyleCount", "LayoutDuration", "RecalcStyleDuration", "ScriptDuration", "TaskDuration", "JSHeapUsedSize"];
  for (const [grid, runs] of Object.entries(results)) {
    runs.forEach((r, i) => {
      const a = r.metrics?.["steps:start"];
      const b = r.metrics?.["steps:end"];
      const d = a && b ? Object.fromEntries(pick.map((k) => [k, +(k.endsWith("Size") ? (b[k] - a[k]) / 1048576 : k.endsWith("Duration") ? (b[k] - a[k]) * 1000 : b[k] - a[k]).toFixed(1)])) : null;
      const grown = ["before-windows", "after-windows", "steps:start", "steps:end"].map((k) => `${k} ${r.metrics?.[k]?.Nodes}/${r.metrics?.[k]?.LayoutObjects}`).join(" · ");
      console.log(`PROBE ${grid} run ${i} nodes/layoutObjects: ${grown}`);
      if (r.micro3) console.log(`PROBE ${grid} run ${i} micro3 ${JSON.stringify(r.micro3)}`);
      const e = r.metrics?.["steps:end"];
      console.log(`PROBE ${grid} run ${i} micro ${JSON.stringify(r.scroll?.micro)} end: ${JSON.stringify(e && Object.fromEntries(["Nodes", "LayoutObjects", "JSEventListeners", "JSHeapTotalSize", "JSHeapUsedSize", "Documents", "Frames"].map((k) => [k, k.startsWith("JSHeap") ? +(e[k] / 1048576).toFixed(0) : e[k]])))}`);
      console.log(`PROBE ${grid} run ${i} steps(120): metrics ${JSON.stringify(d)} (durations ms, heap MiB)`);
      const ta = r.metrics?.["steps:start:threads"] ?? {};
      const tb = r.metrics?.["steps:end:threads"] ?? {};
      const cpu = Object.entries(tb)
        .map(([k, v]) => [k, k.startsWith("cgroup") ? v - (ta[k] ?? 0) : (v - (ta[k] ?? 0)) * 10])
        .filter(([, v]) => v > 0)
        .sort((x, y) => y[1] - x[1])
        .slice(0, 12);
      console.log(`PROBE ${grid} run ${i} threads (cpu ms in the steps; cgroup: periods, throttled periods, throttled us): ${JSON.stringify(cpu)}`);
      if (r.metrics?.gc) console.log(`PROBE ${grid} run ${i} gc: ${JSON.stringify(r.metrics.gc)}`);
      console.log(`PROBE ${grid} run ${i} longAt ${JSON.stringify(r.scroll?.probe?.longAt)} over33At ${JSON.stringify(r.scroll?.probe?.over33At)}`);
      console.log(`PROBE ${grid} run ${i} ${JSON.stringify(r.scroll?.probe)}`);
    });
  }
}
mkdirSync(resolve(here, "results"), { recursive: true });
const file = resolve(here, "results", `${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
writeFileSync(file, JSON.stringify({ machine: os.cpus()[0].model, rows: ROWS, throttle: THROTTLE, cpuLimit: CPU_LIMIT, results }, null, 2));
console.log(`\nwritten: ${file}`);

const baselineFile = option("baseline", null);
if (baselineFile) {
  const earlier = JSON.parse(readFileSync(resolve(baselineFile), "utf8"));
  const tolerance = Number(option("tolerance", 20)) / 100;
  if (earlier.rows !== ROWS || earlier.throttle !== THROTTLE || (earlier.cpuLimit ?? null) !== CPU_LIMIT || earlier.machine !== os.cpus()[0].model) {
    console.log(`\n! the baseline was measured with ${earlier.rows} rows, CPU x${earlier.throttle}, quota ${earlier.cpuLimit ?? "none"}, on ${earlier.machine}: not comparable`);
    process.exit(2);
  }
  // Worse is more, for every figure here. The floor is what is too small to tell from noise.
  const floors = { ms: 16.7, mib: 10, kib: 10 };
  const compared = columns.map(([name, pick]) => [name, pick, name.startsWith("MiB") ? floors.mib : name.startsWith("KiB") ? floors.kib : name.includes(">33ms") ? 1 : floors.ms]);
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
