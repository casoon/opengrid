// How large a pivot a browser draws as a native table (plan point 108).
//
// Serves the repository, opens tests/bench/pivot-size.html in Chromium and
// WebKit, and measures each size: the first draw (answer to painted), a
// sort's redraw, and the DOM it made. `just measure-pivot` runs it after the
// modules are built. Numbers, not a pass/fail: the limits are a decision
// recorded in E39 and plan/spezifikation/12-qualitaet.md.

import { spawn } from "node:child_process";
import { chromium, webkit } from "@playwright/test";

const SIZES = [
  [500, 64],
  [2000, 64],
  [2000, 256],
  [5000, 64],
  [5000, 128],
  [10000, 32],
  [10000, 64],
  [2000, 512],
  [20000, 16],
];
const PORT = 8097;

const server = spawn("python3", ["-m", "http.server", String(PORT), "--bind", "127.0.0.1"], {
  stdio: "ignore",
});
await new Promise((done) => setTimeout(done, 800));

try {
  for (const [name, engine] of [
    ["chromium", chromium],
    ["webkit", webkit],
  ]) {
    const browser = await engine.launch();
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await page.goto(`http://127.0.0.1:${PORT}/tests/bench/pivot-size.html`);
    await page.waitForFunction(() => window.ready === true);
    console.log(`\n${name}\nrows\tcolumns\tcells\tfirst ms\tsort ms\tnodes`);
    for (const [rows, columns] of SIZES) {
      // The median of three, after one to warm up.
      const runs = [];
      for (let i = 0; i < 4; i++) {
        runs.push(await page.evaluate(([r, c]) => window.measure(r, c), [rows, columns]));
      }
      const median = (key) => runs.slice(1).map((run) => run[key]).sort((a, b) => a - b)[1];
      const last = runs.at(-1);
      console.log(
        [rows, columns, rows * columns, median("first"), median("sort"), last.nodes].join("\t"),
      );
    }
    await browser.close();
  }
} finally {
  server.kill();
}
