import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// The Svelte adapter (plan point 79), against the built example: `bind:view`,
// the callbacks, a prop taken away, and a grid that `{#if}` takes out and puts
// back. A development build, so Svelte's warnings exist, and every test fails
// on any of them.

test.use({ launchOptions: { args: ["--js-flags=--expose-gc"] } });

const PAGE = "/examples/svelte/dist/index.html";

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

function selectedRows(page) {
  return page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-selected") === "true")
      .map((tr) => tr.getAttribute("aria-rowindex")),
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
    for (const leaked of ["provider", "view", "windowsize", "windowSize"]) {
      expect(Object.keys(attributes)).not.toContain(leaked);
    }
  });

  test("the source is asked once", async ({ page }) => {
    const queries = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
    expect(queries).toHaveLength(1);
    expect(queries[0].sort).toEqual([{ field: "id", direction: "asc" }]);
  });

  test("bind:view holds the view, and a saved one comes back", async ({ page }) => {
    await page.evaluate(() => {
      window.__queries.length = 0;
    });
    await sortByAmount(page);
    await expect
      .poll(() => page.evaluate(() => window.__view.sort))
      .toEqual([{ field: "amount", direction: "asc" }]);
    await page.waitForTimeout(250);
    expect(await page.evaluate(() => window.__queries.length)).toBe(1);

    await page.getByRole("button", { name: "Restore the saved view" }).click();
    await expect.poll(() => ariaSort(page, 1)).toBe("ascending");
    expect(await ariaSort(page, 3)).toBe("none");
    await page.waitForTimeout(250);
    expect(await page.evaluate(() => window.__queries.length)).toBe(2);
  });

  test("a selection reaches Svelte", async ({ page }) => {
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="2"][data-col="0"]')
        .focus(),
    );
    await page.keyboard.press(" ");
    await expect(page.locator("#selected")).toHaveText("1 rows selected");
  });

  test("bind:element is the element", async ({ page }) => {
    expect(
      await page.evaluate(() => window.__element === document.querySelector("opengrid-grid")),
    ).toBe(true);
  });

  test("a change inside a $state object reaches the grid", async ({ page }) => {
    const status = () =>
      page.evaluate(
        () =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
            .textContent,
      );
    await page.getByRole("button", { name: "German" }).click();
    await expect.poll(status).toMatch(/Treffer$/);
    // The same object, one key changed in place.
    await page.getByRole("button", { name: "Say rows" }).click();
    await expect.poll(status).toMatch(/Zeilen$/);
  });

  test("a binding that refuses the reader's view gets its own back", async ({ page }) => {
    // Controlled: the page's setter keeps the old view, so the grid returns
    // to it — after one tick, not left showing what the page refused.
    await page.getByRole("button", { name: "Lock the view" }).click();
    await sortByAmount(page);
    await expect.poll(() => ariaSort(page, 0)).toBe("ascending");
    expect(await ariaSort(page, 3)).toBe("none");
    expect(await page.evaluate(() => window.__view.sort)).toEqual([
      { field: "id", direction: "asc" },
    ]);
  });

  test("a prop set and taken away again is reset", async ({ page }) => {
    const status = () =>
      page.evaluate(
        () =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
            .textContent,
      );
    await page.getByRole("button", { name: "German" }).click();
    await expect.poll(status).toMatch(/Treffer$/);
    await page.getByRole("button", { name: "German" }).click();
    await expect.poll(status).toMatch(/matches$/);
  });

  test("a grid taken out for good is collected; one put back asks once", async ({
    page,
    browserName,
  }) => {
    await page.evaluate(() => {
      window.__weak = new WeakRef(document.querySelector("opengrid-grid"));
    });
    await page.getByRole("button", { name: "Hide the grid" }).click();
    await expect(page.locator("opengrid-grid")).toHaveCount(0);

    // `window.gc` is Chromium's (`--expose-gc`); the other engines have no way
    // to force a collection, so there only the second half runs.
    if (browserName === "chromium") {
      let alive = true;
      for (let round = 0; round < 20 && alive; round += 1) {
        await page.evaluate(() => window.gc());
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

  test("axe finds nothing", async ({ page }) => {
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
});

test("the adapter renders on the server", () => {
  // In Node, compiled by Vite with the Svelte plugin (the adapter ships
  // `.svelte` sources). The script lives in the example, where `svelte`, Vite
  // and the adapter resolve.
  const html = execFileSync("node", ["examples/svelte/ssr.mjs"], {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    encoding: "utf8",
  });
  // Svelte marks its blocks with comments for hydration; the element is what
  // this is about.
  expect(html.replace(/<!--.*?-->/g, "")).toBe(
    '<opengrid-grid label="Orders" datasource="orders" columns="id,customer" window-size="40" selection="" class="orders"></opengrid-grid>',
  );
});
