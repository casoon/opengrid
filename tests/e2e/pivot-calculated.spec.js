import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Calculated measures (issue #329): a share of the row, the column or the
// grand total, and a running total — computed by the source, shown, chosen
// from the toolbar and exported by `<opengrid-pivot>`.
//
// The fixture's table is the one of crates/opengrid-pivot/tests/calculated.rs,
// small enough to reckon by hand, and the numbers below were: `sum(units)` by
// zone and region across the years is
//
//              2023  2024  2025   row total
//   A N           1     3     –       4
//   A S           –     2     –       2
//   Total A       1     5     –       6
//   B E           –     –     –       –    (no row in 2023 and 2025)
//   B W           5    -5     –       0    (no row in 2025)
//   Total B       5    -5     –       0
//   Total         6     0     –       6
//
// so there is a row total of zero (W, B) and of NULL (E), and a column total
// of zero (2024) and of NULL (2025).

const N = null;
const ROWS = ["A N", "A S", "Total A", "B E", "B W", "Total B", "Total"];
const SUMS = [
  [1, 3, N],
  [N, 2, N],
  [1, 5, N],
  [N, N, N],
  [5, -5, N],
  [5, -5, N],
  [6, 0, N],
];
const BY_HAND = {
  share_of_row: [
    [1 / 4, 3 / 4, N],
    [N, 1, N],
    [1 / 6, 5 / 6, N],
    [N, N, N],
    [N, N, N],
    [N, N, N],
    [1, 0, N],
  ],
  share_of_column: [
    [1 / 6, N, N],
    [N, N, N],
    [1 / 6, N, N],
    [N, N, N],
    [5 / 6, N, N],
    [5 / 6, N, N],
    [1, N, N],
  ],
  share_of_total: [
    [1 / 6, 3 / 6, N],
    [N, 2 / 6, N],
    [1 / 6, 5 / 6, N],
    [N, N, N],
    [5 / 6, -5 / 6, N],
    [5 / 6, -5 / 6, N],
    [1, 0, N],
  ],
  // S is NULL before its first value and keeps 2 after it; E never has one.
  running_total: [
    [1, 4, 4],
    [N, 2, 2],
    [1, 6, 6],
    [N, N, N],
    [5, 0, 0],
    [5, 0, 0],
    [6, 6, 6],
  ],
};

const inside = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(
        document.querySelector("opengrid-pivot").shadowRoot,
        arg,
      ),
    { fn: fn.toString(), arg },
  );

const state = (page) => inside(page, (root) => root.querySelector('[part="status"]').dataset.state);
const status = (page) => inside(page, (root) => root.querySelector('[part="status"]').textContent);

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/pivot-calculated.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => state(page)).toBe("ready");
}

/** The body of the table: per row its header words and its cells as text. */
const body = (page) =>
  inside(page, (root) =>
    [...root.querySelectorAll("tbody tr")].map((row) => ({
      header: [...row.querySelectorAll("th")]
        // A subtotal's header is a fold button: its name without the mark.
        .map((th) => th.querySelector("button > span:last-child")?.textContent ?? th.textContent)
        .join(" "),
      cells: [...row.querySelectorAll("td")].map((td) => td.textContent),
    })),
  );

/** The measure headers, as the header row under the years says them. */
const measureHeaders = (page) =>
  inside(page, (root) => [...root.querySelectorAll("thead tr:last-child th")].map((th) => th.textContent));

/** Sets `values` and waits for the answer it causes. */
async function show(page, values) {
  const json = JSON.stringify(values);
  await page.evaluate((json) => {
    window.__answers = 0;
    const pivot = document.querySelector("opengrid-pivot");
    pivot.addEventListener("opengrid-query", () => window.__answers++, { once: true });
    pivot.setAttribute("values", json);
  }, json);
  await page.waitForFunction(() => window.__answers > 0);
  await expect.poll(() => state(page)).toBe("ready");
}

