// What every page runs: mount a grid on the same rows, then measure the same things, in the
// same order, in the same way. A grid is an adapter (`src/adapters/<name>.js`); everything
// here knows nothing about any of them.
//
// Adapter: { name, async mount(container, rows, columns), firstRow(), visibleCountries(),
//            async sort(field, direction), async filter(field, value), async clearFilter(),
//            scroller(), rowHeight(), destroy() } — `firstRow()` reads what is **on screen**
//            ({ id }), because a grid that has the answer and not yet the pixels has not answered.

import { COLUMNS, expected, makeRows } from "./data.js";

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

const percentile = (sorted, p) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))];
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
  try {
    observer.observe({ entryTypes: ["longtask"] });
  } catch {
    // Not every browser reports them.
  }
  scroller.scrollTop = 0;
  await frame();
  const frames = [];
  let last = await frame();
  for (let i = 0; i < 120; i++) {
    scroller.scrollTop += 3 * rowHeight;
    const now = await frame();
    frames.push(now - last);
    last = now;
  }
  const max = scroller.scrollHeight - scroller.clientHeight;
  for (let i = 0; i < 20; i++) {
    scroller.scrollTop = Math.min(max, (i + 1) * 2000 * rowHeight);
    const now = await frame();
    frames.push(now - last);
    last = now;
  }
  observer.disconnect();
  const moved = scroller.scrollTop > 0;
  scroller.scrollTop = 0;
  await frame();
  return { ...summarise(frames, longTasks), moved };
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
    const rows = makeRows(ROWS);
    const want = expected(rows);
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

    result.scroll = await scroll(adapter);
    await checkpoint("after");
  } catch (error) {
    result.error = String(error?.stack ?? error);
  }
  window.__bench.done = true;
}
