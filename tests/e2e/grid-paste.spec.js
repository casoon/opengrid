import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Pasting a block from a spreadsheet (issue #184): anchored at the focused
// cell, shown before it happens, applied with Enter, dropped with Escape.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);
const summary = (page) =>
  shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent ?? null);
const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-paste.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

/** What a spreadsheet puts on the clipboard, pasted into the focused cell. */
async function paste(page, text) {
  await page.evaluate((text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    // Firefox ignores `clipboardData` here and builds the event's own from its
    // `dataType` and `data` instead; the other engines ignore those two.
    const event = new ClipboardEvent("paste", {
      clipboardData: data, dataType: "text/plain", data: text, bubbles: true, cancelable: true, composed: true,
    });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
  }, text);
}

const block = (rows, make) =>
  Array.from({ length: rows }, (_, index) => make(index).join("\t")).join("\r\n") + "\r\n";

test("30 rows of 4 cells are shown first, applied by Enter as one event of 120 changes", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 1);
  await paste(page, block(30, (i) => [`New ${i + 1}`, "1.234,50", String(100 + i), `note ${i + 1}`]));

  // Nothing has changed yet; the preview says what would.
  await expect.poll(() => summary(page)).toBe("30 rows");
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await cell(page, 0, 1)).toBe("Item 1");
  expect(await status(page)).toContain("30 rows");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);

  await page.keyboard.press("Enter");
  const pasted = await page.evaluate(() => window.__pasted);
  expect(pasted).toHaveLength(1);
  expect(pasted[0].changes).toHaveLength(120);
  expect(pasted[0].added).toEqual([]);
  // Records by key, a German decimal as the exact decimal it is.
  expect(pasted[0].changes[0]).toMatchObject({ key: 1, row: 0, column: "name", value: "New 1", previous: "Item 1" });
  expect(pasted[0].changes[1]).toMatchObject({ key: 1, column: "price", value: "1234.50", previous: "1.00" });
  expect(pasted[0].changes[119]).toMatchObject({ key: 30, column: "note", value: "note 30" });
  // The loaded rows show it, marked unsaved, and the preview is gone.
  expect(await cell(page, 0, 1)).toContain("New 1");
  expect(await shadow(page, (root) => root.querySelector('[part="paste-preview"]'))).toBeNull();
  expect(await status(page)).toContain("120 values pasted");
});

test("a value that cannot be read is marked with its reason, and the paste is not applied", async ({ page }) => {
  await open(page);
  await focusCell(page, 2, 1);
  await paste(page, "A\tabc\nB\t5\n");
  await expect.poll(() => summary(page)).toBe("2 rows, 1 value not readable");
  const mark = await shadow(page, (root) => {
    const bad = root.querySelector('[part="paste-preview"] td[aria-invalid="true"]');
    return bad?.textContent;
  });
  // The price column is a number; "abc" is not one.
  expect(mark).toContain("“abc” cannot be read as number");

  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await status(page)).toContain("Not pasted");
  expect(await summary(page)).not.toBeNull();

  await page.keyboard.press("Escape");
  expect(await summary(page)).toBeNull();
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await status(page)).toContain("Paste discarded");
  // The focus is back on the cell the paste started from.
  expect(
    await shadow(page, (root) => {
      const active = root.activeElement;
      return `${active?.getAttribute("data-row")}:${active?.getAttribute("data-col")}`;
    }),
  ).toBe("2:1");
  expect(await cell(page, 2, 1)).toBe("Item 3");
});

const activeAt = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active?.getAttribute("data-row") ? `${active.getAttribute("data-row")}:${active.getAttribute("data-col")}` : null;
  });

/** Goes to the name column of the row `up` above the last one, by the keyboard (the pool holds only a window). */
async function goToRow(page, last, up) {
  await focusCell(page, 0, 1);
  await page.keyboard.press("Control+End");
  // The window follows; the key after it needs the cell in place.
  await expect.poll(() => activeAt(page)).toBe(`${last}:4`);
  await page.keyboard.press("Home");
  await expect.poll(() => activeAt(page)).toBe(`${last}:0`);
  for (let i = 0; i < up; i++) {
    await page.keyboard.press("ArrowUp");
  }
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => activeAt(page)).toBe(`${last - up}:1`);
}

test("rows past the end are cut off without add-row", async ({ page }) => {
  await open(page);
  // The last row is 39; the block starts one above it.
  await goToRow(page, 39, 1);
  await paste(page, block(4, (i) => [`Late ${i}`, "2,00"]));
  await expect.poll(() => summary(page)).toBe("2 rows, 4 values beyond the end are not pasted");
  await page.keyboard.press("Enter");
  const [pasted] = await page.evaluate(() => window.__pasted);
  expect(pasted.changes).toHaveLength(4);
  expect(pasted.added).toEqual([]);
});

