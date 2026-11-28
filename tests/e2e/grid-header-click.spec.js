import { test, expect } from "@playwright/test";
import { shadow, status } from "./helpers.js";

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

// Header affordances (issue #570).

test("a header shows the pointer cursor, and a cut-off label gets its title on hover", async ({ page }) => {
  await open(page);
  const cursor = await shadow(page, (root) => getComputedStyle(root.querySelector('thead th[part="header"]')).cursor);
  expect(cursor).toBe("pointer");

  const label = (page) => shadow(page, (root) => root.querySelector('thead th[data-col="1"] > span').getAttribute("title"));
  await shadow(page, (root) => (root.querySelector('thead th[data-col="1"] > span').style.maxWidth = "14px"));
  await header(page, "customer").hover();
  expect(await label(page)).toBe("customer");

  await shadow(page, (root) => (root.querySelector('thead th[data-col="1"] > span').style.maxWidth = "none"));
  await header(page, "amount").hover();
  await header(page, "customer").hover();
  expect(await label(page)).toBeNull();
});

// Resize by dragging the edge of a header (issue #563).

const widthOf = (page, col) =>
  shadow(page, (root, col) => Math.round(root.querySelector(`thead th[data-col="${col}"]`).getBoundingClientRect().width), col);

// The edge of a header: a few pixels in from its end, where a mouse finds it.
async function edgePoint(page, col) {
  const box = await page.locator(`opengrid-grid thead th[data-col="${col}"]`).boundingBox();
  return { x: box.x + box.width - 3, y: box.y + box.height / 2 };
}

async function dragEdge(page, col, by) {
  const { x, y } = await edgePoint(page, col);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + by / 2, y, { steps: 3 });
  await page.mouse.move(x + by, y, { steps: 3 });
  await page.mouse.up();
}

test("dragging the edge of a header changes the column's width and does not sort", async ({ page }, testInfo) => {
  // On the 480 px page the columns share what the table leaves them, so a
  // width does not grow by what the pointer moved; the drag is checked wide.
  test.skip(testInfo.project.name === "narrow", "the columns share the width there");
  await open(page);
  const before = await widthOf(page, 1);
  await dragEdge(page, 1, 60);
  // The measured width is a fraction of a pixel, the drag whole ones.
  expect(Math.abs((await widthOf(page, 1)) - (before + 60))).toBeLessThanOrEqual(1);
  expect(await sorts(page)).toMatchObject({ id: "ascending", customer: "none" });
  expect(await status(page)).toContain("customer");

  await dragEdge(page, 1, -40);
  expect(Math.abs((await widthOf(page, 1)) - (before + 20))).toBeLessThanOrEqual(2);
});

test("a column cannot be dragged narrower than the least it is allowed", async ({ page }, testInfo) => {
  test.skip(testInfo.project.name === "narrow", "the columns share the width there");
  await open(page);
  const before = await widthOf(page, 1);
  await dragEdge(page, 1, -2000);
  const after = await widthOf(page, 1);
  expect(after).toBeLessThan(before);
  expect(after).toBeGreaterThanOrEqual(24);
});

test("the cursor says the edge drags, and a click on the edge does not sort", async ({ page }) => {
  await open(page);
  const { x, y } = await edgePoint(page, 1);
  await page.mouse.move(x, y);
  expect(await shadow(page, (root) => getComputedStyle(root.querySelector('thead th[data-col="1"]')).cursor)).toBe("col-resize");
  await page.mouse.click(x, y);
  expect(await sorts(page)).toMatchObject({ id: "ascending", customer: "none" });
  // Off the edge it is the plain header again.
  await page.mouse.move(x - 40, y);
  expect(await shadow(page, (root) => getComputedStyle(root.querySelector('thead th[data-col="1"]')).cursor)).not.toBe("col-resize");
});
