import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` over the Worker provider (plan point 19).
//
// `worker-grid.html` loads the engine in a module worker (`createWorkerProvider`)
// and attaches the provider to the grid. This spec proves the worker path
// renders, sorts and filters exactly like the main-thread path, that the engine
// really runs off-thread, that the local fallback still works, and that the main
// thread stays responsive while a large query runs.

/** The rendered grid facts. */
async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const table = root.querySelector("table");
    return {
      role: table.getAttribute("role"),
      ariaLabel: table.getAttribute("aria-label"),
      rowcount: table.getAttribute("aria-rowcount"),
      colcount: table.getAttribute("aria-colcount"),
      status: root.querySelector('[part="status"]')?.textContent ?? null,
    };
  });
}

/** The rendered text of one column, top to bottom (assigned rows only). */
async function columnText(page, column) {
  return page.evaluate((column) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => tr.querySelectorAll("td")[column].textContent);
  }, column);
}

async function statusText(page) {
  return page.evaluate(
    () =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector(
        '[part="status"]',
      ).textContent,
  );
}

/** Focuses an element inside the shadow root. */
async function focusIn(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

/** Selects an operator by index using only the keyboard (Home + ArrowDown). */
async function chooseOperator(page, column, index) {
  await focusIn(page, `select[data-col="${column}"]`);
  await page.keyboard.press("Home");
  for (let step = 0; step < index; step += 1) {
    await page.keyboard.press("ArrowDown");
  }
}

/** Replaces the value input's text and applies the filter with Enter. */
async function applyFilter(page, column, value) {
  await focusIn(page, `input[data-col="${column}"]`);
  await page.keyboard.press("ControlOrMeta+A");
  await page.keyboard.type(value);
  await page.keyboard.press("Enter");
}

async function waitForGrid(page) {
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
}

test.describe("worker provider", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/e2e/fixtures/worker-grid.html");
    await waitForGrid(page);
  });

  test("runs the engine off-thread and renders through the worker", async ({
    page,
  }) => {
    const rendered = await facts(page);
    expect(rendered).toMatchObject({
      role: "grid",
      ariaLabel: "Bestellungen",
      rowcount: "6",
      colcount: "4",
    });

    // The engine is off-thread: one Worker was created, the page imported no
    // engine module and made no main-thread Engine.
    const proof = await page.evaluate(() => ({
      workers: window.__workers.length,
      workerScript: String(window.__workers[0]),
      realWorker: window.__provider.worker instanceof Worker,
      mainThreadEngine: window.__mainThreadEngine,
    }));
    expect(proof.workers).toBe(1);
    expect(proof.workerScript).toContain("worker.js");
    expect(proof.realWorker).toBe(true);
    expect(proof.mainThreadEngine).toBe(false);
  });

  test("sorts through the worker", async ({ page }) => {
    await focusIn(page, 'th[data-col="1"]');
    await page.keyboard.press("Enter");
    await expect
      .poll(() => columnText(page, 1))
      .toEqual(["Alpha", "Alpha", "Beta", "Beta", "Gamma"]);
  });

  test("filters through the worker", async ({ page }) => {
    await chooseOperator(page, 1, 2);
    await applyFilter(page, 1, "Beta");
    await expect.poll(() => statusText(page)).toBe("2 matches");
    expect(await columnText(page, 1)).toEqual(["Beta", "Beta"]);
  });

  test("has no axe violations", async ({ page }) => {
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
});

test.describe("main-thread fallback (createLocalProvider)", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/e2e/fixtures/grid.html");
    await waitForGrid(page);
  });

  test("renders and sorts without a worker", async ({ page }) => {
    const proof = await page.evaluate(() => ({
      mainThreadEngine: window.__mainThreadEngine,
      hasProvider: !!window.__localProvider,
      workers: window.__workers?.length ?? 0,
    }));
    expect(proof.mainThreadEngine).toBe(true);
    expect(proof.hasProvider).toBe(true);
    expect(proof.workers).toBe(0);

    await focusIn(page, 'th[data-col="1"]');
    await page.keyboard.press("Enter");
    await expect
      .poll(() => columnText(page, 1))
      .toEqual(["Alpha", "Alpha", "Beta", "Beta", "Gamma"]);
  });

  test("has no axe violations", async ({ page }) => {
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
});

test("keeps the main thread responsive during a large query", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/worker-grid.html");
  await waitForGrid(page);

  const measurement = await page.evaluate(async () => {
    const ROWS = 100000;
    const QUERIES = 40;
    await window.__loadLarge(ROWS);

    // A requestAnimationFrame ticker records the gaps between frames while the
    // queries run. A blocked main thread shows up as a handful of huge gaps.
    const gaps = [];
    let running = true;
    let last = performance.now();
    const tick = (now) => {
      gaps.push(now - last);
      last = now;
      if (running) {
        requestAnimationFrame(tick);
      }
    };
    requestAnimationFrame(tick);

    const query = JSON.stringify({
      source: "large",
      select: ["id", "customer", "amount", "qty"],
      sort: [{ field: "amount", direction: "desc" }],
      limit: 50,
    });

    const started = performance.now();
    for (let i = 0; i < QUERIES; i += 1) {
      await window.__provider.execute(query);
    }
    const duration = performance.now() - started;
    running = false;

    return {
      rows: ROWS,
      queries: QUERIES,
      duration,
      perQuery: duration / QUERIES,
      frames: gaps.length,
      maxGap: Math.max(...gaps),
    };
  });

  // Recorded for the spec: the exact numbers come from the run below.
  console.log(
    `[worker responsiveness] ${measurement.rows} rows, ` +
      `${measurement.queries} queries in ${measurement.duration.toFixed(0)} ms ` +
      `(${measurement.perQuery.toFixed(1)} ms/query), ` +
      `${measurement.frames} frames, max gap ${measurement.maxGap.toFixed(1)} ms`,
  );

  // The queries take long enough to observe, the ticker keeps advancing, and no
  // single frame gap comes close to a blocking operation.
  expect(measurement.duration).toBeGreaterThan(100);
  expect(measurement.frames).toBeGreaterThan(5);
  expect(measurement.maxGap).toBeLessThan(100);
});