/** Holds the shown cells against `expected`: an empty cell is NULL. */
async function expectCells(page, expected, label) {
  const shown = await body(page);
  expect(shown.map((row) => row.header)).toEqual(ROWS);
  for (const [index, row] of shown.entries()) {
    for (const [column, text] of row.cells.entries()) {
      const wanted = expected[index][column];
      const where = `${label}: ${ROWS[index]}, ${2023 + column}`;
      if (wanted === null) {
        expect(text, where).toBe("");
      } else {
        expect(text, where).not.toBe("");
        expect(Number(text), where).toBeCloseTo(wanted, 12);
      }
    }
  }
}

const SOLD = { field: "units", fn: "sum", as: "sold" };

test.describe("pivot calculated measures", () => {
  test("the numbers are the ones reckoned by hand", async ({ page }) => {
    await open(page);
    await expectCells(page, SUMS, "sum");
    for (const [calc, expected] of Object.entries(BY_HAND)) {
      await show(page, [{ ...SOLD, calc }]);
      await expectCells(page, expected, calc);
    }
    // A row's shares of its row total add up to one, where it has a total.
    await show(page, [{ ...SOLD, calc: "share_of_row" }]);
    for (const row of await body(page)) {
      const shares = row.cells.filter((text) => text !== "").map(Number);
      if (shares.length > 0) {
        expect(shares.reduce((sum, share) => sum + share, 0), row.header).toBeCloseTo(1, 12);
      }
    }
    // A decimal running total stays a decimal, to the cent.
    await show(page, [{ field: "amount", fn: "sum", as: "revenue", calc: "running_total" }]);
    expect((await body(page)).map((row) => row.cells)).toEqual([
      ["10.00", "40.00", "40.00"],
      ["", "20.00", "20.00"],
      ["10.00", "60.00", "60.00"],
      ["", "", ""],
      ["5.50", "0.00", "0.00"],
      ["5.50", "0.00", "0.00"],
      ["15.50", "60.00", "60.00"],
    ]);
  });

  test("a calculated measure stands beside its aggregate, each under its own header", async ({ page }) => {
    await open(page);
    await show(page, [SOLD, { ...SOLD, as: "share", calc: "share_of_row" }]);
    expect(await measureHeaders(page)).toEqual([
      "sold",
      "share · share of row",
      "sold",
      "share · share of row",
      "sold",
      "share · share of row",
    ]);
    const first = (await body(page))[0];
    expect(first.cells.map((text) => (text === "" ? null : Number(text)))).toEqual([1, 0.25, 3, 0.75, null, null]);
    expect((await new AxeBuilder({ page }).include("opengrid-pivot").analyze()).violations).toEqual([]);
  });

  test("an aggregate that cannot be added up is refused in the status line", async ({ page }) => {
    await open(page);
    await page.evaluate(() =>
      document
        .querySelector("opengrid-pivot")
        .setAttribute("values", '[{"field":"units","fn":"avg","as":"mean","calc":"share_of_row"}]'),
    );
    await expect.poll(() => state(page)).toBe("error");
    expect(await status(page)).toContain("share_of_row is defined for sum and count, not for avg");
  });
});