test("with add-row the rows past the end become added rows, and a required column the block misses refuses them", async ({ page }) => {
  await open(page, "?add");
  // The new row is the last; the last source row is above it.
  await goToRow(page, 40, 1);
  // Without the id (required) the new row cannot be added.
  await paste(page, "Late 1\t2,00\t7\nLate 2\t3,00\t8\n");
  await expect.poll(() => summary(page)).toBe("2 rows, 1 value not readable");
  await page.keyboard.press("Escape");

  await page.keyboard.press("ArrowLeft");
  await paste(page, "40\tLate 1\t2,00\t7\n41\tLate 2\t3,00\t8\n");
  await expect.poll(() => summary(page)).toBe("2 rows");
  await page.keyboard.press("Enter");
  const [pasted] = await page.evaluate(() => window.__pasted);
  expect(pasted.changes).toHaveLength(4);
  expect(pasted.added).toHaveLength(1);
  expect(pasted.added[0].values).toMatchObject({ id: 41, name: "Late 2", price: "3.00", seats: 8 });
  expect(pasted.added[0].provisionalKey).toMatch(/^new-/);
});

// Issue #228: what a paste may not write into, and what it must not outlive.

test("a read-only column is left out of the paste, with its reason in the preview", async ({ page }) => {
  await open(page, "?readonly");
  await focusCell(page, 0, 1);
  await paste(page, "N1\t5,00\t7\n");
  await expect.poll(() => summary(page)).toBe("1 row, price is read-only");
  const left = await shadow(page, (root) => {
    const cell = root.querySelector('[part="paste-preview"] td[aria-readonly="true"]');
    return { text: cell?.textContent, invalid: cell?.hasAttribute("aria-invalid") };
  });
  expect(left.text).toContain("price is read-only");
  expect(left.invalid).toBe(false);

  // It does not block: the other cells are pasted, the read-only one never reaches the page.
  await page.keyboard.press("Enter");
  const [pasted] = await page.evaluate(() => window.__pasted);
  expect(pasted.changes.map((change) => change.column)).toEqual(["name", "seats"]);
  expect(await cell(page, 0, 2)).toContain("1.00");
});

test("a paste of read-only cells only has nothing to tell the page", async ({ page }) => {
  await open(page, "?readonly");
  await focusCell(page, 0, 2);
  await paste(page, "500\n600\n");
  await expect.poll(() => summary(page)).toBe("0 rows, price is read-only");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
  expect(await summary(page)).toBeNull();
});

// A list is picked from the page's choices, not read from text: the paste says so and waits.
test("a list column is not written from the clipboard", async ({ page }) => {
  await open(page, "?list");
  await focusCell(page, 0, 5);
  await paste(page, "a\n");
  await expect.poll(() => summary(page)).toBe("0 rows, 1 value not readable");
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
});

test("a paste while filtered lands on the filtered rows", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => {
    const input = root.querySelector('input[data-col="1"]');
    input.value = "Item 3";
    input.focus();
  });
  await page.keyboard.press("Enter");
  await expect.poll(() => cell(page, 1, 1)).toBe("Item 30");
  await focusCell(page, 0, 1);
  await paste(page, "Only\n");
  await expect.poll(() => summary(page)).toBe("1 row");
  await page.keyboard.press("Enter");
  const [pasted] = await page.evaluate(() => window.__pasted);
  expect(pasted.changes).toEqual([expect.objectContaining({ key: 3, row: 0, column: "name", value: "Only", previous: "Item 3" })]);
});

test("a grouped grid leaves the paste to the browser", async ({ page }) => {
  await open(page, "?group");
  // Open the group and stand on a record in it: a cell a paste could land on.
  await focusCell(page, 0, 0);
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowDown");
  expect(await shadow(page, (root) => root.activeElement?.closest("tr")?.getAttribute("aria-expanded"))).toBeNull();
  expect(await shadow(page, (root) => root.activeElement?.tagName)).toBe("TD");
  const prevented = await page.evaluate(() => {
    const data = new DataTransfer();
    data.setData("text/plain", "x\ty\n");
    const event = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true, composed: true });
    document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(prevented).toBe(false);
  expect(await summary(page)).toBeNull();
});

test("a preview does not add to the queries the footer counts", async ({ page }) => {
  await open(page);
  const footer = () => shadow(page, (root) => root.querySelector('[part="source"]').textContent);
  const before = await footer();
  expect(before).not.toBe("");
  await focusCell(page, 0, 1);
  await paste(page, "A\nB\n");
  await expect.poll(() => summary(page)).toBe("2 rows");
  expect(await footer()).toBe(before);
});

test("a result for another sort drops the preview, and an answer that comes too late shows none", async ({ page }) => {
  await open(page);
  await focusCell(page, 0, 1);
  await paste(page, "A\nB\n");
  await expect.poll(() => summary(page)).toBe("2 rows");

  // The reader sorts: the positions the preview names hold other records.
  await shadow(page, (root) => root.querySelector('th[data-col="1"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => summary(page)).toBeNull();
  expect(await status(page)).toContain("Paste discarded");

  // An answer for the rows of the old order lands after the sort's.
  await page.evaluate(() => (window.__hold = true));
  await focusCell(page, 0, 1);
  await paste(page, "C\nD\n");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBe(1);
  await shadow(page, (root) => root.querySelector('th[data-col="1"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__held.length)).toBe(2);
  await page.evaluate(() => window.__release(1));
  await expect.poll(() => status(page)).not.toContain("Loading");
  await page.evaluate(() => window.__release(0));
  await expect.poll(() => status(page)).toContain("Paste discarded");
  expect(await summary(page)).toBeNull();
  expect(await page.evaluate(() => window.__pasted)).toHaveLength(0);
});
