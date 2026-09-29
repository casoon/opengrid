import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-table>` as the first element through the real loader (plan point 46).
//
// This is the infrastructure proof: the element registers via loader.js + WASM,
// renders into an open shadow root, mirrors the host label, passes axe, and has a
// visual baseline. Later UI points add their own specs here.

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/table.html");
  // loader.js registers asynchronously; the shadow root is the signal that the
  // WASM module loaded and connectedCallback ran.
  await page.waitForFunction(
    () =>
      customElements.get("opengrid-table") &&
      document.querySelector("opengrid-table")?.shadowRoot?.querySelector("table"),
  );
});

test("registers, renders into an open shadow root and mirrors the label", async ({
  page,
}) => {
  const host = page.locator("opengrid-table");
  await expect(host).toHaveCount(1);

  const facts = await host.evaluate((element) => {
    const root = element.shadowRoot;
    return {
      mode: root?.mode,
      hasTable: !!root?.querySelector("table"),
      ariaLabel: root?.querySelector("table")?.getAttribute("aria-label"),
      caption: root?.querySelector("caption")?.textContent,
    };
  });

  expect(facts).toEqual({
    mode: "open",
    hasTable: true,
    ariaLabel: "Bestellungen",
    caption: "Bestellungen",
  });
});

test("has no axe violations", async ({ page }) => {
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});

test("matches the visual baseline", async ({ page, browserName }) => {
  test.skip(browserName !== "chromium", "Screenshot baselines are Chromium's (tests/e2e/playwright.config.js)");
  await expect(page).toHaveScreenshot("table-fixture.png", {
    maxDiffPixelRatio: 0.02,
  });
});

// Issue #86: a table waiting for its rows said nothing — a screen reader met a
// table without rows and could not tell "loading" from "empty".
test("a table without its answer yet is busy", async ({ page }) => {
  const busy = await page.evaluate(
    () => document.querySelector("opengrid-table").shadowRoot.querySelector("table").getAttribute("aria-busy"),
  );
  expect(busy).toBe("true");
});

test("a table with its answer is no longer busy", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/table-data.html");
  await page.evaluate(() => window.__opengridReady);
  await expect
    .poll(() =>
      page.evaluate(() => document.querySelector("opengrid-table").shadowRoot.querySelectorAll("tbody tr").length),
    )
    .toBeGreaterThan(0);
  const busy = await page.evaluate(() =>
    document.querySelector("opengrid-table").shadowRoot.querySelector("table").hasAttribute("aria-busy"),
  );
  expect(busy).toBe(false);
});
