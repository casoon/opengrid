// What every page runs: mount a grid on the same rows, then measure the same things, in the
// same order, in the same way. A grid is an adapter (`src/adapters/<name>.js`); everything
// here knows nothing about any of them.
//
// Adapter: { name, async mount(container, rows, columns), firstRow(), visibleCountries(),
//            async sort(field, direction), async filter(field, value), async clearFilter(),
//            optional: async group(field, { amount: "sum" }), groups() -> [{ key, count, sum }],
//            async ungroup() — a grid without them is not measured for grouping, and says "—",
//            scroller(), rowHeight(), destroy() } — `firstRow()` reads what is **on screen**
//            ({ id }), because a grid that has the answer and not yet the pixels has not answered.

import { COLUMNS, expected, expectedGroups, makeRows } from "./data.js";

const params = new URLSearchParams(location.search);
const ROWS = Number(params.get("rows") ?? 100_000);
const HEIGHT = 600;
const LIMIT_MS = 30_000;

const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));

/**
 * Starts the clock, runs `trigger`, then looks at every frame until `settled()`. The time ends
 * one frame after that, when the frame that showed the result has been painted: a grid that has
 * the answer and not yet the pixels has not answered. The same floor for every grid.
 */
async function timed(trigger, settled) {
  const start = performance.now();
  const triggered = Promise.resolve(trigger());
  for (;;) {
    await frame();
    if (settled()) {
      await frame();
      const end = performance.now();
      await triggered;
      return { ms: end - start, ok: true };
    }
    if (performance.now() - start > LIMIT_MS) {
      return { ms: null, ok: false };
    }
  }
}

/** Every country, with its row count and its sum (to the cent: float sums differ in the last bits). */
function groupsAreRight(shown, want) {
  return (
    shown.length === want.size &&
    shown.every((g) => {
      const w = want.get(g.key);
      return w && g.count === w.count && Math.abs(g.sum - w.sum) < 0.01 + w.sum * 1e-9;
    })
  );
}

/** Tells the runner a stage begins or ends, so it can say what the requests in between were. */
const mark = (name) => (window.__mark ? window.__mark(name) : undefined);

/**
 * What a reader waits for while scrolling a table whose rows are somewhere else: after each move
 * of the scroll bar, the time until the row at the top is painted **with its data**. Three kinds:
 * 120 steps of three rows (reading on), 20 jumps of 2,000 rows, and one drag to the middle.
 * The same move for every adapter that can say which row is painted (`painted`).
 */
async function windows(adapter, rows) {
  const scroller = adapter.scroller();
  const rowHeight = adapter.rowHeight();
  const max = scroller.scrollHeight - scroller.clientHeight;
  // The time from moving the scroll bar to a viewport whose rows all show their data and whose
  // first row is not the one it was: with scaled scrolling the index a position stands for is the
  // grid's to say, so what is waited for is a painted answer that is somewhere else.
  const to = async (top) => {
    const target = Math.min(max, top);
    const before = adapter.painted()?.first;
    const start = performance.now();
    scroller.scrollTop = target;
    for (;;) {
      await frame();
      const now = adapter.painted();
      if (now?.complete && (now.first !== before || target === scroller.scrollTop && target === 0)) return performance.now() - start;
      if (performance.now() - start > 10_000) return null;
    }
  };
  scroller.scrollTop = 0;
  await frame();
  await mark("windows:start");
  const steps = [];
  for (let i = 1; i <= 120; i++) steps.push(await to(i * 3 * rowHeight));
  const jumps = [];
  for (let i = 1; i <= 20; i++) jumps.push(await to(i * 2000 * rowHeight));
  const middle = await to((rows / 2) * rowHeight);
  await mark("windows:end");
  const summary = (times) => {
    const done = times.filter((t) => t != null).sort((a, b) => a - b);
    return { n: times.length, failed: times.length - done.length, p50: percentile(done, 0.5), p95: percentile(done, 0.95), max: done[done.length - 1] };
  };
  scroller.scrollTop = 0;
  await frame();
  return { steps: summary(steps), jumps: summary(jumps), middle };
}

