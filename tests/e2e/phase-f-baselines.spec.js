import { test, expect } from "@playwright/test";

// Screenshot baselines for Phase F (plan point 70), decided 2026-09-24:
// **covered, not crossed.** Every state once in the base look at normal
// density, every look once and every density once in the plain rows state —
// eleven pictures instead of seventy-five. A cross product would catch no
// failure class these do not, and axe already sees every look in every state
// that matters (prototype.spec.js).
//
// Only the element is photographed: the page around it is the example's, and a
// baseline of it would fail on every change to the example.

const URL = "/examples/prototype/index.html";

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

async function look(page, preset) {
  await page.locator(`label[for="preset-${preset}"]`).click();
}

const STATES = {
  rows: async () => {},
  grouped: async (page) => {
    await page.getByRole("tab", { name: "Umsatz nach Land" }).click();
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="total"]'));
  },
  "menu-open": async (page) => {
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="column-menu-button"]')[1].click(),
    );
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="column-menu"]:not([hidden])'));
  },
  "suggestions-open": async (page) => {
    await page.locator("opengrid-grid").locator('[part="search-input"]').focus();
    await page.keyboard.type("cou");
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="search-list"]:not([hidden]) [role="option"]'));
  },
  empty: async (page) => {
    await page.evaluate(() =>
      window.__opengrid.set_view(document.querySelector("opengrid-grid"), {
        sort: [{ field: "id", direction: "asc" }],
        filters: [{ column: "country", op: "eq", value: "XX" }],
      }),
    );
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="empty"]:not([hidden])'));
  },
};

test.beforeEach(async ({ page }) => {
  await page.goto(URL);
  await page.waitForFunction(() => window.__opengridReady && window.__opengrid);
  await page.evaluate(() => window.__opengridReady);
  await expect.poll(() => status(page)).toBe("50 Treffer");
});

// Every state, base look, normal density.
for (const [state, reach] of Object.entries(STATES)) {
  test(`state ${state}`, async ({ page }) => {
    await reach(page);
    await expect(page.locator("opengrid-grid")).toHaveScreenshot(`phase-f-state-${state}.png`);
  });
}

// Every other look, rows state.
for (const preset of ["paper", "violet", "orange", "dark"]) {
  test(`look ${preset}`, async ({ page }) => {
    await look(page, preset);
    await expect(page.locator("opengrid-grid")).toHaveScreenshot(`phase-f-look-${preset}.png`);
  });
}

// The other two densities, base look, rows state.
for (const density of ["compact", "comfortable"]) {
  test(`density ${density}`, async ({ page }) => {
    await page.evaluate(
      (density) => document.querySelector("opengrid-grid").shadowRoot.querySelector(`[data-density="${density}"]`).click(),
      density,
    );
    await expect.poll(() => page.evaluate(() => document.querySelector("opengrid-grid").getAttribute("density"))).toBe(density);
    await expect(page.locator("opengrid-grid")).toHaveScreenshot(`phase-f-density-${density}.png`);
  });
}
