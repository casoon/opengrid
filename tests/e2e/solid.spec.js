import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// The Solid 2 adapter (#510), against the built example: the view held in a
// signal, the options and what happens when one goes away, a lock that refuses
// the reader's view, and a grid that `<Show>` takes out and puts back. A
// development build, so Solid's warnings exist — a read outside a tracking
// scope, a write inside an owned one — and every test fails on any of them.

const PAGE = "/examples/solid/dist/index.html";

async function rows(page) {
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );
}

function ariaSort(page, col) {
  return page.evaluate(
    (col) =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector(`th[data-col="${col}"]`)
        .getAttribute("aria-sort"),
    col,
  );
}

async function sortByAmount(page) {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="3"]').focus(),
  );
  await page.keyboard.press("Enter");
}

function status(page) {
  return page.evaluate(
    () =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
        .textContent,
  );
}

test.describe("in the browser", () => {
  let problems;

  test.beforeEach(async ({ page }) => {
    problems = [];
    page.on("pageerror", (error) => problems.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error" || message.type() === "warning") {
        problems.push(message.text());
      }
    });
    await page.goto(PAGE);
    await rows(page);
    await page.waitForTimeout(250);
  });

  test.afterEach(() => {
    expect(problems).toEqual([]);
  });

  test("it is the Solid this build says it is", async ({ page }) => {
    await expect(page.locator("h1")).toContainText("Solid 2.");
  });

  test("the attributes arrive as HTML has them", async ({ page }) => {
    const attributes = await page.evaluate(() =>
      Object.fromEntries(
        [...document.querySelector("opengrid-grid").attributes].map((a) => [a.name, a.value]),
      ),
    );
    expect(attributes).toMatchObject({
      label: "Orders",
      datasource: "orders",
      columns: "id,customer,country,amount,qty",
      "window-size": "40",
      selection: "",
      toolbar: "",
      class: "orders",
    });
    // Nothing Solid-shaped leaks onto the element, and no option became an attribute.
    for (const leaked of ["provider", "view", "texts", "ref", "windowsize", "windowSize", "onviewchange"]) {
      expect(Object.keys(attributes)).not.toContain(leaked);
    }
  });

  test("the source is asked once", async ({ page }) => {
    const queries = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
    expect(queries).toHaveLength(1);
    expect(queries[0].sort).toEqual([{ field: "id", direction: "asc" }]);
    expect(await status(page)).toMatch(/matches$/);
  });

  test("the view lives in a signal, and a saved one comes back", async ({ page }) => {
    await page.evaluate(() => {
      window.__queries.length = 0;
    });
    await sortByAmount(page);
    await expect
      .poll(() => page.evaluate(() => window.__view.sort))
      .toEqual([{ field: "amount", direction: "asc" }]);
    await page.waitForTimeout(250);
    // Reported, written into the signal, handed back: once, not in a loop.
    expect(await page.evaluate(() => window.__queries.length)).toBe(1);

    await page.getByRole("button", { name: "Restore the saved view" }).click();
    await expect.poll(() => ariaSort(page, 1)).toBe("ascending");
    expect(await ariaSort(page, 3)).toBe("none");
    await page.waitForTimeout(250);
    expect(await page.evaluate(() => window.__queries.length)).toBe(2);
  });

  test("a view the page refuses is written back", async ({ page }) => {
    // Controlled: the page's handler keeps the old view, so the grid returns to
    // it once the page's updates have flushed — not left showing what the page
    // refused.
    await page.getByRole("button", { name: "Lock the view" }).click();
    await sortByAmount(page);
    await expect.poll(() => ariaSort(page, 0)).toBe("ascending");
    expect(await ariaSort(page, 3)).toBe("none");
    expect(await page.evaluate(() => window.__view.sort)).toEqual([{ field: "id", direction: "asc" }]);
  });

  test("the ref is the element, and carries the connection's writes", async ({ page }) => {
    expect(await page.evaluate(() => window.__grid === document.querySelector("opengrid-grid"))).toBe(true);
    expect(
      await page.evaluate(() => [typeof window.__grid.setCellState, typeof window.__grid.setValues]),
    ).toEqual(["function", "function"]);
  });

  test("a prop set and taken away again is reset", async ({ page }) => {
    await page.getByRole("button", { name: "German" }).click();
    await expect.poll(() => status(page)).toMatch(/Treffer$/);
    await page.getByRole("button", { name: "German" }).click();
    await expect.poll(() => status(page)).toMatch(/matches$/);
  });

  test("the provider compares by identity: an update that is none asks nothing, a new provider asks once", async ({
    page,
  }) => {
    await page.evaluate(() => {
      window.__queries.length = 0;
    });
    // The page changes, the grid's options do not: nothing is asked.
    await page.getByRole("button", { name: "Hidden" }).click();
    await page.getByRole("button", { name: "Hidden" }).click();
    await page.waitForTimeout(250);
    expect(await page.evaluate(() => window.__queries.length)).toBe(0);

    // A provider that is another object is another source: asked once more, and no more.
    await page.getByRole("button", { name: "New provider" }).click();
    await expect.poll(() => page.evaluate(() => window.__queries.length)).toBe(1);
    await page.waitForTimeout(250);
    expect(await page.evaluate(() => window.__queries.length)).toBe(1);
  });

  test("hidden hides the grid when true, and only then", async ({ page }) => {
    const hidden = () =>
      page.evaluate(() => {
        const grid = document.querySelector("opengrid-grid");
        return [grid.hidden, grid.hasAttribute("hidden")];
      });
    expect(await hidden()).toEqual([false, false]);
    await page.getByRole("button", { name: "Hidden" }).click();
    await expect.poll(hidden).toEqual([true, true]);
    await page.getByRole("button", { name: "Hidden" }).click();
    await expect.poll(hidden).toEqual([false, false]);
  });

  test("a selection reaches Solid", async ({ page }) => {
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="2"][data-col="0"]')
        .focus(),
    );
    await page.keyboard.press(" ");
    await expect(page.locator("#selected")).toHaveText("1 rows selected");
  });

  test("a grid taken out by Solid is collected; one put back asks once", async ({ page, browserName }) => {
    await page.evaluate(() => {
      window.__weak = new WeakRef(document.querySelector("opengrid-grid"));
      // The ref held the element for the tests; let go of it.
      window.__grid = null;
    });
    await page.getByRole("button", { name: "Hide the grid" }).click();
    await expect(page.locator("opengrid-grid.orders")).toHaveCount(0);

    // `window.gc` is Chromium's (`--expose-gc`); the other engines have no way
    // to force a collection, so there only the second half runs.
    if (browserName === "chromium") {
      let alive = true;
      for (let round = 0; round < 20 && alive; round += 1) {
        // Async: a collection in a task of its own, with no stack that a stale
        // pointer could keep the grid alive from (V8 scans the stack conservatively).
        await page.evaluate(() => window.gc({ type: "major", execution: "async" }));
        await page.waitForTimeout(50);
        alive = await page.evaluate(() => window.__weak.deref() !== undefined);
      }
      expect(alive).toBe(false);
    }

    await page.evaluate(() => {
      window.__queries.length = 0;
    });
    await page.getByRole("button", { name: "Show the grid" }).click();
    await rows(page);
    await page.waitForTimeout(250);
    expect(await page.evaluate(() => window.__queries.length)).toBe(1);
  });

  test("a grid taken out stops reporting: its listeners are gone", async ({ page }) => {
    await page.evaluate(() => {
      window.__detached = document.querySelector("opengrid-grid");
    });
    await page.getByRole("button", { name: "Hide the grid" }).click();
    await expect(page.locator("opengrid-grid.orders")).toHaveCount(0);
    // The element lives on in the page's variable; what it says is no longer heard.
    await page.evaluate(() =>
      window.__detached.dispatchEvent(new CustomEvent("opengrid-selection-change", { detail: { count: 7 } })),
    );
    await page.waitForTimeout(100);
    await expect(page.locator("#selected")).toHaveText("0 rows selected");
  });

  test("axe finds nothing", async ({ page }) => {
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
});

test("the adapter renders on the server", () => {
  // In Node: no window, no document, no customElements. What comes out is the
  // element with its attributes — labelled before any script runs — and the key
  // Solid hydrates it by. The script lives in the example, where `solid-js` and
  // the adapter resolve.
  const html = execFileSync("node", ["examples/solid/ssr.mjs"], {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    encoding: "utf8",
  });
  expect(html).toMatch(
    /^<opengrid-grid _hk=\d+ label="Orders" datasource="orders" columns="id,customer" window-size="40" selection class="orders"><\/opengrid-grid>$/,
  );
});
