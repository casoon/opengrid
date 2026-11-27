import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// Scaled integer columns (issue #537): `scale: 2` makes `price_cent` an int64
// that stores 612 for what a reader reads and types as 6,12. Shown, edited,
// filtered, summed, pasted, set in bulk and saved in the record form as the
// scaled number — in the page's notation — while what the page and the wire see
// stays the stored integer.
// Columns: id (key), region, price_cent (scale 2), qty (a plain integer), rating
// (a float). Rows 0 to 7 hold 612, 500, 499, 123450, 5, 250, NULL and 1000 cents.

const ID = 0;
const PRICE = 2;
const QTY = 3;

const SHOWN = {
  de: ["6,12", "5,00", "4,99", "1.234,50", "0,05", "2,50", "", "10,00"],
  en: ["6.12", "5.00", "4.99", "1,234.50", "0.05", "2.50", "", "10.00"],
};

const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });
/** The texts of a column's cells, in the order of the rows. */
const column = (page, col) =>
  shadow(page, (root, col) =>
    [...root.querySelectorAll("tbody tr[aria-rowindex]")]
      .sort((a, b) => Number(a.getAttribute("aria-rowindex")) - Number(b.getAttribute("aria-rowindex")))
      .map((tr) => tr.querySelector(`td[data-col="${col}"]`))
      .filter((td) => td && td.dataset.row !== undefined && !td.closest("tr").hidden && td.closest("tr").style.display !== "none")
      .map((td) => td.textContent),
    col);
const editor = (page) =>
  shadow(page, (root) => {
    const node = root.querySelector('[part="editor"]');
    if (!node) return null;
    return { type: node.getAttribute("type"), inputmode: node.getAttribute("inputmode"), value: node.value, invalid: node.getAttribute("aria-invalid") };
  });
/** The sentence under the cell editor. */
const sentence = (page) => shadow(page, (root) => root.querySelector('td [part="validation"]')?.textContent ?? null);
const chips = (page) => shadow(page, (root) => [...root.querySelectorAll('[part="chip"]')].map((chip) => chip.firstElementChild.textContent));
const view = (page) => page.evaluate(() => window.__module.get_view(window.__host));
const changes = (page) => page.evaluate(() => window.__changes);
/** The last query for rows: the facets' counting queries select no `id`. */
const lastQuery = (page) => page.evaluate(() => window.__queries.filter((query) => query.select?.includes("id")).at(-1));

async function open(page, lang = "en", query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-scaled.html?lang=${lang}${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => status(page)).toContain("8 matches");
  if (!query.includes("group")) await expect.poll(async () => (await column(page, PRICE)).length).toBe(8);
}

async function focusCell(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
}

