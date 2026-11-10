import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Lookup columns (issue #326): the page gives a column that stores keys their
// labels in `set_columns`. The cell, the editor, the filter row and the facets
// show and choose by label; the query, the sort, the groups and the view keep
// the key — and a sort by the key is not a sort by the label.
//
// The fixture: `customer_id` (int64) with 1 → Zeta GmbH, 2 → Alpha AG,
// 3 → Mid Ltd, and `country` (utf8) with DE → Germany, FR → France. Key 9 and
// `US` have no label; row 5 holds neither key.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) => new Function("root", "arg", `return (${fn})(root, arg)`)(
      document.querySelector("opengrid-grid").shadowRoot,
      arg,
    ),
    { fn: fn.toString(), arg },
  );

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent.trim());
const chips = (page) => shadow(page, (root) => [...root.querySelectorAll('[part="chip"]')].map((chip) => chip.querySelector("span").textContent));
/** The texts of a column's cells, in the order of the rows. */
const column = (page, col) =>
  shadow(page, (root, col) =>
    [...root.querySelectorAll("tbody tr[aria-rowindex]")]
      .sort((a, b) => Number(a.getAttribute("aria-rowindex")) - Number(b.getAttribute("aria-rowindex")))
      .map((tr) => tr.querySelector(`td[data-col="${col}"]`))
      .filter((td) => td && td.dataset.row !== undefined && !td.closest("tr").hidden && td.closest("tr").style.display !== "none")
      .map((td) => td.textContent),
    col);
const view = (page) => page.evaluate(() => window.__module.get_view(window.__host));
/** The last query for rows: the facets' counting queries select no `id`. */
const lastQuery = (page) => page.evaluate(() => window.__queries.filter((query) => query.select?.includes("id")).at(-1));

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-lookup.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => status(page)).toBe("6 matches");
  await expect.poll(async () => (await column(page, 1)).length).toBe(6);
}

async function startEdit(page, row, col) {
  await shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });
  await page.keyboard.press("Enter");
}

const combobox = (page) =>
  shadow(page, (root) => {
    const input = root.querySelector('[part="editor"] input');
    return {
      role: input?.getAttribute("role"),
      expanded: input?.getAttribute("aria-expanded"),
      controls: input?.getAttribute("aria-controls"),
      listbox: root.querySelector('[part~="listbox"]')?.id,
      options: [...root.querySelectorAll('[part~="listbox"] [role="option"]:not([hidden])')].map((option) => option.textContent),
      active: input?.getAttribute("aria-activedescendant"),
      focused: root.activeElement === input,
      value: input?.value,
    };
  });

/** The filter row's value control of a column: which is shown, and its options. */
const filterValue = (page, col) =>
  shadow(page, (root, col) => {
    const choice = root.querySelector(`select[data-value-col="${col}"]`);
    const operator = root.querySelector(`[part="filter-operator"][data-col="${col}"]`);
    return {
      input: root.querySelector(`input[data-col="${col}"]`).hidden,
      choice: choice.hidden,
      options: [...choice.options].map((option) => [option.value, option.textContent]),
      value: choice.value,
      name: choice.getAttribute("aria-label"),
      ops: operator.getAttribute("data-ops"),
      operatorHidden: operator.hidden,
    };
  }, col);

/** Chooses the option of `key` in a column's filter choice and applies it with Enter. */
async function choose(page, col, key) {
  await shadow(page, (root, { col, key }) => {
    const choice = root.querySelector(`select[data-value-col="${col}"]`);
    choice.value = key;
    choice.focus();
  }, { col, key });
  await page.keyboard.press("Enter");
}

/** A facet's values as drawn, in order: `[[label, count], …]`. */
const facet = (page, name) =>
  shadow(page, (root, name) =>
    [...root.querySelectorAll(`fieldset[data-facet="${name}"] [part="facet-value"], fieldset[data-facet="${name}"] [part="facet-pill"]`)].map((item) => [
      item.querySelector("span:not([part])").textContent,
      Number(item.querySelector('[part="facet-count"]').textContent),
    ]),
    name);

const axe = async (page) => (await new AxeBuilder({ page }).analyze()).violations;

