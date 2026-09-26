import { test, expect } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Hydration (plan point 81): the grid rendered on the server by each adapter,
// then hydrated in the browser by the same framework.
//
// The server HTML comes from the example's own ssr.mjs, rendered with the same
// props the page hydrates with (src/ssr-props.js). What has to hold:
//
// - the framework says nothing — no mismatch warning, no error;
// - it keeps the server's element instead of throwing it away and building
//   another (a mismatch it recovers from silently would do that);
// - the grid then fills, asking once, and still answers the reader.
//
// React 18 hydrates the HTML React 19 rendered: the markup is the same, and
// React 18 cannot render on the server here (in Node, `react-dom-18` resolves
// React 19 — see examples/react/vite.config.js).

const root = fileURLToPath(new URL("../..", import.meta.url));

/** The server HTML of an example, as its ssr.mjs prints it. */
function serverHtml(example) {
  return execFileSync("node", [`examples/${example}/ssr.mjs`], { cwd: root, encoding: "utf8" });
}

const CASES = [
  { name: "React 19", example: "react", page: "/examples/react/dist/hydrate.html" },
  { name: "React 18", example: "react", page: "/examples/react/dist-18/hydrate.html" },
  { name: "Vue", example: "vue", page: "/examples/vue/dist/hydrate.html", beforeDefine: true },
  { name: "Svelte", example: "svelte", page: "/examples/svelte/dist/hydrate.html", beforeDefine: true },
];

for (const { name, example, page: path, beforeDefine } of CASES) {
  test(`${name}: the server's grid is hydrated, kept, and filled`, async ({ page }) => {
    const html = serverHtml(example);
    const problems = [];
    page.on("pageerror", (error) => problems.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error" || message.type() === "warning") {
        problems.push(message.text());
      }
    });

    await page.goto(path);
    await page.waitForFunction(() => typeof window.__hydrate === "function");
    await page.evaluate((html) => window.__hydrate(html), html);
    await page.waitForFunction(
      () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
    );
    await page.waitForTimeout(250);

    // Hydrated before the element was defined — the order of a real page.
    if (beforeDefine) {
      expect(await page.evaluate(() => window.__definedAtHydration)).toBe(false);
    }

    // The same node the server sent: hydrated, not replaced.
    expect(
      await page.evaluate(() => document.querySelector("opengrid-grid") === window.__serverNode),
    ).toBe(true);
    const attributes = await page.evaluate(() =>
      Object.fromEntries(
        [...document.querySelector("opengrid-grid").attributes].map((a) => [a.name, a.value]),
      ),
    );
    expect(attributes).toMatchObject({
      label: "Orders",
      datasource: "orders",
      columns: "id,customer",
      "window-size": "40",
      selection: "",
      class: "orders",
    });
    expect(attributes).not.toHaveProperty("toolbar");
    expect(await page.evaluate(() => window.__queries.length)).toBe(1);
    // The texts arrived through connect after hydration: `lang` on our words.
    expect(
      await page.evaluate(() =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector('[part="status"]')
          .getAttribute("lang"),
      ),
    ).toBe("de");

    // And the connection answers the reader.
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector('th[data-col="1"]').focus(),
    );
    await page.keyboard.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(() =>
          document
            .querySelector("opengrid-grid")
            .shadowRoot.querySelector('th[data-col="1"]')
            .getAttribute("aria-sort"),
        ),
      )
      .toBe("ascending");

    expect(problems).toEqual([]);
  });
}