function percentile(sorted, p) {
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))];
}
function summarise(frames, longTasks) {
  const sorted = [...frames].sort((a, b) => a - b);
  return {
    frames: frames.length,
    p50: percentile(sorted, 0.5),
    p95: percentile(sorted, 0.95),
    max: sorted[sorted.length - 1],
    over33: frames.filter((f) => f > 33.4).length,
    longTasks: longTasks.length,
    longTaskMs: longTasks.reduce((sum, d) => sum + d, 0),
  };
}


/**
 * EXPERIMENT (#518), `?probe=1`: the calls that make the browser lay out or recalculate style
 * from script, each with its time and when it ran, to put next to the Long Animation Frames.
 */
function installProbe() {
  const calls = [];
  const wrapGetter = (proto, name) => {
    const d = Object.getOwnPropertyDescriptor(proto, name);
    if (!d?.get) return;
    Object.defineProperty(proto, name, {
      ...d,
      get() {
        const t = performance.now();
        const v = d.get.call(this);
        calls.push([`get ${name}`, t, performance.now() - t]);
        return v;
      },
    });
  };
  const wrapSetter = (proto, name) => {
    const d = Object.getOwnPropertyDescriptor(proto, name);
    if (!d?.set) return;
    Object.defineProperty(proto, name, {
      ...d,
      set(value) {
        const t = performance.now();
        d.set.call(this, value);
        calls.push([`set ${name}`, t, performance.now() - t]);
      },
    });
  };
  const wrapMethod = (proto, name) => {
    const original = proto[name];
    proto[name] = function (...args) {
      const t = performance.now();
      const v = original.apply(this, args);
      calls.push([name, t, performance.now() - t]);
      return v;
    };
  };
  for (const name of ["scrollLeft", "scrollTop", "scrollWidth", "scrollHeight", "clientWidth", "clientHeight", "clientLeft", "clientTop"]) wrapGetter(Element.prototype, name);
  for (const name of ["offsetLeft", "offsetTop", "offsetWidth", "offsetHeight", "innerText"]) wrapGetter(HTMLElement.prototype, name);
  for (const name of ["scrollLeft", "scrollTop"]) wrapSetter(Element.prototype, name);
  for (const name of ["getBoundingClientRect", "getClientRects", "scrollIntoView"]) wrapMethod(Element.prototype, name);
  wrapMethod(HTMLElement.prototype, "focus");
  wrapMethod(CSSStyleDeclaration.prototype, "getPropertyValue");
  return calls;
}

function probeReport(entries, calls, frames) {
  const totals = {};
  for (const [name, , ms] of calls) {
    const t = (totals[name] ??= { n: 0, ms: 0, max: 0 });
    t.n++;
    t.ms += ms;
    t.max = Math.max(t.max, ms);
  }
  const invokers = {};
  for (const e of entries) {
    for (const s of e.scripts) {
      const t = (invokers[`${s.invokerType} ${s.invoker}`] ??= { n: 0, ms: 0, forced: 0, pause: 0 });
      t.n++;
      t.ms += s.duration;
      t.forced += s.forcedStyleAndLayoutDuration;
      t.pause += s.pauseDuration;
    }
  }
  const worst = [...entries]
    .sort((a, b) => b.duration - a.duration)
    .slice(0, 6)
    .map((e) => ({
      at: Math.round(e.startTime),
      ms: Math.round(e.duration),
      renderStart: Math.round(e.renderStart - e.startTime),
      styleLayoutStart: Math.round(e.styleAndLayoutStart - e.startTime),
      blocking: Math.round(e.blockingDuration),
      scripts: e.scripts.map((s) => ({ type: s.invokerType, invoker: s.invoker, from: Math.round(s.startTime - e.startTime), ms: Math.round(s.duration), forced: Math.round(s.forcedStyleAndLayoutDuration), fn: s.sourceFunctionName })),
      calls: calls.filter(([, t, ms]) => ms > 1 && t >= e.startTime && t <= e.startTime + e.duration).map(([name, t, ms]) => `${name}@${Math.round(t - e.startTime)}:${ms.toFixed(1)}`),
    }));
  const sorted = [...frames].sort((a, b) => a - b);
  return { totals, invokers, worst, over33: frames.filter((f) => f > 33.4).length, p50: sorted[60], p95: sorted[114] };
}