test.describe("pivot calculated measures, the toolbar", () => {
  const focused = (page) =>
    inside(page, (root) => {
      const active = root.activeElement;
      return active
        ? {
            part: active.getAttribute("part"),
            role: active.getAttribute("role"),
            name: active.getAttribute("aria-label") ?? active.textContent,
            text: active.textContent,
            expanded: active.getAttribute("aria-expanded"),
          }
        : null;
    });
  const focus = (page, selector) => inside(page, (root, selector) => root.querySelector(selector).focus(), selector);
  const menu = (page) =>
    inside(page, (root) => {
      const menu = root.querySelector('[part="field-menu"]');
      return (
        menu && {
          role: menu.getAttribute("role"),
          name: menu.getAttribute("aria-label"),
          items: [...menu.children].map((item) => [
            item.getAttribute("role"),
            item.textContent,
            item.getAttribute("aria-checked"),
          ]),
        }
      );
    });
  const values = (page) => page.evaluate(() => document.querySelector("opengrid-pivot").getAttribute("values"));
  const record = (page) =>
    page.evaluate(() => {
      window.__views = [];
      document
        .querySelector("opengrid-pivot")
        .addEventListener("opengrid-view-change", (event) => window.__views.push(event.detail.view.values));
    });

  test("a sum and a count offer every calculation, an average the running total", async ({ page }) => {
    await open(page);
    await show(page, [SOLD, { fn: "count", as: "n" }, { field: "units", fn: "avg", as: "mean" }]);
    const buttons = await inside(page, (root) =>
      [...root.querySelectorAll('button[part="chip-calc"]')].map((button) => [
        button.dataset.calc,
        button.textContent,
        button.getAttribute("aria-label"),
        button.getAttribute("aria-haspopup"),
        button.getAttribute("aria-expanded"),
      ]),
    );
    expect(buttons).toEqual([
      ["sold", "value", "Show sold as: value", "menu", "false"],
      ["n", "value", "Show n as: value", "menu", "false"],
      ["mean", "value", "Show mean as: value", "menu", "false"],
    ]);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

    // No share of an average (P10): its menu has the value and the running total.
    await focus(page, 'button[data-calc="mean"]');
    await page.keyboard.press("Enter");
    expect(await menu(page)).toEqual({
      role: "menu",
      name: "Show mean as: value",
      items: [
        ["menuitemradio", "value", "true"],
        ["menuitemradio", "running total", "false"],
      ],
    });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect.poll(() => values(page)).toContain('{"field":"units","fn":"avg","as":"mean","calc":"running_total"}');
    await expect.poll(() => state(page)).toBe("ready");
    await expect
      .poll(() => focused(page))
      .toMatchObject({ part: "chip-calc", name: "Show mean as: running total" });
    // By hand: avg(units) is N 1·3·–, S –·2·–, A 1·2.5·–, W 5·-5, B 5·-5·–,
    // Total 3·0·–; each row adds up its own cells. The third measure of each
    // year is the average's running total.
    const running = (await body(page)).map((row) =>
      [2, 5, 8].map((at) => (row.cells[at] === "" ? null : Number(row.cells[at]))),
    );
    expect(running).toEqual([
      [1, 4, 4],
      [null, 2, 2],
      [1, 3.5, 3.5],
      [null, null, null],
      [5, 0, 0],
      [5, 0, 0],
      [3, 3, 3],
    ]);
  });

  test("a running total of what is no number is refused in the status line", async ({ page }) => {
    await open(page);
    await page.evaluate(() =>
      document
        .querySelector("opengrid-pivot")
        .setAttribute("values", '[{"field":"region","fn":"max","as":"last","calc":"running_total"}]'),
    );
    await expect.poll(() => state(page)).toBe("error");
    expect(await status(page)).toContain("measure last: running_total needs a number, and max of region is none");
    // A largest amount is one: 10.00·30.00·– for N, to the cent.
    await show(page, [{ field: "amount", fn: "max", as: "most", calc: "running_total" }]);
    expect((await body(page)).map((row) => row.cells)).toEqual([
      ["10.00", "40.00", "40.00"],
      ["", "20.00", "20.00"],
      ["10.00", "40.00", "40.00"],
      ["", "", ""],
      ["5.50", "0.00", "0.00"],
      ["5.50", "0.00", "0.00"],
      ["10.00", "40.00", "40.00"],
    ]);
  });

  test("a reader chooses a share from the keyboard, and takes it back", async ({ page }) => {
    await open(page);
    await record(page);
    await focus(page, 'button[data-calc="sold"]');
    await page.keyboard.press("Enter");

    // The menu: the five choices as radio items, the one in use checked and
    // focused, named as its button is.
    expect(await menu(page)).toEqual({
      role: "menu",
      name: "Show sold as: value",
      items: [
        ["menuitemradio", "value", "true"],
        ["menuitemradio", "share of row", "false"],
        ["menuitemradio", "share of column", "false"],
        ["menuitemradio", "share of total", "false"],
        ["menuitemradio", "running total", "false"],
      ],
    });
    expect(await focused(page)).toMatchObject({ role: "menuitemradio", text: "value" });
    expect(
      await inside(page, (root) => root.querySelector('button[data-calc="sold"]').getAttribute("aria-expanded")),
    ).toBe("true");
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

    // Escape closes and goes back to the button; nothing was chosen.
    await page.keyboard.press("Escape");
    expect(await menu(page)).toBeNull();
    expect(await focused(page)).toMatchObject({ part: "chip-calc", name: "Show sold as: value", expanded: "false" });
    expect(await page.evaluate(() => window.__views)).toEqual([]);

    // Arrows move, End and Home go to the ends, Enter picks.
    await page.keyboard.press("Enter");
    await page.keyboard.press("End");
    expect(await focused(page)).toMatchObject({ text: "running total" });
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowDown");
    expect(await focused(page)).toMatchObject({ text: "share of row" });
    await page.keyboard.press("Enter");

    // One write of the view, one query; the attribute carries the choice.
    await expect.poll(() => values(page)).toBe('[{"field":"units","fn":"sum","as":"sold","calc":"share_of_row"}]');
    await expect.poll(() => state(page)).toBe("ready");
    expect(await page.evaluate(() => window.__views)).toEqual([[{ ...SOLD, calc: "share_of_row" }]]);
    // The result is said: the header names the calculation, the status line
    // the answer, and the focus is back on the button, which now says it too.
    expect(await measureHeaders(page)).toEqual(Array(3).fill("sold · share of row"));
    expect(await status(page)).toBe("7 matches");
    await expect
      .poll(() => focused(page))
      .toMatchObject({ part: "chip-calc", name: "Show sold as: share of row", text: "share of row" });
    await expectCells(page, BY_HAND.share_of_row, "share_of_row, chosen");
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

    // The menu opens on the choice in use, and "value" takes the key out.
    await page.keyboard.press("Space");
    expect(await focused(page)).toMatchObject({ role: "menuitemradio", text: "share of row" });
    expect((await menu(page)).items.map(([, , checked]) => checked)).toEqual(["false", "true", "false", "false", "false"]);
    await page.keyboard.press("ArrowUp");
    await page.keyboard.press("Space");
    await expect.poll(() => values(page)).toBe('[{"field":"units","fn":"sum","as":"sold"}]');
    await expect.poll(() => focused(page)).toMatchObject({ part: "chip-calc", name: "Show sold as: value" });
    await expect.poll(() => measureHeaders(page)).toEqual(["sold", "sold", "sold"]);
    await expectCells(page, SUMS, "sum, again");
  });

  test("a choice is made with the pointer too", async ({ page }) => {
    await open(page);
    await page.locator('opengrid-pivot button[data-calc="sold"]').click();
    await page.locator('opengrid-pivot [role="menuitemradio"]', { hasText: "running total" }).click();
    await expect.poll(() => values(page)).toContain('"calc":"running_total"');
    await expect.poll(() => state(page)).toBe("ready");
    await expectCells(page, BY_HAND.running_total, "running_total, clicked");
  });

  test("right to left, the button and its menu follow the line", async ({ page }) => {
    await open(page, "?dir=rtl");
    const boxes = await inside(page, (root) => {
      const chip = root.querySelector('[part="chip"][data-field="sold"]');
      const box = (node) => node.getBoundingClientRect();
      return {
        title: box(chip.querySelector("span")).left,
        calc: box(chip.querySelector('[part="chip-calc"]')).left,
        remove: box(chip.querySelector('[part="chip-remove"]')).left,
      };
    });
    // The line reads from the right: title, what it is shown as, remove.
    expect(boxes.title).toBeGreaterThan(boxes.calc);
    expect(boxes.calc).toBeGreaterThan(boxes.remove);

    await page.locator('opengrid-pivot button[data-calc="sold"]').focus();
    await page.keyboard.press("Enter");
    const edges = await inside(page, (root) => {
      const button = root.querySelector('button[data-calc="sold"]').getBoundingClientRect();
      const menu = root.querySelector('[part="field-menu"]').getBoundingClientRect();
      const item = root.querySelector('[part="field-menu"] [aria-checked="true"]');
      return {
        inside: menu.left >= 0 && menu.right <= document.documentElement.clientWidth,
        under: menu.top >= button.bottom - 1,
        direction: getComputedStyle(item).direction,
      };
    });
    expect(edges).toEqual({ inside: true, under: true, direction: "rtl" });
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await expect.poll(() => values(page)).toContain('"calc":"share_of_row"');
    await expect.poll(() => state(page)).toBe("ready");
    await expectCells(page, BY_HAND.share_of_row, "share_of_row, right to left");
  });
});

