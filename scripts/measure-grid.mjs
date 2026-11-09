// What <opengrid-grid> costs while it scrolls, filters and selects everything
// (issue #235), over 100 000 rows and 10 columns from the real engine.
//
// Serves the repository, opens tests/bench/grid-scroll.html in Chromium and
// WebKit, and measures in fresh pages (RUNS of them, median reported with the
// spread, because other processes load the machine):
//
//   scroll     120 steps of 3 rows, then 20 jumps of 2000 rows: per step the
//              script, style and layout time (Chromium: CDP Performance
//              metrics), the DOM writes (setAttribute, removeAttribute,
//              textContent, style, appendChild; counted at the prototype) and
//              the JS heap bytes allocated (Chromium: sampling profiler).
//   resize     20 width changes of the host (the filter row follows).
//   filter     a value typed into the region filter and Enter: from the key to
//              the redrawn status line, with the same counters.
//   select all Ctrl+A over the 100 000 rows: the handler's synchronous time
//              and until the next frame.
//
// `just measure-grid` runs it after the modules are built. Numbers, not a
// pass/fail; the PR that changes the renderer quotes them.
//
//   node scripts/measure-grid.mjs [--runs 5] [--engines chromium,webkit]

import { spawn } from "node:child_process";
import { chromium, webkit } from "@playwright/test";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at >= 0 ? args[at + 1] : fallback;
};
const RUNS = Number(option("runs", 5));
const ENGINES = option("engines", "chromium,webkit").split(",");
const PORT = 8098;
const URL = `http://127.0.0.1:${PORT}/tests/bench/grid-scroll.html`;

const server = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1"], {
  stdio: "ignore",
});
await new Promise((done) => setTimeout(done, 800));

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
const spread = (values) => `${Math.min(...values).toFixed(1)}..${Math.max(...values).toFixed(1)}`;

/** The CDP metrics a Chromium page can give, as a plain object. */
async function metrics(cdp) {
  if (!cdp) return {};
  const { metrics } = await cdp.send("Performance.getMetrics");
  return Object.fromEntries(metrics.map((metric) => [metric.name, metric.value]));
}

async function startSampling(cdp) {
  if (!cdp) return;
  await cdp.send("HeapProfiler.enable");
  await cdp.send("HeapProfiler.startSampling", {
    samplingInterval: 256,
    includeObjectsCollectedByMajorGC: true,
    includeObjectsCollectedByMinorGC: true,
  });
}

async function stopSampling(cdp) {
  if (!cdp) return 0;
  const { profile } = await cdp.send("HeapProfiler.stopSampling");
  // The samples are a statistical estimate of the bytes allocated.
  let bytes = 0;
  const walk = (node) => {
    bytes += node.selfSize ?? 0;
    node.children?.forEach(walk);
  };
  walk(profile.head);
  return bytes;
}

async function writes(page) {
  return page.evaluate(() => {
    const now = { ...window.__writes };
    for (const key of Object.keys(window.__writes)) window.__writes[key] = 0;
    return now;
  });
}

const sum = (writesObject) => Object.values(writesObject).reduce((a, b) => a + b, 0);

/** Runs `act` and reports the counters per `steps`. */
async function counted(page, cdp, steps, act) {
  await writes(page);
  const before = await metrics(cdp);
  await startSampling(cdp);
  const started = Date.now();
  const inPage = await act();
  const wall = Date.now() - started;
  const allocated = await stopSampling(cdp);
  const after = await metrics(cdp);
  const delta = (key) => ((after[key] ?? 0) - (before[key] ?? 0)) * 1000;
  const done = await writes(page);
  return {
    "wall ms/step": wall / steps,
    "script ms/step": delta("ScriptDuration") / steps,
    "style ms/step": delta("RecalcStyleDuration") / steps,
    "layout ms/step": delta("LayoutDuration") / steps,
    "task ms/step": delta("TaskDuration") / steps,
    "style recalcs/step": ((after.RecalcStyleCount ?? 0) - (before.RecalcStyleCount ?? 0)) / steps,
    "layouts/step": ((after.LayoutCount ?? 0) - (before.LayoutCount ?? 0)) / steps,
    "DOM writes/step": sum(done) / steps,
    "  setAttribute": done.setAttribute / steps,
    "  removeAttribute": done.removeAttribute / steps,
    "  textContent": done.textContent / steps,
    "  style": done.style / steps,
    "  appendChild": done.append / steps,
    "JS KiB alloc/step": allocated / 1024 / steps,
    ...inPage,
  };
}

const frames = () =>
  new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 0))));