test("a cell shows the label of its key, the key where it has none, and nothing for NULL", async ({ page }) => {
  await open(page);
  expect(await column(page, 1)).toEqual(["Mid Ltd", "Zeta GmbH", "Alpha AG", "9", "", "Zeta GmbH"]);
  expect(await column(page, 2)).toEqual(["Germany", "France", "Germany", "US", "", "France"]);
  // The other columns are what they were.
  expect(await column(page, 3)).toEqual(["10", "20", "30", "40", "50", "60"]);
  // Labels are text: the number column of keys lines up at the start, the amounts at the end.
  expect(await shadow(page, (root) => [1, 3].map((col) => root.querySelector(`td[data-row="0"][data-col="${col}"]`).dataset.align))).toEqual(["start", "end"]);
  expect(await axe(page)).toEqual([]);
});

test("sorting a lookup column sorts by the key, not by the label the reader sees", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('th[data-col="1"]').focus());
  await page.keyboard.press("Enter");
  // Keys 1, 1, 2, 3, 9, NULL: Zeta stands before Alpha, which no alphabet says.
  await expect.poll(() => column(page, 1)).toEqual(["Zeta GmbH", "Zeta GmbH", "Alpha AG", "Mid Ltd", "9", ""]);
  expect((await lastQuery(page)).sort[0]).toMatchObject({ field: "customer_id", direction: "asc" });
  expect(await shadow(page, (root) => root.querySelector('th[data-col="1"]').getAttribute("aria-sort"))).toBe("ascending");
  const labels = (await column(page, 1)).slice(0, 4);
  expect(labels).not.toEqual([...labels].sort());
});

test("the editor is a combobox over the labels; what it commits is the key", async ({ page }) => {
  await open(page);
  await startEdit(page, 0, 1);
  // The ARIA combobox: the text field controls the listbox, the focus stays in it.
  const opened = await combobox(page);
  expect(opened).toMatchObject({ role: "combobox", expanded: "false", focused: true, value: "Mid Ltd" });
  expect(opened.controls).toBe(opened.listbox);

  await page.keyboard.type("al");
  await expect.poll(() => combobox(page)).toMatchObject({ expanded: "true", options: ["Alpha AG"] });
  expect(await axe(page)).toEqual([]);
  await page.keyboard.press("ArrowDown");
  expect((await combobox(page)).active).toBeTruthy();
  await page.keyboard.press("Enter");

  await expect.poll(() => page.evaluate(() => window.__changes.length)).toBe(1);
  expect(await page.evaluate(() => window.__changes[0])).toMatchObject({ key: 1, column: "customer_id", value: "2", previous: "3" });
  expect((await column(page, 1))[0]).toBe("Alpha AG");
  expect(await shadow(page, (root) => !!root.querySelector('[part="editor"]'))).toBe(false);
});

test("every label is offered in the map's order, and a text that is none of them is refused", async ({ page }) => {
  await open(page);
  // The cell's own label is selected in the field; emptied, every label fits.
  await startEdit(page, 0, 2);
  await page.keyboard.press("Backspace");
  await expect.poll(async () => (await combobox(page)).options).toEqual(["Germany", "France"]);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  await startEdit(page, 0, 1);
  await page.keyboard.press("Backspace");
  // JavaScript lists whole-number keys ascending: 1, 2, 3.
  await expect.poll(async () => (await combobox(page)).options).toEqual(["Zeta GmbH", "Alpha AG", "Mid Ltd"]);
  await page.keyboard.type("Nobody Inc");
  await page.keyboard.press("Enter");
  expect(await shadow(page, (root) => root.querySelector('[part="validation"]')?.textContent)).toBe("“Nobody Inc” is not one of the choices");
  expect(await page.evaluate(() => window.__changes)).toHaveLength(0);
  expect((await combobox(page)).focused).toBe(true);
});

test("set_choices narrows what the editor offers, named by the lookup's labels", async ({ page }) => {
  await open(page);
  await page.evaluate(() => window.__module.set_choices(window.__host, { customer_id: [2, 3] }));
  await startEdit(page, 1, 1);
  // The cell's key is none of the choices; it is still named by its label.
  expect((await combobox(page)).value).toBe("Zeta GmbH");
  await page.keyboard.press("Backspace");
  await expect.poll(async () => (await combobox(page)).options).toEqual(["Alpha AG", "Mid Ltd"]);
});

