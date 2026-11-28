import { test, expect } from "@playwright/test";
import { shadow } from "./helpers.js";

// A click on a column header sorts (issue #561): the pointer path to what
// Enter does, `Shift` adds a key. Real mouse clicks, not `element.click()`.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-actions.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

const header = (page, name) => page.locator("opengrid-grid").getByRole("columnheader", { name, exact: false });

const sorts = (page) =>
  shadow(page, (root) =>
    Object.fromEntries(
      [...root.querySelectorAll('thead th[part="header"]')].map((th) => [
        th.querySelector("span").textContent.trim(),
        th.getAttribute("aria-sort"),
      ]),
    ));

test("a click sorts ascending, then descending, then back to the first column", async ({ page }) => {
  await open(page);
  await header(page, "Customer").click();
  expect(await sorts(page)).toMatchObject({ customer: "ascending", id: "none" });
  await header(page, "Customer").click();
  expect(await sorts(page)).toMatchObject({ customer: "descending" });
  await header(page, "Customer").click();
  expect(await sorts(page)).toMatchObject({ customer: "none", id: "ascending" });
});

test("Shift+click adds a second key", async ({ page }) => {
  await open(page);
  await header(page, "Customer").click();
  await header(page, "Amount").click({ modifiers: ["Shift"] });
  // The second key shows its rank; `aria-sort` names the primary one.
  const ranks = await shadow(page, (root) =>
    [...root.querySelectorAll('thead th[part="header"] [part="sort-index"]')].map((mark) => mark.textContent));
  expect(ranks.filter((rank) => rank !== "")).toEqual(["1", "2"]);
});

test("a click on an action's header sorts nothing", async ({ page }) => {
  await open(page);
  await header(page, "Open").click();
  expect(await sorts(page)).toMatchObject({ id: "ascending" });
});