async function scrollRun(page, cdp) {
  await page.evaluate(() => {
    const viewport = document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="viewport"]');
    viewport.scrollTop = 0;
  });
  await page.evaluate(`(${frames})()`);
  const small = await counted(page, cdp, 120, () =>
    page.evaluate(async () => {
      const viewport = document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('[part="viewport"]');
      const next = () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 0))),
        );
      for (let step = 1; step <= 120; step += 1) {
        viewport.scrollTop = step * 3 * 32;
        await next();
      }
      return {};
    }),
  );
  const jump = await counted(page, cdp, 20, () =>
    page.evaluate(async () => {
      const viewport = document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('[part="viewport"]');
      const next = () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 0))),
        );
      for (let step = 1; step <= 20; step += 1) {
        viewport.scrollTop = 20_000 * 32 + step * 2000 * 32;
        await next();
      }
      return {};
    }),
  );
  return { "scroll 3 rows": small, "scroll jump 2000 rows": jump };
}

async function filterRun(page, cdp) {
  const root = "document.querySelector('opengrid-grid').shadowRoot";
  await page.evaluate(`(() => {
    const status = ${root}.querySelector('[part="status"]');
    window.__statusChanged = new Promise((done) => {
      new MutationObserver(() => done(performance.now())).observe(status, {
        childList: true, characterData: true, subtree: true,
      });
    });
    document.addEventListener('keydown', () => { window.__keyAt = performance.now(); }, { capture: true, once: true });
  })()`);
  await page.locator('opengrid-grid input[data-col="1"]').fill("north");
  return counted(page, cdp, 1, async () => {
    await page.keyboard.press("Enter");
    const took = await page.evaluate(async () => {
      const at = await window.__statusChanged;
      await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
      return performance.now() - window.__keyAt + 0 * at;
    });
    return { "key to drawn ms": took };
  });
}

/** The host narrower and wider again: the filter row follows (`align_filter`). */
async function resizeRun(page, cdp) {
  return counted(page, cdp, 20, () =>
    page.evaluate(async () => {
      const host = document.querySelector("opengrid-grid");
      const next = () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 0))),
        );
      for (let step = 0; step < 20; step += 1) {
        host.style.width = step % 2 ? "1600px" : "1100px";
        await next();
      }
      return {};
    }),
  );
}

async function selectAllRun(page, cdp) {
  await page.evaluate(`(async () => {
    const root = document.querySelector('opengrid-grid').shadowRoot;
    root.querySelector('td[data-row][data-col="0"]').focus();
  })()`);
  return counted(page, cdp, 1, async () => {
    const took = await page.evaluate(async () => {
      window.__selected = 0;
      document.addEventListener("opengrid-selection-change", (event) => {
        window.__selected = event.detail.count;
      });
      const cell = document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row][data-col="0"]');
      const start = performance.now();
      cell.dispatchEvent(
        new KeyboardEvent("keydown", { key: "a", ctrlKey: true, bubbles: true, composed: true }),
      );
      const sync = performance.now() - start;
      await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
      return { sync, drawn: performance.now() - start, count: window.__selected };
    });
    return {
      "handler ms": took.sync,
      "key to frame ms": took.drawn,
      "rows reported": took.count,
    };
  });
}

try {
  for (const name of ENGINES) {
    const engine = name === "webkit" ? webkit : chromium;
    const browser = await engine.launch();
    const results = [];
    for (let run = 0; run < RUNS; run += 1) {
      const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
      const page = await context.newPage();
      const cdp = name === "chromium" ? await context.newCDPSession(page) : null;
      if (cdp) await cdp.send("Performance.enable");
      await page.goto(URL);
      await page.waitForFunction(() => window.__opengridReady);
      await page.evaluate(() => window.__opengridReady);
      await page.waitForSelector("opengrid-grid td[data-row]");
      await page.evaluate(`(${frames})()`);
      const scenarios = {
        ...(await scrollRun(page, cdp)),
        resize: await resizeRun(page, cdp),
        "select all": await selectAllRun(page, cdp),
        filter: await filterRun(page, cdp),
      };
      results.push(scenarios);
      await context.close();
    }
    await browser.close();

    console.log(`\n## ${name} (median of ${RUNS} fresh pages, min..max in brackets)`);
    for (const scenario of Object.keys(results[0])) {
      console.log(`\n${scenario}`);
      for (const key of Object.keys(results[0][scenario])) {
        const values = results.map((result) => result[scenario][key]);
        if (values.every((value) => !value)) continue;
        console.log(`  ${key.padEnd(22)} ${median(values).toFixed(2).padStart(10)}  [${spread(values)}]`);
      }
    }
  }
} finally {
  server.kill();
}