test("the filter row offers a choice among the labels and filters by the key", async ({ page }) => {
  await open(page);
  expect(await filterValue(page, 1)).toEqual({
    input: true,
    choice: false,
    options: [["", "any"], ["1", "Zeta GmbH"], ["2", "Alpha AG"], ["3", "Mid Ltd"]],
    value: "",
    name: "Customer value",
    ops: "eq ne is_null is_not_null",
    operatorHidden: false,
  });
  expect((await filterValue(page, 2)).options).toEqual([["", "any"], ["DE", "Germany"], ["FR", "France"]]);
  // A column without a lookup keeps its typed field.
  expect(await filterValue(page, 3)).toMatchObject({ input: false, choice: true });
  // The comparison's button sits inside the field, and the choice's text starts after it.
  const inside = await shadow(page, (root) => {
    const operator = root.querySelector('[part="filter-operator"][data-col="1"]').getBoundingClientRect();
    const choice = root.querySelector('select[data-value-col="1"]');
    return { button: operator.width, room: parseFloat(getComputedStyle(choice).paddingInlineStart) };
  });
  expect(inside.room).toBeGreaterThanOrEqual(inside.button);

  await choose(page, 1, "1");
  await expect.poll(() => status(page)).toBe("2 matches");
  expect(await column(page, 1)).toEqual(["Zeta GmbH", "Zeta GmbH"]);
  // The query and the view hold the key; the chip names the label.
  expect((await lastQuery(page)).filter).toEqual({ and: [{ field: "customer_id", op: "eq", value: 1 }] });
  expect((await view(page)).filters).toEqual([{ column: "customer_id", op: "eq", value: "1" }]);
  expect(await chips(page)).toEqual(["Customer is Zeta GmbH"]);
  expect(await page.evaluate(() => window.__module.get_query(window.__host).filter)).toEqual({
    and: [{ field: "customer_id", op: "eq", value: 1 }],
  });
  expect(await axe(page)).toEqual([]);

  // *is not*, from the operator's menu.
  await shadow(page, (root) => root.querySelector('[part="filter-operator"][data-col="1"]').click());
  expect(await shadow(page, (root) => [...root.querySelectorAll('[part="operator-menu"] [role="menuitemradio"]')].map((item) => item.dataset.op))).toEqual([
    "eq",
    "ne",
    "is_null",
    "is_not_null",
  ]);
  await shadow(page, (root) => root.querySelector('[part="operator-menu"] [data-op="ne"]').click());
  await expect.poll(() => status(page)).toBe("3 matches");
  expect(await column(page, 1)).toEqual(["Mid Ltd", "Alpha AG", "9"]);
});

test("a view's filter on a key without a label is kept, named by the key", async ({ page }) => {
  await open(page);
  await page.evaluate(() => window.__module.set_view(window.__host, { filters: [{ column: "customer_id", op: "eq", value: "9" }] }));
  await expect.poll(() => status(page)).toBe("1 match");
  expect(await filterValue(page, 1)).toMatchObject({ value: "9" });
  expect((await filterValue(page, 1)).options.at(-1)).toEqual(["9", "9"]);
  expect(await chips(page)).toEqual(["Customer is 9"]);
  expect((await view(page)).filters).toEqual([{ column: "customer_id", op: "eq", value: "9" }]);
});

test("the Add filter dialog offers the labels too", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('[data-toolbar="add-filter"]').click());
  const dialog = (fn) => shadow(page, `(root) => (${fn})(root.querySelector('[part="filter-dialog"]'))`);
  await page.evaluate(() => {
    const dialog = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="filter-dialog"]');
    const pick = dialog.querySelector('[data-dialog="column"]');
    pick.value = "country";
    pick.dispatchEvent(new Event("change", { bubbles: true }));
  });
  expect(
    await page.evaluate(() => {
      const dialog = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="filter-dialog"]');
      const value = dialog.querySelector('[data-dialog="value"]');
      return {
        tag: value.localName,
        labelled: value.closest("label").querySelector("span").textContent,
        options: [...value.options].map((option) => [option.value, option.textContent]),
        ops: [...dialog.querySelector('[data-dialog="op"]').options].map((option) => option.value),
      };
    }),
  ).toEqual({
    tag: "select",
    labelled: "Value",
    options: [["DE", "Germany"], ["FR", "France"]],
    ops: ["eq", "ne", "is_null", "is_not_null"],
  });
  void dialog;
  expect(await axe(page)).toEqual([]);
  await page.evaluate(() => {
    const dialog = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="filter-dialog"]');
    dialog.querySelector('[data-dialog="value"]').value = "FR";
    dialog.querySelector('[data-dialog-action="apply"]').click();
  });
  await expect.poll(() => status(page)).toBe("2 matches");
  expect(await chips(page)).toEqual(["Country is France"]);
  expect(await filterValue(page, 2)).toMatchObject({ value: "FR" });

  // A column without a lookup gets its typed field back.
  await shadow(page, (root) => root.querySelector('[data-toolbar="add-filter"]').click());
  expect(
    await page.evaluate(() => {
      const dialog = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="filter-dialog"]');
      const pick = dialog.querySelector('[data-dialog="column"]');
      pick.value = "amount";
      pick.dispatchEvent(new Event("change", { bubbles: true }));
      const value = dialog.querySelector('[data-dialog="value"]');
      return [value.localName, value.type];
    }),
  ).toEqual(["input", "number"]);
});

