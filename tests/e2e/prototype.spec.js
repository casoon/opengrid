import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The host page of the design prototype (plan point 69).
//
// Everything this spec drives — the view tabs, the look studio — is page code
// in examples/prototype/index.html, built only from `get_view`/`set_view` and
// custom properties. If one of these tests needs element code to pass, the seam
// of point 59 or 57 is in the wrong place.

const URL = "/examples/prototype/index.html";
const PRESETS = ["base", "paper", "violet", "orange", "dark"];

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

async function view(page) {
  return page.evaluate(() => window.__opengrid.get_view(document.querySelector("opengrid-grid")));
}

test.beforeEach(async ({ page }) => {
  await page.goto(URL);
  await page.waitForFunction(() => window.__opengridReady && window.__opengrid);
  await page.evaluate(() => window.__opengridReady);
  await expect.poll(() => status(page)).toBe("50 Treffer");
});

test("switching a tab applies its view, and the grid follows", async ({ page }) => {
  await page.getByRole("tab", { name: "Deutschland ab 10 €" }).click();
  await expect.poll(() => status(page)).toBe("15 Treffer");
  expect((await view(page)).filters).toEqual([
    { column: "country", op: "eq", value: "DE" },
    { column: "amount", op: "gte", value: "10" },
  ]);

  await page.getByRole("tab", { name: "Umsatz nach Land" }).click();
  await expect
    .poll(() => page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector("table").getAttribute("role")))
    .toBe("treegrid");
  expect((await view(page)).group).toEqual(["country", "customer"]);

  await page.getByRole("tab", { name: "Alle" }).click();
  await expect.poll(() => status(page)).toBe("50 Treffer");
  expect((await view(page)).group).toEqual([]);
});

test("the tabs follow the ARIA tabs pattern", async ({ page }) => {
  const first = page.getByRole("tab", { name: "Alle" });
  await first.focus();
  await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab", { name: "Deutschland ab 10 €" })).toBeFocused();
  await expect(page.getByRole("tab", { name: "Deutschland ab 10 €" })).toHaveAttribute("aria-selected", "true");
  await expect.poll(() => status(page)).toBe("15 Treffer");
  await page.keyboard.press("End");
  await expect(page.getByRole("tab", { name: "Umsatz nach Land" })).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(first).toBeFocused();
  // One tab stop: the others are reached with the arrows.
  expect(await page.locator('[role="tab"][tabindex="0"]').count()).toBe(1);
});

test("a changed view is marked, and Verwerfen brings the saved one back", async ({ page }) => {
  await page.evaluate(() => {
    const box = [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('input[data-facet="customer"]')].find(
      (input) => input.dataset.key === '"Alpha"',
    );
    box.checked = true;
    box.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  });
  await expect.poll(() => status(page)).toBe("12 Treffer");
  await expect(page.getByRole("tab", { name: "Alle, geändert" })).toBeVisible();

  await page.getByRole("button", { name: "Verwerfen" }).click();
  await expect.poll(() => status(page)).toBe("50 Treffer");
  await expect(page.getByRole("tab", { name: "Alle", exact: true })).toBeVisible();
});

test("a saved view is the grid's view with a name, and survives a reload", async ({ page }) => {
  await page.getByRole("tab", { name: "Ohne Kunde" }).click();
  await expect.poll(() => status(page)).toBe("4 Treffer");
  await page.getByRole("button", { name: "+ Ansicht" }).click();
  await expect(page.getByRole("tab", { name: "Ansicht 5" })).toHaveAttribute("aria-selected", "true");

  await page.reload();
  await page.waitForFunction(() => window.__opengridReady && window.__opengrid);
  await page.getByRole("tab", { name: "Ansicht 5" }).click();
  await expect.poll(() => status(page)).toBe("4 Treffer");
});

test("a colour changed in the studio changes the grid", async ({ page }) => {
  await page.getByRole("button", { name: "Ausprägung anpassen" }).click();
  const background = () =>
    page.evaluate(() => getComputedStyle(document.querySelector("opengrid-grid")).backgroundColor);
  const before = await background();
  await page.locator("#color-surface").evaluate((input) => {
    input.value = "#ffe4c4";
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect.poll(background).toBe("rgb(255, 228, 196)");
  expect(before).not.toBe("rgb(255, 228, 196)");
  // A changed preset is a look of its own, and says so.
  await expect(page.locator("#preset-custom")).toBeChecked();
  await expect(page.locator("#css")).toContainText("--og-surface: #ffe4c4;");
});

test("theme.css names only opengrid-grid, and copying it is announced", async ({ page, context }) => {
  // Contradiction 3 of point 56: table and pivot ship no stylesheet.
  const css = await page.locator("#css").textContent();
  expect(css.match(/^[^\s].*\{$/gm)).toEqual(["opengrid-grid {"]);

  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.getByRole("button", { name: "Ausprägung anpassen" }).click();
  await page.getByRole("button", { name: "Kopieren" }).click();
  await expect(page.locator("#page-status")).toHaveText(/^theme\.css: /);
});

test("decimals are shown exactly, with a real minus", async ({ page }) => {
  // S8: `999999999.99` and `-0.01` are in the prototype's rows on purpose.
  const cells = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('td[data-col="3"]')].map((td) => td.textContent),
  );
  expect(cells).toContain("999.999.999,99");
  expect(cells).toContain("−0,01");
});

test("a group label is not styled like the column it sits in", async ({ page }) => {
  // The id column is monospaced and muted; the group label drawn in its cell
  // is neither.
  await page.getByRole("tab", { name: "Umsatz nach Land" }).click();
  await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="group"]'));
  const [label, value] = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [
      getComputedStyle(root.querySelector('tr[data-kind="group"] td[data-col="0"]')).fontFamily,
      getComputedStyle(root.querySelector('tr[data-kind="group"] td[data-col="1"]')).fontFamily,
    ];
  });
  expect(label).toBe(value);
});

for (const preset of PRESETS) {
  test(`has no axe violations in the ${preset} preset`, async ({ page }) => {
    await page.locator(`label[for="preset-${preset}"]`).click();
    await page.getByRole("button", { name: "Ausprägung anpassen" }).click();
    await page.getByRole("tab", { name: "Umsatz nach Land" }).click();
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="group"]'));
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
}

for (const preset of ["base", "dark"]) {
  test(`looks like the prototype in the ${preset} preset`, async ({ page }) => {
    await page.locator(`label[for="preset-${preset}"]`).click();
    await page.getByRole("tab", { name: "Umsatz nach Land" }).click();
    await page.waitForFunction(() => !!document.querySelector("opengrid-grid").shadowRoot.querySelector('tr[data-kind="group"]'));
    await expect(page).toHaveScreenshot(`prototype-${preset}.png`, { fullPage: true });
  });
}
