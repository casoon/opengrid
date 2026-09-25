import { test, expect } from "@playwright/test";

// The packed npm package, not the repository (plan point 40).
//
// Everything the page imports comes from `target/npm-package/package`, which
// `bash xtask/pack-npm.sh` produced with `npm pack` and unpacked again. That
// makes `package.json#files` a tested promise rather than a hopeful list: a
// forgotten file cannot load, and the elements never register.

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/packaged.html");
  await page.waitForFunction(() => window.__opengridReady);
  // Rejects if the DOM fallback ran, i.e. the packaged WASM module was missing.
  await page.evaluate(() => window.__opengridReady);
});

test("the packed package registers all three elements", async ({ page }) => {
  const defined = await page.evaluate(() =>
    ["opengrid-table", "opengrid-grid", "opengrid-pivot"].map((tag) =>
      Boolean(customElements.get(tag)),
    ),
  );
  expect(defined).toEqual([true, true, true]);
});

test("a grid loaded from the package renders its rows", async ({ page }) => {
  const grid = page.locator("opengrid-grid");
  await expect(grid.locator("tbody td[data-row]").first()).toBeVisible();

  const shape = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      status: root.querySelector('[part="status"]').textContent,
      columns: [...root.querySelectorAll("thead th")].map((th) =>
        th.textContent.replace(/[▲▼]/g, "").trim(),
      ),
      // The one-module decision (E25): the same package carries the pivot.
      pivotDefined: Boolean(customElements.get("opengrid-pivot")),
    };
  });
  expect(shape.status).toBe("5 matches");
  expect(shape.columns).toEqual(["id", "customer", "amount", "qty"]);
  expect(shape.pivotDefined).toBe(true);
});

test("the manifest promises what a consumer needs", async ({ page }) => {
  const manifest = await page.evaluate(() => window.__manifest);
  expect(manifest.name).toBe("@casoon/opengrid");
  expect(manifest.type).toBe("module");
  // E1: the scope and the element names are the published identity.
  expect(manifest.license).toBe("MIT OR Apache-2.0");
  // The loader resolves `./pkg/...` against `import.meta.url`, so a bundler
  // has to be allowed through the exports map as well.
  // Point 75. TypeScript would find `loader.d.ts` beside `loader.js` without
  // the conditions; they are here so a resolver that reads only `exports` finds
  // it too, and this is the test that holds their shape.
  expect(manifest.exports["."]).toEqual({ types: "./loader.d.ts", default: "./loader.js" });
  expect(manifest.exports["./loader.js"]).toEqual({
    types: "./loader.d.ts",
    default: "./loader.js",
  });
  expect(manifest.types).toBe("./loader.d.ts");
  expect(manifest.exports["./pkg/*"]).toBe("./pkg/*");
  // `worker.js` installs an `onmessage` handler at import time; the rest of the
  // package is side-effect free and may be tree-shaken.
  expect(manifest.sideEffects).toEqual(["./worker.js"]);
});

test("every import path the docs promise resolves in the package", async ({
  page,
  baseURL,
}) => {
  // README.md and docs/api.md both show `@casoon/opengrid/loader.js`. A subpath
  // that is not in the `exports` map does not resolve, however present the file
  // is — so the documentation is the source of this list, not a copy of it.
  const documented = new Set();
  for (const doc of ["/README.md", "/docs/api.md"]) {
    const text = await (await page.request.get(`${baseURL}${doc}`)).text();
    for (const [, path] of text.matchAll(/["']@casoon\/opengrid(\/[^"']*)?["']/g)) {
      documented.add(path ? `.${path}` : ".");
    }
  }
  expect(documented.size, "the docs show at least one import").toBeGreaterThan(0);

  const manifest = await page.evaluate(() => window.__manifest);
  // `toHaveProperty` would read the dots in "./loader.js" as a nested path, so
  // the keys are compared directly.
  const exported = Object.keys(manifest.exports);
  for (const path of documented) {
    expect(exported, `${path} is exported`).toContain(path);
  }
});

test("the package leaks nothing about the machine that built it", async ({
  page,
  baseURL,
}) => {
  // Panic locations are baked into a release build as **absolute** paths, so an
  // unremapped module carries the build machine's home directory to everyone
  // who installs it. Measured before `justfile`'s REMAP existed: 27 of them,
  // e.g. "/Users/<name>/.rustup/toolchains/...". Useless to a consumer and
  // nobody's business.
  const files = [
    "pkg/opengrid_web_components_bg.wasm",
    "pkg/opengrid_web_components.js",
    "loader.js",
    "worker.js",
  ];
  for (const file of files) {
    const response = await page.request.get(
      `${baseURL}/target/npm-package/package/${file}`,
    );
    expect(response.status(), file).toBe(200);
    const body = await response.body();
    const text = body.toString("latin1");
    for (const pattern of [/\/Users\//, /\/home\/[a-z]/, /C:\\Users\\/i]) {
      expect(pattern.test(text), `${file} contains a home directory`).toBe(false);
    }
  }
});

test("the package says where it comes from", async ({ page }) => {
  const manifest = await page.evaluate(() => window.__manifest);
  // A package with no link to its source is a dead end for anyone who wants to
  // read it, file something or check what they are running.
  expect(manifest.repository.url).toContain("github.com/casoon/opengrid");
  expect(manifest.homepage).toContain("github.com/casoon/opengrid");
  expect(manifest.bugs.url).toContain("github.com/casoon/opengrid");
  expect(manifest.files).toContain("CHANGELOG.md");
});

test("the package carries both licences", async ({ page, baseURL }) => {
  for (const file of ["LICENSE-MIT", "LICENSE-APACHE"]) {
    const response = await page.request.get(
      `${baseURL}/target/npm-package/package/${file}`,
    );
    expect(response.status(), `${file} is in the tarball`).toBe(200);
    expect((await response.text()).length).toBeGreaterThan(500);
  }
});