test("the facets list the keys the rows hold by their labels, in the keys' order", async ({ page }) => {
  await open(page);
  // A list facet over an int64 column: a lookup makes it one.
  await expect.poll(() => facet(page, "customer_id")).toEqual([
    ["Zeta GmbH", 2],
    ["Alpha AG", 1],
    ["Mid Ltd", 1],
    ["9", 1],
    ["(no value)", 1],
  ]);
  expect(await facet(page, "country")).toEqual([["Germany", 2], ["France", 2], ["US", 1], ["(no value)", 1]]);
  expect(await shadow(page, (root) => [...root.querySelectorAll('[part="facet"] legend')].map((legend) => legend.textContent))).toEqual(["Customer", "Country"]);

  await shadow(page, (root) => {
    const box = [...root.querySelectorAll('input[data-facet="customer_id"]')].find((input) => input.dataset.key === "2");
    box.checked = true;
    box.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  });
  await expect.poll(() => status(page)).toBe("1 match");
  expect(await column(page, 1)).toEqual(["Alpha AG"]);
  expect(await chips(page)).toEqual(["Customer is Alpha AG"]);
  // The view holds the key.
  expect((await view(page)).facets).toEqual({ customer_id: { values: [2] } });

  await shadow(page, (root) => [...root.querySelectorAll('[part="facet-pill"]')].find((pill) => pill.dataset.key === '"DE"').click());
  await expect.poll(() => chips(page)).toEqual(["Customer is Alpha AG", "Country is Germany"]);
  expect(await axe(page)).toEqual([]);
});

test("grouped by a lookup column, the groups are the keys in their order, named by their labels", async ({ page }) => {
  await open(page);
  await page.evaluate(() => window.__host.setAttribute("group-by", "customer_id"));
  // The groups start closed: the rows drawn are their headers and the total.
  const groups = () =>
    shadow(page, (root) =>
      [...root.querySelectorAll("tbody tr")]
        .filter((tr) => tr.style.transform && !tr.style.display)
        .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
        .map((tr) => tr.querySelector("td[data-col]")?.textContent ?? ""),
    );
  await expect.poll(groups).toEqual([
    "Customer: Zeta GmbH (2 rows)",
    "Customer: Alpha AG (1 row)",
    "Customer: Mid Ltd (1 row)",
    "Customer: 9 (1 row)",
    "Customer: (no value) (1 row)",
    "Total (6 rows)",
  ]);
  expect(await axe(page)).toEqual([]);
});

test("a search expression compares the key, with = and ≠ only", async ({ page }) => {
  await open(page);
  const search = async (text) => {
    await shadow(page, (root) => root.querySelector('[part="search-input"]').focus());
    await page.keyboard.press("Control+a");
    await page.keyboard.type(text);
    await page.keyboard.press("Enter");
  };
  await search("customer_id = 2");
  await expect.poll(() => status(page)).toBe("1 match");
  expect(await filterValue(page, 1)).toMatchObject({ value: "2" });
  expect(await chips(page)).toEqual(["Customer is Alpha AG"]);

  await search("country ~ Ger");
  await expect.poll(() => status(page)).toContain("~");
  expect(await chips(page)).toEqual(["Customer is Alpha AG"]);
});

test("the print table shows the labels, and its caption names the filter by label", async ({ page }) => {
  await open(page);
  await choose(page, 2, "DE");
  await expect.poll(() => status(page)).toBe("2 matches");
  expect(await page.evaluate(() => window.__module.prepare_print(window.__host))).toBe(2);
  const printed = await page.evaluate(() => {
    const table = document.querySelector("opengrid-grid").shadowRoot.querySelector("[data-print]").shadowRoot.querySelector("table");
    return {
      caption: table.querySelector("caption").textContent,
      rows: [...table.querySelectorAll("tbody tr")].map((tr) => [...tr.children].map((cell) => cell.textContent)),
    };
  });
  expect(printed.caption).toBe("Orders · 2 matches · Country is Germany");
  expect(printed.rows).toEqual([
    ["1", "Mid Ltd", "Germany", "10"],
    ["3", "Alpha AG", "Germany", "30"],
  ]);
});

