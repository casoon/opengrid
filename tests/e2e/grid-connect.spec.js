import { test, expect } from "@playwright/test";

// `connect` (plan point 76): an element supplied from one options object.
//
// The module functions have rules a page otherwise has to know — wait for the
// module, texts before the provider, the view before the provider, write a
// controlled view back without a loop. These tests hold each rule where it
// shows: the number of queries, and what the status line says, in order.

async function open(page, search = "") {
  await page.goto(`/tests/e2e/fixtures/grid-connect.html${search}`);
  await page.waitForFunction(() => window.__ready);
}

/** Records every distinct state of the status line (announcements.spec.js). */
async function record(page) {
  await page.waitForFunction(() => document.querySelector("opengrid-grid")?.shadowRoot);
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const seen = [];
    const read = () => {
      const text = root.querySelector('[part="status"]')?.textContent.trim();
      if (text !== undefined && seen.at(-1) !== text) seen.push(text);
    };
    read();
    new MutationObserver(read).observe(root, {
      subtree: true,
      childList: true,
      characterData: true,
    });
    window.__announced = seen;
  });
}

async function rows(page) {
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot?.querySelector("td[data-row]"),
  );
}

/** Sorts by the amount column from its header, the way a reader does. */
async function sortByAmount(page) {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="3"]').focus(),
  );
  await page.keyboard.press("Enter");
}

const GERMAN = { lang: "de", loading: "Wird geladen …", matchesOther: "{count} Treffer" };

test("one object: one query, in the right words", async ({ page }) => {
  await open(page);
  // The elements are registered already, as on a page that loaded the module
  // for another grid: this one has painted its skeleton, in English.
  await page.evaluate(async () => (await import("/packages/opengrid/loader.js")).loadOpengrid());
  await record(page);
  await page.evaluate(async (texts) => {
    const grid = document.querySelector("opengrid-grid");
    await window.__connect(grid, {
      provider: window.__provider(),
      texts,
      presentation: { amount: { align: "start" } },
      view: { sort: [{ field: "amount", direction: "desc" }] },
    }).ready;
  }, GERMAN);
  await rows(page);
  await page.waitForTimeout(250);

  // Texts, view and provider in the one order that asks once: the view before
  // the provider, or the grid would ask for the default sort first.
  const queries = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
  expect(queries).toHaveLength(1);
  expect(queries[0].sort[0]).toMatchObject({ field: "amount", direction: "desc" });
  const said = await page.evaluate(() => window.__announced);
  expect(said.at(-1)).toMatch(/Treffer$/);
  expect(said.filter((text) => /match/.test(text))).toEqual([]);
});

test("a controlled view written back on every change asks once", async ({ page }) => {
  await open(page);
  await page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    window.__views = [];
    const connection = window.__connect(grid, {
      provider: window.__provider(),
      view: { sort: [{ field: "id", direction: "asc" }] },
      onViewChange: (view) => {
        window.__views.push(view);
        // What a framework does with `value` + `onChange`.
        connection.update({ view });
      },
    });
    await connection.ready;
  });
  await rows(page);
  await page.waitForTimeout(250);
  await record(page);
  await page.evaluate(() => {
    window.__queries.length = 0;
    window.__views.length = 0;
  });

  await sortByAmount(page);
  await page.waitForTimeout(400);

  expect(await page.evaluate(() => window.__queries.length)).toBe(1);
  expect(await page.evaluate(() => window.__views.length)).toBe(1);
  // What was said after the status line's state at the start of recording:
  // the result once — a loop would say it again, or say "loading" twice.
  const said = (await page.evaluate(() => window.__announced)).slice(1);
  expect(said.filter((text) => /matches$/.test(text))).toHaveLength(1);
  expect(said.filter((text) => /Loading/.test(text)).length).toBeLessThanOrEqual(1);
});

