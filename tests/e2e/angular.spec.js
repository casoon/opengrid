import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { readFileSync } from "node:fs";

// Angular (plan point 81) — no adapter (E32), the directive recipe from
// docs/guides/frameworks.md in a real Angular 22 app, AOT-compiled and zoneless:
// `[(view)]`, the element's own event bound in the template, an input taken
// away, and a grid that `@if` takes out and puts back. A development build, so
// Angular's warnings exist, and every test fails on any of them.

test.use({ launchOptions: { args: ["--js-flags=--expose-gc"] } });

const PAGE = "/examples/angular/dist/browser/index.html";

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

  test("[(view)] holds the view, and a saved one comes back", async ({ page }) => {
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

  test("a selection reaches Angular", async ({ page }) => {
    await page.evaluate(() =>
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('td[data-row="2"][data-col="0"]')
        .focus(),
    );
    await page.keyboard.press(" ");
    await expect(page.locator("#selected")).toHaveText("1 rows selected");
  });

  test("one-way [view]: the reader's change survives another input's change", async ({
    page,
  }) => {
    // The directive passes on only what changed; passing the unchanged view
    // along with the texts would put the page's view back over the reader's.
    await page.goto(`${PAGE}?one-way`);
    await rows(page);
    await sortByAmount(page);
    await expect.poll(() => ariaSort(page, 3)).toBe("ascending");
    await page.getByRole("button", { name: "German" }).click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
              .textContent,
        ),
      )
      .toMatch(/Treffer$/);
    expect(await ariaSort(page, 3)).toBe("ascending");
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

test("the directive in the app is the recipe in the docs", () => {
  // The recipe is what a reader copies; this app is what runs. They are the
  // same text, or the test that ran proves nothing about the recipe.
  const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");
  const docs = read("docs/guides/frameworks.md");
  const recipe = docs.split("### Angular")[1].split("```ts\n")[1].split("```")[0];
  expect(recipe).toBe(read("examples/angular/src/app/opengrid.directive.ts"));
});