/**
 * EXPERIMENT (#518): what a forced style recalc costs in this page's state: the grid's rows given
 * a new `transform` each, a tree of the same size that has nothing to do with the grid, and
 * nothing at all. Median of 40, ms.
 */
export function microbench(scroller) {
  const time = (setup, force) => {
    const t = [];
    for (let i = 0; i < 40; i++) {
      setup(i);
      const s = performance.now();
      force();
      t.push(performance.now() - s);
    }
    t.sort((a, b) => a - b);
    return +t[20].toFixed(2);
  };
  const rows = [...scroller.querySelectorAll("tbody tr")];
  const saved = rows.map((r) => r.style.cssText);
  const move = (i) => rows.forEach((r, k) => (r.style.transform = `translateY(${k * 42 + (i % 2) * 0.5}px)`));
  const style = time(move, () => getComputedStyle(rows[0]).color);
  scroller.clientHeight;
  const grid = time(move, () => scroller.clientHeight);
  rows.forEach((r, k) => (r.style.cssText = saved[k]));
  scroller.clientHeight;
  const box = document.createElement("div");
  box.style.cssText = "position:fixed;left:0;top:0;width:300px;height:200px;overflow:hidden";
  for (let k = 0; k < 40; k++) {
    const r = document.createElement("div");
    r.style.cssText = "position:absolute";
    for (let c = 0; c < 6; c++) {
      const d = document.createElement("span");
      d.textContent = "x";
      r.append(d);
    }
    box.append(r);
  }
  document.body.append(box);
  const kids = [...box.children];
  const plain = time((i) => kids.forEach((r, k) => (r.style.transform = `translateY(${k * 42 + (i % 2) * 0.5}px)`)), () => box.clientHeight);
  box.remove();
  const idle = time(() => {}, () => scroller.clientHeight);
  return { grid, style, plain, idle, rows: rows.length };
}

async function scroll(adapter) {
  const scroller = adapter.scroller();
  const rowHeight = adapter.rowHeight();
  const longTasks = [];
  const observer = new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) longTasks.push(entry.duration);
  });
  // Long Animation Frames (#518): a frame over 50 ms with what its time went to — script,
  // style and layout forced from script, and the rendering after it. Chromium only; cheap,
  // unlike a trace, whose invalidation tracking inflates the very recalcs it records.
  const loafs = [];
  const frameObserver = new PerformanceObserver((list) => loafs.push(...list.getEntries()));
  try {
    observer.observe({ entryTypes: ["longtask"] });
    frameObserver.observe({ type: "long-animation-frame" });
  } catch {
    // Not every browser reports them.
  }
  const probeCalls = params.has("probe") ? installProbe() : null;
  const micro = params.has("micro") ? microbench(scroller) : null;
  scroller.scrollTop = 0;
  await frame();
  const frames = [];
  await window.__metrics?.("steps:start");
  let last = await frame();
  const fineStart = last;
  if (probeCalls) probeCalls.length = 0;
  for (let i = 0; i < 120; i++) {
    scroller.scrollTop += 3 * rowHeight;
    const now = await frame();
    frames.push(now - last);
    last = now;
  }
  const jumpStart = last;
  const probeSteps = probeCalls ? probeCalls.slice() : null;
  await window.__metrics?.("steps:end");
  const max = scroller.scrollHeight - scroller.clientHeight;
  for (let i = 0; i < 20; i++) {
    scroller.scrollTop = Math.min(max, (i + 1) * 2000 * rowHeight);
    const now = await frame();
    frames.push(now - last);
    last = now;
  }
  await frame();
  observer.disconnect();
  frameObserver.disconnect();
  const moved = scroller.scrollTop > 0;
  scroller.scrollTop = 0;
  await frame();
  return {
    ...summarise(frames, longTasks),
    // The 20 jumps (a drag of the thumb) apart from the 120 wheel steps.
    jumpsOver33: frames.slice(120).filter((f) => f > 33.4).length,
    loaf: { steps: longFrames(loafs, fineStart, jumpStart), jumps: longFrames(loafs, jumpStart, last) },
    micro,
    probe: probeSteps ? probeReport(loafs.filter((e) => e.startTime >= fineStart && e.startTime < jumpStart), probeSteps, frames.slice(0, 120)) : null,
    moved,
  };
}