/** Opens the editor on a cell, replaces its text and commits with Enter. */
async function enter(page, row, col, text) {
  await focusCell(page, row, col);
  await page.keyboard.press("Enter");
  await page.keyboard.press("ControlOrMeta+a");
  if (text === "") await page.keyboard.press("Backspace");
  else await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

/** Chooses an operator by its wire token, types a value into a column's filter field and applies it. */
async function filterBy(page, col, token, text) {
  await shadow(page, (root, { col, token }) => {
    root.querySelector(`[part="filter-operator"][data-col="${col}"]`).click();
    root.querySelector(`[part="operator-menu"] [data-op="${token}"]`).click();
    root.querySelector(`input[data-col="${col}"]`).focus();
  }, { col, token });
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type(text);
  await page.keyboard.press("Enter");
}

const position = (page) => shadow(page, (root) => root.querySelector('[part="position"]').textContent);
const filterField = (page, col) => shadow(page, (root, col) => root.querySelector(`input[data-col="${col}"]`).value, col);

for (const lang of ["de", "en"]) {
  test.describe(`on a ${lang === "de" ? "German" : "English"} page`, () => {
    test("the cell shows the scaled number through the column's format, the other integer column its own", async ({ page }) => {
      await open(page, lang);
      expect(await column(page, PRICE)).toEqual(SHOWN[lang]);
      expect(await column(page, QTY)).toEqual(["1", "2", "3", "4", "5", "6", "7", "8"]);
    });

    test("the editor opens with the scaled number, reads it, and the page is told the stored integer", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await focusCell(page, 0, PRICE);
      await page.keyboard.press("Enter");
      // A text field for decimals, not a number spinner: the page's notation is read.
      expect(await editor(page)).toMatchObject({ type: "text", inputmode: "decimal", value: `6${decimal}12` });
      await page.keyboard.press("Escape");

      await enter(page, 0, PRICE, `7${decimal}5`);
      await expect.poll(() => cell(page, 0, PRICE)).toBe(`7${decimal}50`);
      expect(await changes(page)).toEqual([{ row: 0, key: 1, column: "price_cent", value: "750", previous: "612" }]);

      // 6,12 is 612, and a whole number is that many euros.
      await enter(page, 1, PRICE, `6${decimal}12`);
      await expect.poll(() => cell(page, 1, PRICE)).toBe(`6${decimal}12`);
      expect((await changes(page)).at(-1)).toEqual({ row: 1, key: 2, column: "price_cent", value: "612", previous: "500" });
      await enter(page, 2, PRICE, "3");
      expect((await changes(page)).at(-1)).toMatchObject({ column: "price_cent", value: "300", previous: "499" });
      // A group mark where it stands right; an empty field is still NULL.
      await enter(page, 4, PRICE, lang === "de" ? "1.234,5" : "1,234.5");
      expect((await changes(page)).at(-1)).toMatchObject({ column: "price_cent", value: "123450", previous: "5" });
      await enter(page, 5, PRICE, "");
      expect((await changes(page)).at(-1)).toMatchObject({ column: "price_cent", value: "", previous: "250" });

      // A validator is given what the page is told, on every key and on commit.
      const seen = await page.evaluate(() => window.__seen);
      expect(seen).toContain("750");
      expect(seen).not.toContain(`7${decimal}5`);
    });

    test("more decimals than the scale are refused with the reason, never rounded", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await enter(page, 2, PRICE, `1${decimal}234`);
      expect(await sentence(page)).toBe(`“1${decimal}234” has more than 2 decimal places`);
      expect(await editor(page)).toMatchObject({ value: `1${decimal}234`, invalid: "true" });
      await expect.poll(() => status(page)).toContain("has more than 2 decimal places");
      const results = await new AxeBuilder({ page }).analyze();
      expect(results.violations).toEqual([]);
      expect(await changes(page)).toEqual([]);

      // Too large once shifted: the stored integer would not fit.
      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type("92233720368547759");
      await page.keyboard.press("Enter");
      expect(await sentence(page)).toBe("“92233720368547759” is too large for this column");

      // Zeros past the scale lose nothing.
      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type(`1${decimal}230`);
      await page.keyboard.press("Enter");
      await expect.poll(() => cell(page, 2, PRICE)).toBe(`1${decimal}23`);
      expect(await changes(page)).toEqual([{ row: 2, key: 3, column: "price_cent", value: "123", previous: "499" }]);
    });

    test("the filter row reads the scaled number: >= 5 filters 500 of the stored integer", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      expect(await shadow(page, (root, col) => root.querySelector(`input[data-col="${col}"]`).getAttribute("inputmode"), PRICE)).toBe("decimal");
      await filterBy(page, PRICE, "gte", "5");
      await expect.poll(() => status(page)).toContain("4 matches");
      expect(await column(page, PRICE)).toEqual([SHOWN[lang][0], SHOWN[lang][1], SHOWN[lang][3], SHOWN[lang][7]]);

      // What the page and the wire see is the stored integer: the view, the query.
      expect((await view(page)).filters).toEqual([{ column: "price_cent", op: "gte", value: "500" }]);
      expect(JSON.stringify((await lastQuery(page)).filter)).toContain('"value":500');
      const query = await page.evaluate(() => window.__module.get_query(window.__host));
      expect(JSON.stringify(query.filter)).toContain('"value":500');
      // What a reader reads is the scaled number: in the chip.
      expect((await chips(page)).at(-1)).toMatch(new RegExp(`Price .* 5${decimal}00$`));

      // A value with a decimal mark; more decimals than the scale are refused.
      await filterBy(page, PRICE, "lt", `6${decimal}5`);
      await expect.poll(() => status(page)).toContain("5 matches");
      expect((await view(page)).filters).toEqual([{ column: "price_cent", op: "lt", value: "650" }]);
      const shown = await column(page, PRICE);
      await filterBy(page, PRICE, "lt", `1${decimal}234`);
      await expect.poll(() => status(page)).toContain(`1${decimal}234`);
      expect(await column(page, PRICE)).toEqual(shown);
    });

    test("a view with the stored integer fills the field with the scaled number", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await page.evaluate(() =>
        window.__module.set_view(window.__host, { filters: [{ column: "price_cent", op: "gte", value: "250" }] }),
      );
      await expect.poll(() => status(page)).toContain("6 matches");
      expect(await filterField(page, PRICE)).toBe(`2${decimal}50`);
      expect((await view(page)).filters).toEqual([{ column: "price_cent", op: "gte", value: "250" }]);
    });

    test("the search expression is written in the scale, in a nested one too", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      const search = async (text) => {
        await shadow(page, (root) => root.querySelector('[part="search-input"]').focus());
        await page.keyboard.type(text);
        await page.keyboard.press("Enter");
      };
      await search("price_cent >= 5");
      await expect.poll(() => status(page)).toContain("4 matches");
      expect((await view(page)).filters).toEqual([{ column: "price_cent", op: "gte", value: "500" }]);
      expect(await filterField(page, PRICE)).toBe(`5${decimal}00`);

      await search(`price_cent < 2${decimal}5 or price_cent > 10`);
      await expect.poll(() => status(page)).toContain("Filter set");
      expect((await view(page)).where).toEqual({
        or: [
          { field: "price_cent", op: "lt", value: 250 },
          { field: "price_cent", op: "gt", value: 1000 },
        ],
      });
      // Under the filter row's >= 5,00 only 1.234,50 is above 10,00 (and none is below 2,50).
      await expect.poll(() => column(page, PRICE)).toEqual([SHOWN[lang][3]]);
      // The chip says the nested filter in the scaled number.
      expect((await chips(page)).join(" ")).toContain(`2${decimal}50`);
    });

    test("the filter builder shows the scaled number and keeps the stored integer", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await shadow(page, (root) => root.querySelector('[part="search-input"]').focus());
      await page.keyboard.type(`price_cent < 2${decimal}5 or price_cent > 10`);
      await page.keyboard.press("Enter");
      await expect.poll(() => status(page)).toContain("Filter set");

      await page.locator("opengrid-grid [data-chip-edit]").click();
      const dialog = page.locator('opengrid-grid [part="filter-builder-dialog"]');
      const values = dialog.locator('[data-dialog="value"]');
      await expect(values).toHaveCount(2);
      expect(await values.evaluateAll((nodes) => nodes.map((node) => [node.type, node.getAttribute("inputmode"), node.value]))).toEqual([
        ["text", "decimal", `2${decimal}50`],
        ["text", "decimal", `10${decimal}00`],
      ]);
      // Changed and applied: the tree holds the stored integer.
      await values.first().fill(`3${decimal}5`);
      await dialog.getByRole("button", { name: "Apply" }).click();
      await expect(dialog).toHaveCount(0);
      expect((await view(page)).where).toEqual({
        or: [
          { field: "price_cent", op: "lt", value: 350 },
          { field: "price_cent", op: "gt", value: 1000 },
        ],
      });
    });

    test("the + Filter dialog types the value as the scaled number and writes it into the filter row", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await shadow(page, (root) => root.querySelector('[part="add-filter"]').click());
      const dialog = page.locator('opengrid-grid [part="filter-dialog"]');
      await dialog.locator('[data-dialog="column"]').selectOption("price_cent");
      await dialog.locator('[data-dialog="op"]').selectOption("gte");
      const value = dialog.locator('[data-dialog="value"]');
      expect(await value.evaluate((node) => [node.type, node.getAttribute("inputmode"), node.getAttribute("step")])).toEqual(["text", "decimal", null]);
      await value.fill(`5${decimal}5`);
      await value.press("Enter");
      await expect.poll(() => status(page)).toContain("3 matches");
      expect((await view(page)).filters).toEqual([{ column: "price_cent", op: "gte", value: "550" }]);
      expect(await filterField(page, PRICE)).toBe(`5${decimal}50`);
    });

    test("a range facet takes the scaled number and keeps the stored integer", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await page.evaluate(() => {
        const input = document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector('input[data-facet="price_cent"][data-bound="low"]');
        input.value = "5";
        input.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
      });
      await expect.poll(() => status(page)).toContain("4 matches");
      expect((await view(page)).facets).toEqual({ price_cent: { min: "500", max: "" } });
      expect(JSON.stringify((await lastQuery(page)).filter)).toContain('"value":500');
      expect((await chips(page)).join(" ")).toContain(`Price 5${decimal}00 – …`);
      await expect
        .poll(() => shadow(page, (root) => root.querySelector('input[data-facet="price_cent"][data-bound="low"]').value))
        .toBe(`5${decimal}00`);
    });

    test("a sum is of the scaled number, in a group and in the total, and exact", async ({ page }) => {
      await open(page, lang, "&group");
      const sums = lang === "de" ? ["13,62", "1.249,54", "1.263,16"] : ["13.62", "1,249.54", "1,263.16"];
      const drawn = () =>
        shadow(page, (root) =>
          [...root.querySelectorAll("tbody tr[data-kind]")]
            .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
            .map((tr) => {
              const td = tr.querySelector('td[data-col="2"]');
              return { kind: tr.dataset.kind, text: td.textContent, said: td.getAttribute("aria-label") };
            }),
        );
      await expect
        .poll(drawn)
        .toEqual([
          { kind: "group", text: sums[0], said: `Sum: ${sums[0]}` },
          { kind: "group", text: sums[1], said: `Sum: ${sums[1]}` },
          { kind: "total", text: sums[2], said: `Sum: ${sums[2]}` },
        ]);
    });

    test("the sum and the average of a range of cells are of the scaled numbers, across columns too", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      // 6,12 and 5,00 — 612 and 500 cents, summed as integers.
      await focusCell(page, 0, PRICE);
      await page.keyboard.press("Shift+ArrowDown");
      await expect.poll(() => position(page)).toBe(`2 by 1 cells · Count: 2 · Sum: 11${decimal}12 · Average: 5${decimal}56`);

      // Beside a plain integer the sum is of what each cell reads, 6,12 + 5,00 +
      // 1 + 2 — in the wire notation, as every sum over more than one column.
      await page.keyboard.press("Shift+ArrowRight");
      await expect.poll(() => position(page)).toContain("Sum: 14.12");
    });

    test("a paste reads the scaled number, the page is told the stored integer, and a copy writes what it reads", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      const paste = (text) =>
        page.evaluate((text) => {
          const data = new DataTransfer();
          data.setData("text/plain", text);
          const event = new ClipboardEvent("paste", {
            clipboardData: data, dataType: "text/plain", data: text, bubbles: true, cancelable: true, composed: true,
          });
          document.querySelector("opengrid-grid").shadowRoot.activeElement.dispatchEvent(event);
        }, text);
      const summary = () => shadow(page, (root) => root.querySelector('[part="paste-summary"]')?.textContent ?? null);

      await focusCell(page, 0, PRICE);
      await paste(`7${decimal}5\t9\n6${decimal}12\t8\n`);
      await expect.poll(summary).toBe("2 rows");
      await page.keyboard.press("Enter");
      const pasted = await page.evaluate(() => window.__pasted);
      expect(pasted).toHaveLength(1);
      expect(pasted[0].changes.map(({ column, value, previous }) => [column, value, previous])).toEqual([
        ["price_cent", "750", "612"],
        ["qty", "9", "1"],
        ["price_cent", "612", "500"],
        ["qty", "8", "2"],
      ]);
      await expect.poll(() => cell(page, 0, PRICE)).toBe(`7${decimal}50`);
      expect(await cell(page, 1, PRICE)).toBe(`6${decimal}12`);

      // More decimals than the scale are not read: the paste is not applied.
      await focusCell(page, 2, PRICE);
      await paste(`1${decimal}234\t9\n`);
      await expect.poll(summary).toBe("1 row, 1 value not readable");
      expect(
        await shadow(page, (root) => root.querySelector('[part="paste-preview"] td[aria-invalid="true"]')?.textContent),
      ).toContain(`“1${decimal}234” cannot be read as number`);
      await page.keyboard.press("Escape");

      // A copy is the scaled number, in the wire notation, which a paste reads back.
      await focusCell(page, 0, PRICE);
      await page.keyboard.press("Shift+ArrowDown");
      await page.keyboard.press("ControlOrMeta+c");
      await expect.poll(() => page.evaluate(() => window.__copied.at(-1))).toBe("7.50\n6.12\n");
    });

    test("the record form holds the scaled number and saves the stored integer", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await focusCell(page, 0, PRICE);
      await page.keyboard.press("Shift+F2");
      const field = page.locator('opengrid-grid [part="record-dialog"] [data-record-col="2"]');
      await expect(field).toHaveAttribute("inputmode", "decimal");
      await expect(field).toHaveValue(`6${decimal}12`);
      await expect(page.locator('opengrid-grid [part="record-dialog"] [data-record-col="3"]')).toHaveAttribute("inputmode", "numeric");
      await expect(page.locator('opengrid-grid [part="record-dialog"] [data-record-col="3"]')).toHaveValue("1");

      await field.focus();
      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type(`1${decimal}234`);
      await page.keyboard.press("Enter");
      await expect(page.locator('opengrid-grid [part="record-dialog"] #og-record-error-2')).toHaveText(
        `“1${decimal}234” has more than 2 decimal places`,
      );
      expect(await changes(page)).toEqual([]);

      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type(`9${decimal}99`);
      await page.keyboard.press("Enter");
      await expect.poll(() => cell(page, 0, PRICE)).toBe(`9${decimal}99`);
      expect(await changes(page)).toEqual([{ row: 0, key: 1, column: "price_cent", value: "999", previous: "612" }]);
    });

    test("the bulk panel holds the scaled number and sets the stored integer", async ({ page }) => {
      await open(page, lang);
      const decimal = lang === "de" ? "," : ".";
      await shadow(page, (root) =>
        root.querySelector('tr[aria-rowindex="3"] td[data-select="row"] [part="select-mark"], tr[aria-rowindex="3"] td[data-select="row"]').focus(),
      );
      for (let i = 0; i < 3; i++) {
        await page.keyboard.press(" ");
        await page.keyboard.press("ArrowDown");
      }
      await focusCell(page, 1, PRICE);
      await page.keyboard.press("F2");
      const field = page.locator('opengrid-grid [part="bulk"] [part="editor"]');
      await expect(field).toHaveAttribute("inputmode", "decimal");

      await page.keyboard.type(`1${decimal}234`);
      await page.keyboard.press("Enter");
      await expect(page.locator('opengrid-grid [part="bulk"] [part="validation"]')).toHaveText(
        `“1${decimal}234” has more than 2 decimal places`,
      );
      expect(await page.evaluate(() => window.__bulk)).toEqual([]);

      await page.keyboard.press("ControlOrMeta+a");
      await page.keyboard.type(`1${decimal}5`);
      await page.keyboard.press("Enter");
      expect(await page.evaluate(() => window.__bulk)).toEqual([{ keys: [2, 3, 4], column: "price_cent", value: "150" }]);
      expect([await cell(page, 1, PRICE), await cell(page, 2, PRICE), await cell(page, 3, PRICE)]).toEqual([
        `1${decimal}50`,
        `1${decimal}50`,
        `1${decimal}50`,
      ]);
    });
  });
}