test("a lookup column is counted and nothing else: its menu offers no other aggregate", async ({ page }) => {
  await open(page);
  const aggregatesOf = async (col) => {
    await shadow(page, (root, col) => root.querySelector(`th[data-col="${col}"] [part="column-menu-button"]`).click(), col);
    const actions = await shadow(page, (root) =>
      [...root.querySelectorAll('[part="column-menu"] [data-action]')].map((item) => item.dataset.action).filter((action) => action.startsWith("aggregate:")));
    await page.keyboard.press("Escape");
    return actions;
  };
  // `amount` is an int64 like `customer_id`, and has the aggregates of a number.
  expect(await aggregatesOf(3)).toContain("aggregate:sum");
  expect(await aggregatesOf(1)).toEqual([]);

  // A view that asks for one all the same is told so, when the groups are asked for.
  await page.evaluate(() => window.__module.set_view(window.__host, { group: ["country"], aggregates: { customer_id: "sum", amount: "sum" } }));
  await expect.poll(() => status(page)).toContain("customer_id: sum is not an aggregate for a lookup column");
});

test("a lookup the column cannot have is reported, and nothing of the call applied", async ({ page }) => {
  await open(page);
  const refuse = async (columns) => {
    await page.evaluate((columns) => window.__module.set_columns(window.__host, columns), columns);
    return status(page);
  };
  expect(await refuse({ customer_id: { title: "Buyer", lookup: { abc: "A" } } })).toContain(
    "customer_id: lookup: abc is not a value of this column",
  );
  // The title of the refused call is not applied, and the labels of the last accepted one stand.
  expect(await shadow(page, (root) => root.querySelector('th[data-col="1"] [part~="header-label"]')?.textContent ?? root.querySelector('th[data-col="1"]').textContent)).toContain("Customer");
  expect((await column(page, 1))[0]).toBe("Mid Ltd");

  expect(await refuse({ customer_id: { lookup: [[1, "One"]] } })).toContain("customer_id: lookup is not a map of keys to labels");
  // A `Map` is not a plain object (it does not survive the way into the page, so it is made there).
  await page.evaluate(() => window.__module.set_columns(window.__host, { country: { lookup: new Map([["DE", "Germany"]]) } }));
  expect(await status(page)).toContain("country: lookup is not a map of keys to labels");
  expect(await refuse({ country: { lookup: { DE: "" } } })).toContain("country: lookup: DE has no label");
  expect(await refuse({ customer_id: { lookup: { 1: "One" }, aggregate: "sum" } })).toContain(
    "customer_id: sum is not an aggregate for a lookup column",
  );
  expect(await refuse({ customer_id: { lookup: { 1: "One" }, facet: "range" } })).toContain(
    "customer_id: range is not a facet for a lookup column",
  );

  // New labels are a new call with the whole configuration.
  await page.evaluate(() =>
    window.__module.set_columns(window.__host, { ...window.__columns, customer_id: { title: "Customer", lookup: { 3: "Third" } } }),
  );
  await expect.poll(() => column(page, 1)).toEqual(["Third", "1", "2", "9", "", "1"]);
  expect((await filterValue(page, 1)).options).toEqual([["", "any"], ["3", "Third"]]);
});

test("right to left: labels, the choice and the combobox work, with no axe violations", async ({ page }) => {
  await open(page, "?dir=rtl");
  expect(await shadow(page, (root) => getComputedStyle(root.querySelector('[part="viewport"]')).direction)).toBe("rtl");
  expect(await column(page, 1)).toEqual(["Mid Ltd", "Zeta GmbH", "Alpha AG", "9", "", "Zeta GmbH"]);
  expect(await axe(page)).toEqual([]);

  await choose(page, 2, "FR");
  await expect.poll(() => status(page)).toBe("2 matches");
  expect(await chips(page)).toEqual(["Country is France"]);

  await startEdit(page, 0, 1);
  await page.keyboard.type("mid");
  await expect.poll(() => combobox(page)).toMatchObject({ expanded: "true", options: ["Mid Ltd"] });
  // The list hangs under its cell, from the cell's start: the right edge.
  const edges = await shadow(page, (root) => {
    const cell = root.querySelector('[part="editor"]').closest("td").getBoundingClientRect();
    const list = root.querySelector('[part~="listbox"]').getBoundingClientRect();
    return { cell: Math.round(cell.right), list: Math.round(list.right), below: list.top >= cell.top };
  });
  expect(Math.abs(edges.cell - edges.list)).toBeLessThanOrEqual(2);
  expect(edges.below).toBe(true);
  expect(await axe(page)).toEqual([]);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__changes.at(-1))).toMatchObject({ column: "customer_id", value: "3" });
});
