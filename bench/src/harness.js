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
  scroller.scrollTop = 0;
  await frame();
  const frames = [];
  let last = await frame();
  const fineStart = last;
  for (let i = 0; i < 120; i++) {
    scroller.scrollTop += 3 * rowHeight;
    const now = await frame();
    frames.push(now - last);
    last = now;
  }
  const jumpStart = last;
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
  const result = { grid: adapter.name, rows: ROWS };
  window.__bench = { done: false, waiting: null, result };
  try {
    const want = expected(ROWS);
    const wantGroups = expectedGroups(ROWS);
    // A grid whose rows are in a database does not get them: the page holds none, as a page
    // that uses a server would not.
    const rows = adapter.remote ? null : makeRows(ROWS);
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

    if (adapter.painted) result.windows = await windows(adapter, ROWS);
    await mark("scroll:start");
    result.scroll = await scroll(adapter);
    await mark("scroll:end");
    await checkpoint("after");
  } catch (error) {
    result.error = String(error?.stack ?? error);
  }
  window.__bench.done = true;
}