test("an update before the module is loaded is folded in", async ({ page }) => {
  await open(page);
  await page.evaluate(async (texts) => {
    const grid = document.querySelector("opengrid-grid");
    const connection = window.__connect(grid, {
      provider: window.__provider(),
      texts: { loading: "…" },
    });
    // Synchronously, before `ready`: nothing may be lost or applied twice.
    connection.update({ texts });
    await connection.ready;
  }, GERMAN);
  await rows(page);
  await page.waitForTimeout(250);

  expect(await page.evaluate(() => window.__queries.length)).toBe(1);
  const status = await page.evaluate(
    () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent,
  );
  expect(status).toMatch(/Treffer$/);
});

test("undefined resets an option; a key left out keeps it", async ({ page }) => {
  await open(page);
  await page.evaluate(async (texts) => {
    const grid = document.querySelector("opengrid-grid");
    window.__connection = window.__connect(grid, { provider: window.__provider(), texts });
    await window.__connection.ready;
  }, GERMAN);
  await rows(page);
  const status = () =>
    page.evaluate(
      () => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent,
    );

  await page.evaluate(() => window.__connection.update({ choices: { customer: ["Alpha"] } }));
  await page.waitForTimeout(250);
  expect(await status()).toMatch(/Treffer$/);

  await page.evaluate(() => window.__connection.update({ texts: undefined }));
  await expect.poll(status).toMatch(/matches$/);
});

test("after disconnect the page hears nothing, and the grid keeps its state", async ({
  page,
}) => {
  await open(page);
  await page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    window.__views = [];
    window.__connection = window.__connect(grid, {
      provider: window.__provider(),
      onViewChange: (view) => window.__views.push(view),
    });
    await window.__connection.ready;
  });
  await rows(page);
  await sortByAmount(page);
  await expect.poll(() => page.evaluate(() => window.__views.length)).toBe(1);

  await page.evaluate(() => window.__connection.disconnect());
  await sortByAmount(page);
  await page.waitForTimeout(300);

  expect(await page.evaluate(() => window.__views.length)).toBe(1);
  const sort = await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('th[data-col="3"]')
      .getAttribute("aria-sort"),
  );
  expect(sort).toBe("descending");
});

test("a view given before the element is in the document waits, with the provider", async ({
  page,
}) => {
  await open(page);
  const result = await page.evaluate(async () => {
    const grid = document.createElement("opengrid-grid");
    for (const [name, value] of [
      ["label", "Später"],
      ["datasource", "orders"],
      ["columns", "id,customer,country,amount,qty"],
    ]) {
      grid.setAttribute(name, value);
    }
    const connection = window.__connect(grid, {
      provider: window.__provider(),
      view: { sort: [{ field: "customer", direction: "desc" }] },
    });
    await connection.ready;
    document.querySelector("main").append(grid);
    connection.update();
    await new Promise((resolve) => setTimeout(resolve, 300));
    return window.__queries.map((q) => JSON.parse(q).sort[0]);
  });
  // One query, and it already has the view's order: the provider waited with
  // the view instead of asking for the default order first.
  expect(result).toEqual([{ field: "customer", direction: "desc" }]);
});

test("a saved view comes back after the reader sorted", async ({ page }) => {
  // The controlled contract: `view` is what the grid shows whenever it differs
  // — so a page restoring its saved view after the reader moved on gets it.
  await open(page);
  const saved = { sort: [{ field: "customer", direction: "asc" }] };
  await page.evaluate(async (saved) => {
    const grid = document.querySelector("opengrid-grid");
    window.__connection = window.__connect(grid, {
      provider: window.__provider(),
      view: saved,
      onViewChange: (view) => window.__connection.update({ view }),
    });
    await window.__connection.ready;
  }, saved);
  await rows(page);
  await sortByAmount(page);
  await page.waitForTimeout(300);
  const ariaSort = (col) =>
    page.evaluate(
      (col) =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector(`th[data-col="${col}"]`)
          .getAttribute("aria-sort"),
      col,
    );
  expect(await ariaSort(3)).toBe("ascending");

  await page.evaluate((saved) => window.__connection.update({ view: saved }), saved);
  await page.waitForTimeout(300);
  expect(await ariaSort(1)).toBe("ascending");
  expect(await ariaSort(3)).toBe("none");
});