test.describe("pivot calculated measures, the look and the export", () => {
  test("titles, formats and texts are the page's; the export stays raw and names the calculation", async ({
    page,
  }) => {
    await open(page);
    await show(page, [{ ...SOLD, calc: "share_of_row" }]);
    await page.evaluate(() => {
      const pivot = document.querySelector("opengrid-pivot");
      window.__opengridModule.set_columns(pivot, { sold: { title: "Units" } });
      window.__opengridModule.set_formats(pivot, {
        sold: { kind: "number", locale: "en-US", style: "percent", maximumFractionDigits: 1 },
      });
    });
    // A title or a format asks again: the answer drawn with both is awaited.
    await expect.poll(async () => (await body(page))[0]?.cells).toEqual(["25%", "75%", ""]);
    expect(await measureHeaders(page)).toEqual(Array(3).fill("Units · share of row"));
    expect((await body(page))[2].cells).toEqual(["16.7%", "83.3%", ""]);
    expect(
      await inside(page, (root) => root.querySelector('button[data-calc="sold"]').getAttribute("aria-label")),
    ).toBe("Show Units as: share of row");

    // The CSV: the header by alias and calculation, the values as fractions.
    const csv = await page.evaluate(() =>
      window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot"), { bom: false }),
    );
    const lines = csv.split("\r\n");
    expect(lines[0]).toBe(
      "zone,region,2023 · sold · share of row,2024 · sold · share of row,2025 · sold · share of row",
    );
    expect(lines[1]).toBe("A,N,0.25,0.75,");
    expect(lines[2]).toBe("A,S,,1.0,");
    expect(lines[7]).toBe("Total,,1.0,0.0,");

    // In German: the header, the button and the export's header.
    await page.evaluate(async () => {
      const { default: de } = await import("/packages/opengrid/texts/de.js");
      window.__opengridModule.set_texts(document.querySelector("opengrid-pivot"), de);
    });
    await expect.poll(() => measureHeaders(page)).toEqual(Array(3).fill("Units · Anteil an der Zeile"));
    await expect.poll(() => state(page)).toBe("ready");
    expect(
      await inside(page, (root) => {
        const button = root.querySelector('button[data-calc="sold"]');
        return [button.textContent, button.getAttribute("aria-label")];
      }),
    ).toEqual(["Anteil an der Zeile", "Units anzeigen als: Anteil an der Zeile"]);
    const german = await page.evaluate(() =>
      window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot"), { bom: false }),
    );
    expect(german.split("\r\n")[0]).toContain("2023 · sold · Anteil an der Zeile");
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });

  test("the rows behind a calculated cell are the cell's own", async ({ page }) => {
    await open(page);
    await page.evaluate(() => {
      window.__drills = [];
      const pivot = document.querySelector("opengrid-pivot");
      pivot.addEventListener("opengrid-drill-down", (event) => window.__drills.push(event.detail));
      pivot.setAttribute("drill-down", "");
    });
    await show(page, [{ ...SOLD, calc: "share_of_row" }]);
    const button = page.locator('opengrid-pivot button[data-cell="0:1"]');
    await expect(button).toHaveAttribute("aria-label", "0.75, show rows");
    await button.click();
    expect(await page.evaluate(() => window.__drills)).toEqual([
      {
        query: {
          source: "sales",
          filter: {
            and: [
              { field: "zone", op: "eq", value: "A" },
              { field: "region", op: "eq", value: "N" },
              { field: "year", op: "eq", value: 2024 },
            ],
          },
        },
        row: ["A", "N"],
        column: [2024],
        measure: "sold",
        value: 0.75,
      },
    ]);
    // A share that is NULL is no value: nothing to press.
    expect(await page.locator('opengrid-pivot button[data-cell="0:2"]').count()).toBe(0);
  });
});
