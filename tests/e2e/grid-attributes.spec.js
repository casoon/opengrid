import { test, expect } from "@playwright/test";

// What an attribute means by its value, not by being there (issue #233), and
// the two element rules that were told differently on the grid, the table and
// the pivot.

const shadow = (page, selector, fn, arg) =>
  page.evaluate(
    ({ selector, fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(document.querySelector(selector).shadowRoot, arg),
    { selector, fn: fn.toString(), arg },
  );
const status = (page) => shadow(page, "opengrid-grid", (root) => root.querySelector('[part="status"]').textContent);

test("page-size=0 is no paging: the grid still groups", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-group.html");
  await page.waitForFunction(() => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"));
  await page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    host.removeAttribute("group-by");
    host.setAttribute("page-size", "0");
    host.setAttribute("group-by", "country");
  });
  await expect
    .poll(() => shadow(page, "opengrid-grid", (root) => root.querySelectorAll("tr[aria-expanded]").length))
    .toBeGreaterThan(0);
  expect(await status(page)).not.toContain("Cannot group");
});

test("page-size=0 is no paging: the new row stays", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-add-row.html");
  await page.waitForFunction(() => window.__ready === true);
  const newRow = () => shadow(page, "opengrid-grid", (root) => !!root.querySelector("tr[data-new-row]"));
  await expect.poll(newRow).toBe(true);
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("page-size", "0"));
  // The change asks again and draws again; the row is still there after it.
  await page.waitForTimeout(300);
  expect(await newRow()).toBe(true);
  expect(await shadow(page, "opengrid-grid", (root) => root.querySelector("table").getAttribute("aria-rowcount"))).toBe("4");
});

test("a row-key that names no field offers no bulk edit", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-bulk.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("row-key", ""));
  await shadow(page, "opengrid-grid", (root) =>
    root.querySelector('tr[aria-rowindex="3"] td[data-select="row"]').focus());
  await page.keyboard.press(" ");
  await shadow(page, "opengrid-grid", (root) => root.querySelector('td[data-row="1"][data-col="3"]').focus());
  await page.keyboard.press("F2");
  expect(await shadow(page, "opengrid-grid", (root) => !!root.querySelector('[part="bulk"]'))).toBe(false);
});

test("set_texts adds to the texts an earlier call set", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-texts.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.evaluate(() => window.__setTexts({ clear: "Leeren" }));
  await page.evaluate(() => window.__setTexts({ filterGroup: "Filter hier" }));
  const words = () =>
    shadow(page, "opengrid-grid", (root) => ({
      clear: root.querySelector('[part="filter-clear"]')?.textContent,
      group: root.getElementById(root.querySelector('[part="filter"]').getAttribute("aria-labelledby"))?.textContent,
    }));
  await expect.poll(words).toEqual({ clear: "Leeren", group: "Filter hier" });
});

test("set_texts(host, null) takes the texts back to the defaults", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-texts.html");
  await page.waitForFunction(() => window.__opengridReady);
  const clear = () => shadow(page, "opengrid-grid", (root) => root.querySelector('[part="filter-clear"]')?.textContent);
  await page.evaluate(() => window.__setTexts({ clear: "Leeren" }));
  await expect.poll(clear).toBe("Leeren");
  await page.evaluate(() => window.__setTexts(null));
  await expect.poll(clear).toBe("Clear");
});

test("a pivot's label is mirrored, not asked again", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/pivot-local.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect
    .poll(() => shadow(page, "opengrid-pivot", (root) => root.querySelectorAll("tbody tr").length))
    .toBeGreaterThan(0);
  await page.evaluate(() => {
    window.__asked = 0;
    document.querySelector("opengrid-pivot").addEventListener("opengrid-query", () => window.__asked++);
    document.querySelector("opengrid-pivot").setAttribute("label", "Renamed");
  });
  await expect
    .poll(() => shadow(page, "opengrid-pivot", (root) => root.querySelector("table").getAttribute("aria-label")))
    .toBe("Renamed");
  expect(await shadow(page, "opengrid-pivot", (root) => root.querySelector("caption")?.textContent)).toBe("Renamed");
  // Give a wrongly asked query the time to answer.
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.__asked)).toBe(0);
});