test("an export keeps the stored integer", async ({ page }) => {
  await open(page, "de");
  const csv = await page.evaluate(async () => {
    const { exportRows } = await import("/packages/opengrid/loader.js");
    const blob = await exportRows(window.__provider, window.__module.get_query(window.__host), { bom: false });
    return await blob.text();
  });
  // What a file is read back from: the stored integer, not 6,12.
  const lines = csv.split("\r\n");
  expect(lines.slice(0, 5)).toEqual([
    "id,region,price_cent,qty,rating",
    "1,A,612,1,0.5",
    "2,A,500,2,0.5",
    "3,B,499,3,0.5",
    "4,B,123450,4,0.5",
  ]);
  expect(lines[7]).toBe("7,A,,7,0.5");
});

test("a scale on a column that is not an integer, or one that is no number of places, is reported and none of the call applied", async ({ page }) => {
  await open(page, "de");
  const call = (columns) => page.evaluate((columns) => window.__module.set_columns(window.__host, columns), columns);

  // qty is a valid integer; rating is a float: the whole call is refused.
  await call({ qty: { scale: 2 }, rating: { scale: 2 } });
  await expect.poll(() => status(page)).toContain("rating: a scale needs an int64 column");
  expect(await cell(page, 0, QTY)).toBe("1");
  expect(await cell(page, 0, PRICE)).toBe("6,12");

  await call({ price_cent: { scale: "2" }, region: { scale: 2 } });
  await expect.poll(() => status(page)).toContain("price_cent: scale is not a whole number from 0 to 18");
  expect(await status(page)).toContain("region: a scale needs an int64 column");
  await call({ qty: { scale: 19 } });
  await expect.poll(() => status(page)).toContain("qty: scale is not a whole number from 0 to 18");
  await call({ qty: { scale: 1.5 } });
  await expect.poll(() => status(page)).toContain("qty: scale is not a whole number from 0 to 18");
  await call({ qty: { scale: 2, lookup: { 1: "One" } } });
  await expect.poll(() => status(page)).toContain("qty: a lookup column takes no scale");
  expect(await cell(page, 0, QTY)).toBe("1");

  // The configuration of the page stands again with the next valid call.
  await call({ price_cent: { title: "Price", scale: 2 }, qty: { scale: 1 } });
  await expect.poll(() => cell(page, 0, QTY)).toBe("0.1");
  expect(await cell(page, 0, PRICE)).toBe("6,12");
});

test("an integer column without a scale is what it was", async ({ page }) => {
  await open(page, "de");
  await filterBy(page, QTY, "gte", "5");
  await expect.poll(() => status(page)).toContain("4 matches");
  expect((await view(page)).filters).toEqual([{ column: "qty", op: "gte", value: "5" }]);
  await enter(page, 0, QTY, "1.234");
  expect((await changes(page)).at(-1)).toMatchObject({ key: 5, column: "qty", value: "1234", previous: "5" });
  expect(await shadow(page, (root, col) => root.querySelector(`input[data-col="${col}"]`).getAttribute("type"), QTY)).toBe("number");
  expect(await shadow(page, (root, col) => root.querySelector(`input[data-col="${col}"]`).getAttribute("type"), ID)).toBe("number");
});
