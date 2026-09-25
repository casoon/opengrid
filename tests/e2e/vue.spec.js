import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// The Vue adapter (plan point 78), against the built example: `v-model:view`,
// the events, `<KeepAlive>` — which detaches the grid without unmounting it —
// and a grid that is taken out for good. A development build, so Vue's
// warnings exist, and every test fails on any of them.

test.use({ launchOptions: { args: ["--js-flags=--expose-gc"] } });

const PAGE = "/examples/vue/dist/index.html";

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
    for (const leaked of ["provider", "view", "windowsize", "window-size-"]) {
      expect(Object.keys(attributes)).not.toContain(leaked);
    }
  });

  test("the source is asked once", async ({ page }) => {
    const queries = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
    expect(queries).toHaveLength(1);
    expect(queries[0].sort).toEqual([{ field: "id", direction: "asc" }]);
  });

  test("v-model:view holds the view, and a saved one comes back", async ({ page }) => {
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

  test("a selection reaches Vue", async ({ page }) => {
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="2"][data-col="0"]')
        .focus(),
    );
    await page.keyboard.press(" ");
    await expect(page.locator("#selected")).toHaveText("1 rows selected");
  });

  test("KeepAlive: away and back, the same grid, and nothing asked", async ({ page }) => {
    await sortByAmount(page);
    await expect.poll(() => ariaSort(page, 3)).toBe("ascending");
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="2"][data-col="0"]')
        .focus(),
    );
    await page.keyboard.press(" ");
    await expect(page.locator("#selected")).toHaveText("1 rows selected");
    const selected = await selectedRows(page);
    const status = await page.evaluate(
      () =>
        document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
          .textContent,
    );
    const queries = await page.evaluate(() => window.__queries.length);

    await page.getByRole("button", { name: "Other" }).click();
    await expect(page.locator("opengrid-grid")).toHaveCount(0);
    // Long enough for the grid to let go of its DOM (plan point 74).
    await page.waitForTimeout(300);
    await page.getByRole("button", { name: "Orders" }).click();
    await rows(page);
    await page.waitForTimeout(250);

    expect(await ariaSort(page, 3)).toBe("ascending");
    expect(await selectedRows(page)).toEqual(selected);
    await expect(page.locator("#selected")).toHaveText("1 rows selected");
    expect(
      await page.evaluate(
        () =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
            .textContent,
      ),
    ).toBe(status);
    expect(await page.evaluate(() => window.__queries.length)).toBe(queries);
  });

  test("a grid taken out for good is collected; one put back asks once", async ({ page }) => {
    await page.evaluate(() => {
      window.__weak = new WeakRef(document.querySelector("opengrid-grid"));
    });
    await page.getByRole("button", { name: "Leave the page" }).click();
    await expect(page.locator("opengrid-grid")).toHaveCount(0);
    // A development build of Vue buffers its devtools events — component
    // instances among them — for three seconds after start when no devtools
    // are installed. Not ours to hold; past that, the grid has to go.
    await page.waitForTimeout(3500);

    let alive = true;
    for (let round = 0; round < 20 && alive; round += 1) {
      await page.evaluate(() => window.gc());
      await page.waitForTimeout(50);
      alive = await page.evaluate(() => window.__weak.deref() !== undefined);
    }
    expect(alive).toBe(false);

    await page.evaluate(() => {
      window.__queries.length = 0;
    });
    await page.getByRole("button", { name: "Come back" }).click();
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
  // In Node: no window, no document, no customElements. The script lives in the
  // example, where `vue` and the adapter resolve.
  const html = execFileSync("node", ["examples/vue/ssr.mjs"], {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    encoding: "utf8",
  });
  // Vue writes a boolean attribute bare (`selection`), React as `selection=""`:
  // the same attribute.
  expect(html).toBe(
    '<opengrid-grid label="Orders" datasource="orders" columns="id,customer" window-size="40" selection class="orders"></opengrid-grid>',
  );
});
