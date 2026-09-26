import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// The React adapter (plan point 77), against the built example — twice: once
// with React 19 (`dist/`), once with React 18 (`dist-18/`), because the
// adapter promises both and the two treat custom elements differently.
//
// Both builds are development builds, so StrictMode really mounts the grid
// twice: the tests that count queries count through that.

test.use({ launchOptions: { args: ["--js-flags=--expose-gc"] } });

const BUILDS = [
  { react: "19", path: "/examples/react/dist/index.html" },
  { react: "18", path: "/examples/react/dist-18/index.html" },
];

/** Waits until the grid shows rows. */
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

for (const build of BUILDS) {
  test.describe(`React ${build.react}`, () => {
    let problems;

    test.beforeEach(async ({ page }) => {
      // React says what it dislikes on the console — an unknown DOM property,
      // a hook called wrongly, a key warning. Any of it is a failure here.
      problems = [];
      page.on("pageerror", (error) => problems.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error" || message.type() === "warning") {
          problems.push(message.text());
        }
      });
      await page.goto(build.path);
      await rows(page);
      await page.waitForTimeout(250);
    });

    test.afterEach(() => {
      expect(problems).toEqual([]);
    });

    test("it is the React this build says it is", async ({ page }) => {
      await expect(page.locator("h1")).toContainText(`React ${build.react}.`);
    });

    test("the attributes arrive as HTML has them", async ({ page }) => {
      const attributes = await page.evaluate(() => {
        const grid = document.querySelector("opengrid-grid");
        return Object.fromEntries([...grid.attributes].map((a) => [a.name, a.value]));
      });
      expect(attributes).toMatchObject({
        label: "Orders",
        datasource: "orders",
        columns: "id,customer,country,amount,qty",
        "window-size": "40",
        selection: "",
        toolbar: "",
        class: "orders",
      });
      // Nothing React-shaped leaks onto the element.
      expect(Object.keys(attributes)).not.toContain("classname");
      expect(Object.keys(attributes)).not.toContain("windowsize");
      expect(Object.keys(attributes)).not.toContain("provider");
    });

    test("StrictMode mounts twice, and the source is asked once", async ({ page }) => {
      // Twice — so this is the double mount, not a production build.
      expect(await page.evaluate(() => window.__mounts)).toBe(2);
      const queries = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
      expect(queries).toHaveLength(1);
      expect(queries[0].sort).toEqual([{ field: "id", direction: "asc" }]);
      const status = await page.evaluate(
        () =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]')
            .textContent,
      );
      expect(status).toMatch(/matches$/);
    });

    test("the view lives in React state, and a saved one comes back", async ({ page }) => {
      await page.evaluate(() => {
        window.__queries.length = 0;
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector('th[data-col="3"]')
          .focus();
      });
      await page.keyboard.press("Enter");
      await expect
        .poll(() => page.evaluate(() => window.__view.sort))
        .toEqual([{ field: "amount", direction: "asc" }]);
      await page.waitForTimeout(250);
      // Reported, written into state, handed back: once, not in a loop.
      expect(await page.evaluate(() => window.__queries.length)).toBe(1);

      await page.getByRole("button", { name: "Restore the saved view" }).click();
      await expect.poll(() => ariaSort(page, 1)).toBe("ascending");
      expect(await ariaSort(page, 3)).toBe("none");
      await page.waitForTimeout(250);
      expect(await page.evaluate(() => window.__queries.length)).toBe(2);
    });

    test("the ref is the element", async ({ page }) => {
      expect(await page.evaluate(() => window.__grid.current === document.querySelector("opengrid-grid"))).toBe(
        true,
      );
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

    test("hidden hides the grid when true, and only then", async ({ page }) => {
      // React 18 wrote hidden="false" for false — hidden. React 19 sets the
      // property for a name the element has, so "" for true would be falsy —
      // shown. Both are checked on the element React rendered.
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

    test("a selection reaches React", async ({ page }) => {
      await page.evaluate(() =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector('td[data-row="2"][data-col="0"]')
          .focus(),
      );
      await page.keyboard.press(" ");
      await expect(page.locator("#selected")).toHaveText("1 rows selected");
    });

    test("a grid taken out by React is collected; one put back asks once", async ({
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
}

test("the adapter renders on the server", () => {
  // In Node: no window, no document, no customElements. What comes out is the
  // element with its attributes — labelled before any script runs. The script
  // lives in the example, where `react` and the adapter resolve.
  const html = execFileSync("node", ["examples/react/ssr.mjs"], {
    cwd: fileURLToPath(new URL("../..", import.meta.url)),
    encoding: "utf8",
  });
  expect(html).toBe(
    '<opengrid-grid label="Orders" datasource="orders" columns="id,customer" window-size="40" selection="" class="orders"></opengrid-grid>',
  );
});
