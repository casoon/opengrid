import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Data without JavaScript (issue #84). The page writes its data as a plain
// table inside the element. Without script that is what everyone reads — a
// crawler, a reader mode, a text browser. Once the element has rendered, its
// shadow root replaces it, so nothing is read twice. And when the module cannot
// load, the page's table stays: the fallback slots it in.

const FIXTURE = "/tests/e2e/fixtures/light-dom.html";

/** The cells a reader can reach, by the text the page wrote. */
const pageCells = (page) => page.getByRole("cell", { name: /Aus dem HTML/ });

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("the page's own tables are what everyone reads", async ({ page }) => {
    await page.goto(FIXTURE);
    await expect(page.getByRole("table", { name: "Bestellungen" })).toBeVisible();
    await expect(page.getByRole("table", { name: "Raster" })).toBeVisible();
    await expect(pageCells(page)).toHaveCount(3);
    await expect(page.getByRole("columnheader", { name: "amount" })).toHaveCount(2);
  });

  // No axe here: axe is a script. The same tables are checked below, when the
  // module cannot load and the page shows them as they are.
});

test.describe("with the elements rendered", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(FIXTURE);
    await page.evaluate(() => window.__opengridReady);
    await expect
      .poll(() =>
        page.evaluate(
          () => document.querySelector("opengrid-table").shadowRoot?.querySelectorAll("tbody tr").length ?? 0,
        ),
      )
      .toBeGreaterThan(0);
  });

  test("the shadow root replaces the page's tables, and nothing is read twice", async ({ page }) => {
    await expect(pageCells(page)).toHaveCount(0);
    // One table per element: the element's own, with the engine's rows.
    await expect(page.getByRole("table", { name: "Bestellungen" })).toHaveCount(1);
    await expect(page.getByRole("grid", { name: "Raster" })).toHaveCount(1);
    await expect(page.getByRole("table", { name: "Raster" })).toHaveCount(0);
  });

  test("has no axe violations", async ({ page }) => {
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

test.describe("when the module cannot load", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(FIXTURE + "?fallback");
    expect((await page.evaluate(() => window.__opengridReady)).fallback).toBe(true);
  });

  test("the page's own tables stay readable", async ({ page }) => {
    // The stand-in draws no table of its own next to the page's: in WebKit even
    // an unrendered default content of a slot counted as a second table.
    const ownTable = await page.evaluate(
      () => !!document.querySelector("opengrid-table").shadowRoot.querySelector("table"),
    );
    expect(ownTable).toBe(false);
    await expect(page.getByRole("table", { name: "Bestellungen" })).toBeVisible();
    await expect(page.getByRole("table", { name: "Raster" })).toBeVisible();
    await expect(pageCells(page)).toHaveCount(3);
  });

  test("an element without a table of its own still shows the empty skeleton", async ({ page }) => {
    const skeleton = await page.evaluate(() => {
      const table = document.querySelector("#empty").shadowRoot.querySelector("table");
      return { caption: table.querySelector("caption").textContent, label: table.getAttribute("aria-label"), shown: table.checkVisibility() };
    });
    expect(skeleton).toEqual({ caption: "Leer", label: "Leer", shown: true });
  });

  test("has no axe violations", async ({ page }) => {
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});
