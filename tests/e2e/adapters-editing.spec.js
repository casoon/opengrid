import { test, expect } from "@playwright/test";

// Editing through the adapters (issue #206): each example's rooms grid adds a
// row, pastes, undoes and runs a row action through props and callbacks alone,
// and tells a saved edit through its ref. The examples record every callback in
// `window.__edits`.

const EXAMPLES = [
  { name: "React 19", page: "/examples/react/dist/index.html" },
  { name: "React 18", page: "/examples/react/dist-18/index.html" },
  { name: "Vue", page: "/examples/vue/dist/index.html" },
  { name: "Svelte", page: "/examples/svelte/dist/index.html" },
  { name: "Angular", page: "/examples/angular/dist/browser/index.html" },
];

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid#rooms").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const rows = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.getAttribute("aria-rowindex"))
      .map((tr) => ({
        name: tr.querySelector('td[data-col="1"]')?.textContent.replace(/\s*\*$/, ""),
        added: tr.getAttribute("data-added"),
      })));

const edits = (page) => page.evaluate(() => window.__edits);

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

for (const example of EXAMPLES) {
  test.describe(example.name, () => {
    let problems;

    test.beforeEach(async ({ page }) => {
      problems = [];
      page.on("pageerror", (error) => problems.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error" || message.type() === "warning") {
          problems.push(message.text());
        }
      });
      await page.goto(example.page);
      await expect.poll(async () => (await rows(page).catch(() => [])).length).toBe(4);
    });

    test.afterEach(() => {
      expect(problems).toEqual([]);
    });

    test("the attributes and the options arrive: a record key, the new row, an action column", async ({ page }) => {
      const attributes = await page.evaluate(() => {
        const grid = document.querySelector("opengrid-grid#rooms");
        return { rowKey: grid.getAttribute("row-key"), addRow: grid.getAttribute("add-row") };
      });
      expect(attributes).toEqual({ rowKey: "id", addRow: "" });
      expect(await shadow(page, (root) => root.querySelector('tr[aria-rowindex="2"] button[part="action"]')?.textContent)).toBe("Remove");
    });

    test("a row is added and the callback's answer saves it", async ({ page }) => {
      await shadow(page, (root) => root.querySelector('tr[data-new-row] td[data-col="0"]').focus());
      for (const text of ["4", "D4", "40"]) {
        await page.keyboard.press("Enter");
        await page.keyboard.type(text);
        await page.keyboard.press("Enter");
      }
      expect((await edits(page)).filter((edit) => edit.type === "rowAdd")).toEqual([
        expect.objectContaining({ values: { id: 4, name: "D4", seats: 40 } }),
      ]);
      await expect.poll(async () => (await rows(page))[3]).toEqual({ name: "D4", added: "saved" });
    });

    test("a paste and its undo reach the page, and so does a row action", async ({ page }) => {
      await focusCell(page, 0, 1);
      await page.evaluate(() => {
        const text = "P1\t15\n";
        const data = new DataTransfer();
        data.setData("text/plain", text);
        const root = document.querySelector("opengrid-grid#rooms").shadowRoot;
        // Firefox reads `dataType` and `data`, not `clipboardData` (grid-paste.spec.js).
        root.activeElement.dispatchEvent(new ClipboardEvent("paste", {
          clipboardData: data, dataType: "text/plain", data: text, bubbles: true, cancelable: true, composed: true,
        }));
      });
      await expect.poll(() => shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent)).toBe("1 row");
      await page.keyboard.press("Enter");
      await page.keyboard.press("Control+z");
      const pastes = (await edits(page)).filter((edit) => edit.type === "paste");
      expect(pastes).toHaveLength(2);
      expect(pastes[0].changes[0]).toMatchObject({ key: 1, column: "name", value: "P1" });
      expect(pastes[1]).toMatchObject({ undo: true });

      await shadow(page, (root) => root.querySelector('tr[aria-rowindex="3"] button[part="action"]').click());
      expect((await edits(page)).filter((edit) => edit.type === "rowAction")).toEqual([
        expect.objectContaining({ id: "remove", key: 2 }),
      ]);
    });

    test("a validator from the options speaks, and an edit is marked saved through the ref", async ({ page }) => {
      await focusCell(page, 0, 2);
      await page.keyboard.press("Enter");
      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type("-5");
      await expect.poll(() => shadow(page, (root) => root.querySelector('[part="validation"]')?.textContent ?? null))
        .toContain("Seats cannot be negative.");
      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type("15");
      await page.keyboard.press("Enter");
      expect((await edits(page)).filter((edit) => edit.type === "cellChange")).toEqual([
        expect.objectContaining({ key: 1, column: "seats", value: "15" }),
      ]);
      await expect.poll(() => shadow(page, (root) => root.querySelector('td[data-row="0"][data-col="2"]').getAttribute("part")))
        .toBe("cell cell-saved");
    });
  });
}