test("the same partial view passed again asks nothing", async ({ page }) => {
  // A framework re-renders with the view it has — often a part of one. The
  // grid normalizes it (a sort is added, S6), so it never equals what the grid
  // reports; connect has to know it wrote it.
  await open(page);
  await page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    window.__connection = window.__connect(grid, {
      provider: window.__provider(),
      view: { density: "compact" },
    });
    await window.__connection.ready;
  });
  await rows(page);
  await page.waitForTimeout(250);
  await page.evaluate(() => {
    window.__queries.length = 0;
    for (let i = 0; i < 3; i += 1) window.__connection.update({ view: { density: "compact" } });
  });
  await page.waitForTimeout(250);
  expect(await page.evaluate(() => window.__queries.length)).toBe(0);
});

test("a view the grid refused is tried again", async ({ page }) => {
  // Refused because it names a column the grid does not show yet — a framework
  // may pass the view before it patches the attribute. Nothing is recorded as
  // written, so the next update tries again.
  await open(page);
  const view = { sort: [{ field: "qty", direction: "desc" }] };
  await page.evaluate(async (view) => {
    const grid = document.querySelector("opengrid-grid");
    grid.setAttribute("columns", "id,customer,country,amount");
    window.__connection = window.__connect(grid, { provider: window.__provider(), view });
    await window.__connection.ready;
  }, view);
  await rows(page);
  await page.evaluate((view) => {
    document.querySelector("opengrid-grid").setAttribute("columns", "id,customer,country,amount,qty");
    window.__connection.update({ view });
  }, view);
  await page.waitForTimeout(300);
  const sort = await page.evaluate(
    async () =>
      (await (await import("/packages/opengrid/loader.js")).loadOpengrid()).module.get_view(
        document.querySelector("opengrid-grid"),
      ).sort,
  );
  expect(sort).toEqual(view.sort);
});

test("defaultView is applied once, then the grid leads", async ({ page }) => {
  await open(page);
  await page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    window.__connection = window.__connect(grid, {
      provider: window.__provider(),
      defaultView: { sort: [{ field: "customer", direction: "asc" }] },
    });
    await window.__connection.ready;
  });
  await rows(page);
  await sortByAmount(page);
  await page.waitForTimeout(300);
  await page.evaluate(() =>
    window.__connection.update({
      defaultView: { sort: [{ field: "customer", direction: "asc" }] },
      texts: { lang: "en" },
    }),
  );
  await page.waitForTimeout(300);
  const ariaSort = await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('th[data-col="3"]')
      .getAttribute("aria-sort"),
  );
  expect(ariaSort).toBe("ascending");
});

test("a null view is no view", async ({ page }) => {
  // `useState(null)` for "nothing saved yet" is common; it must not reach the
  // grid as a view that "is not an object".
  await open(page);
  await page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    await window.__connect(grid, { provider: window.__provider(), view: null }).ready;
  });
  await rows(page);
  const state = await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="status"]')
      .getAttribute("data-state"),
  );
  expect(state).toBe("ready");
});

test("without the module, connect says so and does nothing", async ({ page }) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await open(page, "?fallback");
  const result = await page.evaluate(async () => {
    const grid = document.querySelector("opengrid-grid");
    const connection = window.__connect(grid, { provider: window.__provider() });
    const loaded = await connection.ready;
    connection.update({ texts: { loading: "…" } });
    connection.disconnect();
    return loaded.fallback;
  });
  expect(result).toBe(true);
  expect(errors).toEqual([]);
});

test("importing the package touches no DOM (server-side rendering)", async () => {
  // Runs in Node, where there is no window, no document and no customElements.
  const loader = await import("../../packages/opengrid/loader.js");
  expect(typeof loader.connect).toBe("function");
  expect(typeof globalThis.document).toBe("undefined");
});