/** The long animation frames that started in [from, to): how many, and their milliseconds. */
function longFrames(entries, from, to) {
  const inside = entries.filter((e) => e.startTime >= from && e.startTime < to);
  const sum = (pick) => inside.reduce((total, e) => total + pick(e), 0);
  return {
    count: inside.length,
    ms: sum((e) => e.duration),
    script: sum((e) => e.scripts.reduce((t, s) => t + s.duration, 0)),
    forced: sum((e) => e.scripts.reduce((t, s) => t + s.forcedStyleAndLayoutDuration, 0)),
    render: sum((e) => (e.renderStart ? e.startTime + e.duration - e.renderStart : 0)),
  };
}

/**
 * Memory is measured from outside, by the runner (the whole browser process tree, so a Worker
 * and WebAssembly count like everything else). The page stops here until it has.
 */
async function checkpoint(name) {
  if (!params.has("sync")) return;
  await new Promise((release) => {
    window.__bench.release = release;
    window.__bench.waiting = name;
  });
  window.__bench.waiting = null;
}

export async function start(adapter) {
  if (params.has("reuse")) globalThis.__ogReuseText = true;
  const result = { grid: adapter.name, rows: ROWS };
  window.__bench = { done: false, waiting: null, result };
  try {
    const want = expected(ROWS);
    const wantGroups = expectedGroups(ROWS);
    // A grid whose rows are in a database does not get them: the page holds none, as a page
    // that uses a server would not.
    let rows = adapter.remote ? null : makeRows(ROWS);
    const container = document.getElementById("grid");
    container.style.height = `${HEIGHT}px`;

    // First rows: from the rows being in memory to the first row on the screen. What the page
    // downloaded and compiled before that is in `startMs`, which is not compared.
    result.startMs = performance.now();
    let mounted;
    result.firstRows = await timed(
      () => {
        mounted = adapter.mount(container, rows, COLUMNS);
        return mounted;
      },
      () => adapter.firstRow()?.id === 1,
    );
    await mounted;

    await checkpoint("load");
    // EXPERIMENT (#518): the page lets go of its rows, and the browser collects, before anything is measured.
    if (params.has("release")) {
      rows = null;
      await window.__gc?.();
    }

    result.sort = await timed(
      () => adapter.sort("amount", "desc"),
      () => adapter.firstRow()?.id === want.topId,
    );

    result.filter = await timed(
      () => adapter.filter("country", "DE"),
      () => {
        const row = adapter.firstRow();
        const countries = adapter.visibleCountries();
        return row?.id === want.topDeId && countries.length > 0 && countries.every((c) => c === "DE");
      },
    );

    result.clear = await timed(
      async () => {
        await adapter.clearFilter();
        await adapter.sort("id", "asc");
      },
      () => adapter.firstRow()?.id === 1,
    );

    if (adapter.group) {
      result.group = await timed(
        () => adapter.group("country", { amount: "sum" }),
        () => groupsAreRight(adapter.groups(), wantGroups),
      );
      result.ungroup = await timed(
        () => adapter.ungroup(),
        () => adapter.firstRow()?.id === 1,
      );
    }

    await window.__metrics?.("before-windows");
    if (adapter.painted) result.windows = await windows(adapter, ROWS);
    await window.__metrics?.("after-windows");
    if (params.has("micro3")) {
      const m = {};
      const scroller = adapter.scroller();
      m.asis = microbench(scroller);
      await window.__gc?.();
      m.gc = microbench(scroller);
      rows = null;
      await window.__gc?.();
      m.release = microbench(scroller);
      result.micro3 = m;
    }
    await mark("scroll:start");
    result.scroll = await scroll(adapter);
    await mark("scroll:end");
    await checkpoint("after");
  } catch (error) {
    result.error = String(error?.stack ?? error);
  }
  window.__bench.done = true;
}
