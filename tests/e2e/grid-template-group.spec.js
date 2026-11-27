import { test, expect } from "@playwright/test";
import { shadow } from "./helpers.js";

// Cell templates in a grouped grid (issue #533): the data rows are templated as
// in a flat grid; the group headers and the total are not — an aggregate is not
// a record.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-template-group.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });

/** Opens (or closes) the group at a display position. */
async function toggle(page, row) {
  await focusCell(page, row, 0);
  await page.keyboard.press("Enter");
}

const where = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active ? `${active.tagName.toLowerCase()}:${active.dataset.row ?? ""}:${active.dataset.col ?? ""}` : null;
  });

/** Every drawn row: its kind, and for each cell whether it holds a node. */
const rows = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.style.display)
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .map((tr) => ({
        kind: tr.dataset.kind ?? "row",
        cells: [...tr.querySelectorAll("td[data-col]")].map((td) => ({
          col: td.dataset.col,
          node: td.childElementCount > 0,
          template: td.hasAttribute("data-template"),
          text: td.textContent,
        })),
      })),
  );

const bare = (row) => row.cells.every((cell) => !cell.node && !cell.template);

test("data rows are templated, group headers and the total are not", async ({ page }) => {
  await open(page);
  // Closed: three headers and the total, no template anywhere.
  let drawn = await rows(page);
  expect(drawn.map((row) => row.kind)).toEqual(["group", "group", "group", "total"]);
  expect(drawn.every(bare)).toBe(true);

  await toggle(page, 0);
  await expect.poll(async () => (await rows(page)).some((row) => row.kind === "row")).toBe(true);
  drawn = await rows(page);
  const data = drawn.filter((row) => row.kind === "row");
  expect(data.length).toBeGreaterThan(1);
  for (const row of data) {
    // name, price, change, status — columns 2 to 5 — carry the page's node; id and section do not.
    expect(row.cells.map((cell) => cell.template)).toEqual([false, false, true, true, true, true]);
    expect(row.cells.map((cell) => cell.node)).toEqual([false, false, true, true, true, true]);
  }
  // The three columns of the price list: sign and symbol, not only colour.
  const first = await shadow(page, (root) => {
    const tr = [...root.querySelectorAll("tbody tr:not([data-kind])")].find((tr) => tr.querySelector('td[data-col="0"]').textContent === "1");
    return [...tr.querySelectorAll("td")].map((td) => td.innerHTML).join("|");
  });
  expect(first).toContain('<span class="deviation" data-id="1">▼ −1.00</span>');
  expect(first).toContain('<span class="change" data-id="1">▲ +0.30</span>');
  expect(first).toContain('<span class="badge badge-planned">planned</span>');
  // The headers and the total around them stay bare.
  expect(drawn.filter((row) => row.kind !== "row").every(bare)).toBe(true);
});

test("scrolling across groups redraws: no cell shows another row's node", async ({ page }) => {
  await open(page);
  // Last group first, so the positions of the others do not move.
  await toggle(page, 2);
  await toggle(page, 1);
  await toggle(page, 0);
  await expect.poll(async () => (await rows(page)).filter((row) => row.kind === "row").length).toBeGreaterThan(3);
  await focusCell(page, 1, 2);
  const seen = new Set();
  const stale = [];
  for (let step = 0; step < 40; step++) {
    await page.keyboard.press("ArrowDown");
    const state = await shadow(page, (root) =>
      [...root.querySelectorAll("tbody tr")]
        .filter((tr) => tr.style.transform && !tr.style.display)
        .map((tr) => {
          const kind = tr.dataset.kind ?? "row";
          const id = tr.querySelector('td[data-col="0"]').textContent;
          const bad = [...tr.querySelectorAll("td[data-col]")].filter((td) => {
            if (kind !== "row") return td.hasAttribute("data-template") || td.childElementCount > 0;
            // A row the window has not loaded yet has no id to draw from.
            if (id === "" || !["2", "3", "4", "5"].includes(td.dataset.col)) return false;
            const node = td.querySelector("[data-id], .badge");
            return !node || (node.dataset.id !== undefined && node.dataset.id !== id);
          }).length;
          return { kind, id, bad };
        }),
    );
    for (const row of state) {
      if (row.kind === "row") seen.add(row.id);
      if (row.bad) stale.push(row);
    }
  }
  expect(stale).toEqual([]);
  // The walk crossed the groups: rows of all three sections were in view.
  expect(seen.size).toBeGreaterThan(20);
  // The last position is the total, still untemplated.
  await expect.poll(async () => (await rows(page)).some((row) => row.kind === "total")).toBe(true);
  expect(bare((await rows(page)).find((row) => row.kind === "total"))).toBe(true);
  // At rest nothing is built again.
  let built = -1;
  await expect
    .poll(async () => {
      const now = await page.evaluate(() => window.__built);
      const settled = now === built;
      built = now;
      return settled;
    }, { intervals: [250] })
    .toBe(true);
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => window.__built)).toBe(built);
});

test("Enter goes into the link of a grouped row, Escape comes back", async ({ page }) => {
  await open(page);
  await toggle(page, 0);
  await expect.poll(async () => (await rows(page)).some((row) => row.kind === "row")).toBe(true);
  await focusCell(page, 1, 2);
  await page.keyboard.press("Enter");
  expect(await where(page)).toBe("a::");
  await page.keyboard.press("Escape");
  expect(await where(page)).toBe("td:1:2");
  expect(await shadow(page, (root) => root.querySelector('td[data-row="1"][data-col="2"] a').tabIndex)).toBe(-1);
  await page.keyboard.press("ArrowDown");
  expect(await where(page)).toBe("td:2:2");
});

test("closing a group puts the headers back to bare text", async ({ page }) => {
  await open(page);
  await toggle(page, 0);
  await expect.poll(async () => (await rows(page)).some((row) => row.kind === "row")).toBe(true);
  await toggle(page, 0);
  await expect.poll(async () => (await rows(page)).map((row) => row.kind)).toEqual(["group", "group", "group", "total"]);
  expect((await rows(page)).every(bare)).toBe(true);
});
