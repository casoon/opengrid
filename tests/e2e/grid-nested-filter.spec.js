import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The nested filter (issue #324): a filter with `or` across columns, which the
// filter row — one condition per column, and-ed — cannot hold. One tree in the
// wire form, written by the search expression and by the builder dialog, shown
// as one chip in words, carried by the view as `where`, and and-ed onto every
// query the grid sends.
//
// The fixture of `grid-query.spec.js`: 200 orders with a toolbar, the search
// and facets; it records every query. 52 orders are from DE, 16 have a qty of
// 900 or more, 66 are one or the other, 2 both; 21 are Alpha's.

const DE = { field: "country", op: "eq", value: "DE" };
const QTY = { field: "qty", op: "gte", value: 900 };
const ALPHA = { field: "customer", op: "eq", value: "Alpha" };
const DE_OR_QTY = "country is DE or qty greater or equal 900";

const grid = (page) => page.locator("opengrid-grid");
const dialog = (page) => grid(page).locator('[part="filter-builder-dialog"]');
const condition = (page, number) => dialog(page).getByRole("group", { name: `Condition ${number}`, exact: true });
// A condition's three fields, and the outermost group's choice.
const column = (page, number) => condition(page, number).locator('[data-dialog="column"]');
const operator = (page, number) => condition(page, number).locator('[data-dialog="op"]');
const value = (page, number) => condition(page, number).locator('[data-dialog="value"]');
const join = (page) => dialog(page).locator('[data-builder="join"]').first();

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-query.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("200 matches");
}

const status = (page) =>
  page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );

const view = (page) =>
  page.evaluate(() => window.__opengridModule.get_view(document.querySelector("opengrid-grid")));

const setView = (page, value) =>
  page.evaluate((value) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), value), value);

/** The chips: each one's words, and the names of its buttons. */
const chips = (page) =>
  page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('[part="chip"]')].map((chip) => ({
      text: chip.firstElementChild.textContent,
      edit: chip.querySelector("[data-chip-edit]")?.getAttribute("aria-label") ?? null,
      remove: chip.querySelector("[data-chip-remove]").getAttribute("aria-label"),
    })),
  );

/** Every query asked since `forget`, parsed. */
const queries = (page) => page.evaluate(() => window.__queries.map((json) => JSON.parse(json)));
const forget = (page) => page.evaluate(() => (window.__queries.length = 0));

/** The grid's last query for rows. */
const lastRows = async (page) => (await queries(page)).filter((query) => !query.group && "offset" in query).at(-1);

/** What has the focus inside the grid: the dialog's field by its `data-*`, a part, or a tag. */
const focused = (page) =>
  page.evaluate(() => {
    const active = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!active) return null;
    return (
      active.getAttribute("data-dialog") ??
      active.getAttribute("data-builder") ??
      active.getAttribute("data-dialog-action") ??
      active.getAttribute("part") ??
      active.localName
    );
  });

/** Types an expression into the search field and applies it. */
async function search(page, expression) {
  const input = grid(page).locator('[part="search-input"]');
  await input.fill(expression);
  await input.press("Enter");
}

/** Every distinct state of the status line from now on, oldest first. */
async function record(page) {
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const seen = [];
    const read = () => {
      const text = root.querySelector('[part="status"]')?.textContent.trim();
      if (text !== undefined && seen.at(-1) !== text) seen.push(text);
    };
    read();
    new MutationObserver(read).observe(root, { subtree: true, childList: true, characterData: true });
    window.__announced = seen;
  });
}
const announced = (page) => page.evaluate(() => window.__announced);

test.beforeEach(async ({ page }) => {
  await open(page);
});

test.describe("the search expression", () => {
  test("an or becomes the nested filter: one chip in words, the view's where, the query's filter", async ({ page }) => {
    await forget(page);
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toBe(`66 matches · Filter set: ${DE_OR_QTY}`);
    await expect(grid(page).locator('[part="search-input"]')).toHaveValue("");

    const tree = { or: [DE, QTY] };
    const now = await view(page);
    // Typed, as the wire writes a filter: 900 is a number.
    expect(now.where).toEqual(tree);
    // Not in the filter row: an or across columns has no place there.
    expect(now.filters).toEqual([]);
    expect(await chips(page)).toEqual([
      { text: DE_OR_QTY, edit: `Edit ${DE_OR_QTY}`, remove: `Remove ${DE_OR_QTY}` },
    ]);

    expect((await lastRows(page)).filter).toEqual(tree);
    const query = await page.evaluate(() => window.__opengridModule.get_query(document.querySelector("opengrid-grid")));
    expect(query.filter).toEqual(tree);
    // The facets count under it, like under the filter row.
    const counts = (await queries(page)).filter((query) => query.group);
    expect(counts.length).toBeGreaterThan(0);
    for (const count of counts) {
      expect(JSON.stringify(count.filter)).toContain(JSON.stringify(tree));
    }
  });

  test("and binds tighter than or, and parentheses say otherwise", async ({ page }) => {
    await search(page, "country = DE or qty >= 900 and customer = Alpha");
    await expect.poll(() => status(page)).toMatch(/^53 matches/);
    expect((await view(page)).where).toEqual({ or: [DE, { and: [QTY, ALPHA] }] });
    expect((await chips(page))[0].text).toBe(
      "country is DE or (qty greater or equal 900 and customer is Alpha)",
    );

    // A second expression replaces the first: there is one nested filter.
    await search(page, "(country = DE or qty >= 900) and customer = Alpha");
    await expect.poll(() => status(page)).toMatch(/^7 matches/);
    expect((await view(page)).where).toEqual({ and: [{ or: [DE, QTY] }, ALPHA] });
    expect(await chips(page)).toHaveLength(1);
    expect((await chips(page))[0].text).toBe(
      "(country is DE or qty greater or equal 900) and customer is Alpha",
    );
  });

  test("and alone still goes to the filter row, parentheses or not", async ({ page }) => {
    await search(page, "(country = DE and (qty >= 900))");
    await expect.poll(() => status(page)).toBe("2 matches");
    const now = await view(page);
    expect(now.filters).toEqual([
      { column: "country", op: "eq", value: "DE" },
      { column: "qty", op: "gte", value: "900" },
    ]);
    expect("where" in now).toBe(false);
    expect((await chips(page)).map((chip) => chip.edit)).toEqual([null, null]);
  });

  for (const [expression, sentence] of [
    ["(country = DE or qty >= 900", "A parenthesis is not closed"],
    ["country = DE or qty >= 900)", "A closing parenthesis has no opening one"],
    ["country = DE or ()", "A condition is missing"],
    ["(country = DE) qty >= 900", "“and” or “or” is missing before “qty >= 900”"],
    ["country = DE or colour = red", "colour is not a column of this grid"],
  ]) {
    test(`${expression} is named and kept for correcting`, async ({ page }) => {
      await search(page, expression);
      await expect.poll(() => status(page)).toBe(sentence);
      await expect(grid(page).locator('[part="search-input"]')).toHaveValue(expression);
      expect("where" in (await view(page))).toBe(false);
      expect(await chips(page)).toEqual([]);
    });
  }

  test("the page's joining words work, and the English ones still do", async ({ page }) => {
    await page.evaluate(() =>
      window.__opengridModule.set_texts(document.querySelector("opengrid-grid"), { queryAnd: "und", queryOr: "oder" }),
    );
    await expect.poll(() => status(page)).toBe("200 matches");
    await search(page, "country = DE ODER qty >= 900 und customer = Alpha");
    await expect.poll(() => status(page)).toMatch(/^53 matches/);
    // The chip says it with the page's words.
    expect((await chips(page))[0].text).toBe(
      "country is DE oder (qty greater or equal 900 und customer is Alpha)",
    );
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toMatch(/^66 matches/);
  });

  test("setting it is said once, with the result", async ({ page }) => {
    await record(page);
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toContain("66 matches");
    const said = (await announced(page)).filter((text) => text.includes("Filter set"));
    expect(said).toEqual([`66 matches · Filter set: ${DE_OR_QTY}`]);
  });
});

test.describe("the view", () => {
  test("a nested filter survives get_view and set_view, in one query", async ({ page }) => {
    await search(page, "country = DE or (qty >= 900 and (customer = Alpha or customer = Beta))");
    await expect.poll(() => status(page)).toMatch(/^\d+ matches · Filter set/);
    const matches = (await status(page)).split(" · ")[0];
    const saved = await view(page);
    expect(saved.where).toEqual({
      or: [DE, { and: [QTY, { or: [ALPHA, { field: "customer", op: "eq", value: "Beta" }] }] }],
    });

    await grid(page).locator('[part="chips-clear"]').click();
    await expect.poll(() => status(page)).toMatch(/^200 matches/);
    expect("where" in (await view(page))).toBe(false);

    await forget(page);
    await setView(page, saved);
    await expect.poll(() => status(page)).toBe(matches);
    expect(await view(page)).toEqual(saved);
    expect((await queries(page)).filter((query) => !query.group)).toHaveLength(1);
    expect((await lastRows(page)).filter).toEqual(saved.where);
    expect((await chips(page)).map((chip) => chip.text)).toEqual([
      "country is DE or (qty greater or equal 900 and (customer is Alpha or customer is Beta))",
    ]);

    // The view it already has costs nothing.
    await forget(page);
    await setView(page, saved);
    expect(await queries(page)).toEqual([]);
    // A view without the field has no nested filter.
    await setView(page, { ...saved, where: undefined });
    await expect.poll(() => status(page)).toBe("200 matches");
    expect("where" in (await view(page))).toBe(false);
  });

  for (const [name, where, sentence] of [
    ["an unknown column", { or: [DE, { field: "colour", op: "eq", value: "red" }] }, "where: colour is not a column of this grid"],
    ["not", { not: DE }, "where: not is not part of a view's filter"],
    ["in", { field: "country", op: "in", value: ["DE"] }, "where: in is not part of a view's filter"],
    ["an empty group", { or: [] }, "where: a group has no member"],
  ]) {
    test(`a where with ${name} is refused, and nothing of the view is applied`, async ({ page }) => {
      const before = await view(page);
      await setView(page, { sort: [{ field: "id", direction: "desc" }], where });
      await expect.poll(() => status(page)).toBe(sentence);
      expect(await view(page)).toEqual(before);
      expect(await chips(page)).toEqual([]);
    });
  }

  test("a value of the wrong type is the source's refusal, in the status line", async ({ page }) => {
    await setView(page, { where: { or: [DE, { field: "qty", op: "gte", value: "many" }] } });
    await expect
      .poll(() =>
        page.evaluate(() =>
          document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').getAttribute("data-state"),
        ),
      )
      .toBe("error");
    // Named by its place in the filter: the second member of the or.
    expect(await status(page)).toContain("filter.or[1].value");
  });
});

test.describe("with the other filters", () => {
  test("everything is and-ed: the page's filter, the filter row, a facet and the nested filter", async ({ page }) => {
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toMatch(/^66 matches/);

    // The filter row.
    await search(page, "customer = Alpha");
    await expect.poll(() => status(page)).toBe("7 matches");
    const row = { and: [ALPHA] };
    expect((await lastRows(page)).filter).toEqual({ and: [row, { or: [DE, QTY] }] });

    // A facet on top: the pills of country.
    const saved = await view(page);
    await setView(page, { ...saved, facets: { country: { values: ["DE"] } } });
    const expected = await page.evaluate(() => {
      const filter = {
        and: [
          { field: "customer", op: "eq", value: "Alpha" },
          { field: "country", op: "eq", value: "DE" },
        ],
      };
      return JSON.parse(window.__engine.execute(JSON.stringify({ source: "orders", select: ["id"], filter, limit: 1 })))
        .total_count;
    });
    await expect.poll(() => status(page)).toBe(`${expected} matches`);

    // The page's filter on top of all of it.
    const page50 = { field: "id", op: "lte", value: 50 };
    await page.evaluate(
      (filter) => document.querySelector("opengrid-grid").setAttribute("filter", JSON.stringify(filter)),
      page50,
    );
    await expect.poll(async () => (await lastRows(page)).filter.and?.[0]).toEqual(page50);
    const sent = (await lastRows(page)).filter;
    expect(JSON.stringify(sent)).toContain(JSON.stringify({ or: [DE, QTY] }));
    expect(JSON.stringify(sent)).toContain(JSON.stringify(ALPHA));
    // The nested filter is the reader's: it has its chip beside the others.
    expect((await chips(page)).map((chip) => chip.text)).toEqual([
      "country is DE",
      "customer is Alpha",
      DE_OR_QTY,
    ]);
  });

  test("the chip removes it as a whole, says so once, and the focus stays in the chips", async ({ page }) => {
    await search(page, "country = DE or qty >= 900");
    await search(page, "customer = Alpha");
    await expect.poll(() => status(page)).toBe("7 matches");
    await record(page);
    await grid(page).locator('[data-chip-remove="where"]').press("Enter");
    await expect.poll(() => status(page)).toBe(`21 matches · ${DE_OR_QTY} removed`);
    expect((await announced(page)).filter((text) => text.includes("removed"))).toHaveLength(1);
    expect("where" in (await view(page))).toBe(false);
    expect((await chips(page)).map((chip) => chip.text)).toEqual(["customer is Alpha"]);
    expect(await focused(page)).toBe("chip-remove");
  });

  test("remove all takes it, and so does the empty state's reset", async ({ page }) => {
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toMatch(/^66 matches/);
    await grid(page).locator('[part="chips-clear"]').click();
    await expect.poll(() => status(page)).toBe("200 matches · All filters removed");
    expect("where" in (await view(page))).toBe(false);

    // No row matches: the empty state knows a reader's filter is why.
    await search(page, "country = XX or qty > 100000");
    await expect.poll(() => status(page)).toMatch(/^No matches/);
    await grid(page).locator('[part="empty-reset"]').click();
    await expect.poll(() => status(page)).toMatch(/^200 matches/);
    expect(await chips(page)).toEqual([]);
  });

  test("the print table runs under it, and its caption says it", async ({ page }) => {
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toMatch(/^66 matches/);
    const rows = await page.evaluate(() =>
      window.__opengridModule.prepare_print(document.querySelector("opengrid-grid")),
    );
    expect(rows).toBe(66);
    const caption = await page.evaluate(
      () =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector("[data-print]")
          .shadowRoot.querySelector("caption").textContent,
    );
    expect(caption).toBe(`Bestellungen · 66 matches · ${DE_OR_QTY}`);
  });
});

test.describe("the builder", () => {
  const openBuilder = async (page) => {
    await grid(page).locator('[part="filter-builder"]').press("Enter");
    await expect(dialog(page)).toBeVisible();
  };

  /** The dialog's tree as the reader sees it. */
  const shown = (page) =>
    page.evaluate(() => {
      const read = (group) => ({
        join: group.querySelector(":scope > [data-join] select").value,
        members: [...group.querySelectorAll(":scope > [data-members] > *")].map((member) =>
          member.hasAttribute("data-cond")
            ? [
                member.getAttribute("aria-label"),
                member.querySelector('[data-dialog="column"]').value,
                member.querySelector('[data-dialog="op"]').value,
                member.querySelector('[data-dialog="value"]').value,
              ]
            : { name: member.querySelector(":scope > legend").textContent, ...read(member) },
        ),
      });
      const dialog = document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector('[part="filter-builder-dialog"]');
      return read(dialog.querySelector("[data-group]"));
    });

  test("is a modal dialog with a name, the focus inside and the grid behind it inert", async ({ page }) => {
    await openBuilder(page);
    const state = await page.evaluate(() => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const dialog = root.querySelector('[part="filter-builder-dialog"]');
      const search = root.querySelector('[part="search-input"]');
      search.focus();
      return {
        tag: dialog.localName,
        modal: dialog.matches(":modal"),
        name: root.getElementById(dialog.getAttribute("aria-labelledby")).textContent,
        // The grid behind a modal dialog takes no focus.
        outside: root.activeElement === search,
        inside: dialog.contains(root.activeElement),
      };
    });
    expect(state).toEqual({ tag: "dialog", modal: true, name: "Advanced filter", outside: false, inside: true });
    await expect(dialog(page).getByRole("heading", { name: "Advanced filter" })).toBeVisible();
    // It opens with one condition to fill, on the first column.
    expect(await shown(page)).toEqual({ join: "and", members: [["Condition 1", "id", "eq", ""]] });
    expect(await focused(page)).toBe("join");
  });

  test("every field has a name, and the buttons say what they remove", async ({ page }) => {
    await openBuilder(page);
    await dialog(page).getByRole("button", { name: "+ Condition" }).click();
    await expect(join(page)).toHaveAccessibleName("Rows match");
    await expect(join(page).locator("option")).toHaveText(["all conditions", "any condition"]);
    for (const number of [1, 2]) {
      await expect(column(page, number)).toHaveAccessibleName("Column");
      await expect(operator(page, number)).toHaveAccessibleName("Condition");
      await expect(value(page, number)).toHaveAccessibleName("Value");
      await expect(condition(page, number).getByRole("button")).toHaveAccessibleName(`Remove Condition ${number}`);
    }
    // An inner group is a fieldset with a legend, and its button names it.
    await dialog(page).getByRole("button", { name: "+ Group" }).click();
    const group = dialog(page).getByRole("group", { name: "Group 1", exact: true });
    await expect(group).toHaveJSProperty("localName", "fieldset");
    await expect(group.getByRole("button", { name: "Remove Group 1" })).toBeVisible();
  });

  test("builds an or with the keyboard, applies it, and gives the focus back to its button", async ({ page }) => {
    await record(page);
    await openBuilder(page);
    await page.keyboard.press("Tab");
    expect(await focused(page)).toBe("column");
    await join(page).selectOption("or");
    await column(page, 1).selectOption("country");
    // The conditions are the column's: a text has "contains", a number has not.
    expect(await operator(page, 1).locator("option").allTextContents()).toContain(
      "contains",
    );
    await operator(page, 1).selectOption("eq");
    await value(page, 1).fill("DE");

    await dialog(page).getByRole("button", { name: "+ Condition" }).press("Enter");
    // The focus is on the new condition's column.
    expect(await focused(page)).toBe("column");
    await column(page, 2).selectOption("qty");
    expect(
      await operator(page, 2).locator("option").allTextContents(),
    ).not.toContain("contains");
    await expect(value(page, 2)).toHaveAttribute("type", "number");
    await operator(page, 2).selectOption("gte");
    await value(page, 2).fill("900");
    // Enter in a value applies.
    await value(page, 2).press("Enter");

    await expect(dialog(page)).toHaveCount(0);
    await expect.poll(() => status(page)).toBe(`66 matches · Filter set: ${DE_OR_QTY}`);
    expect((await view(page)).where).toEqual({ or: [DE, QTY] });
    expect(await focused(page)).toBe("filter-builder");
    // Said once, with the result.
    expect((await announced(page)).filter((text) => text.includes("Filter set"))).toHaveLength(1);
  });

  test("opens from the chip on the tree as it stands, and gives the focus back to the chip", async ({ page }) => {
    await search(page, "(country = DE or qty >= 900) and customer = Alpha");
    await expect.poll(() => status(page)).toMatch(/^7 matches/);
    await grid(page).locator("[data-chip-edit]").press("Enter");
    await expect(dialog(page)).toBeVisible();
    expect(await shown(page)).toEqual({
      join: "and",
      members: [
        {
          name: "Group 1",
          join: "or",
          members: [
            ["Condition 1", "country", "eq", "DE"],
            ["Condition 2", "qty", "gte", "900"],
          ],
        },
        ["Condition 3", "customer", "eq", "Alpha"],
      ],
    });

    // Edit one value and apply: the same tree, changed.
    await value(page, 3).fill("Beta");
    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect(dialog(page)).toHaveCount(0);
    await expect.poll(() => status(page)).toContain("Filter set: (country is DE or qty greater or equal 900) and customer is Beta");
    expect((await view(page)).where).toEqual({
      and: [{ or: [DE, QTY] }, { field: "customer", op: "eq", value: "Beta" }],
    });
    expect(await focused(page)).toBe("chip-edit");
  });

  test("a group inside a group, and what is removed renumbers the rest", async ({ page }) => {
    await openBuilder(page);
    await column(page, 1).selectOption("customer");
    await operator(page, 1).selectOption("eq");
    await value(page, 1).fill("Alpha");
    await dialog(page).getByRole("button", { name: "+ Group" }).press("Enter");
    // The focus is on the new group's choice.
    expect(await focused(page)).toBe("join");
    const group = dialog(page).getByRole("group", { name: "Group 1", exact: true });
    await group.locator('[data-builder="join"]').selectOption("or");
    await column(page, 2).selectOption("country");
    await operator(page, 2).selectOption("eq");
    await value(page, 2).fill("DE");
    await group.getByRole("button", { name: "+ Condition" }).click();
    await column(page, 3).selectOption("qty");
    await operator(page, 3).selectOption("gte");
    await value(page, 3).fill("900");
    // One more, and away again: the numbers follow, the focus stays in the group.
    await group.getByRole("button", { name: "+ Condition" }).click();
    await expect(condition(page, 4)).toBeVisible();
    await condition(page, 2).getByRole("button", { name: "Remove Condition 2" }).press("Enter");
    await expect(condition(page, 4)).toHaveCount(0);
    expect(await focused(page)).toBe("add-condition");
    await expect(column(page, 2)).toHaveValue("qty");
    await column(page, 3).selectOption("country");
    await operator(page, 3).selectOption("eq");
    await value(page, 3).fill("DE");

    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect.poll(() => status(page)).toMatch(/^7 matches/);
    expect((await view(page)).where).toEqual({ and: [ALPHA, { or: [QTY, DE] }] });

    // Removing the group leaves its holder: a group of one is that one.
    await grid(page).locator("[data-chip-edit]").click();
    await dialog(page).getByRole("button", { name: "Remove Group 1" }).click();
    expect(await focused(page)).toBe("add-condition");
    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect.poll(() => status(page)).toMatch(/^21 matches/);
    expect((await view(page)).where).toEqual(ALPHA);
  });

  test("a missing value keeps the dialog open, names the column and marks the field", async ({ page }) => {
    await openBuilder(page);
    await column(page, 1).selectOption("qty");
    await operator(page, 1).selectOption("gte");
    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect(dialog(page)).toBeVisible();
    const problem = dialog(page).locator("[data-dialog-problem]");
    await expect(problem).toHaveText("qty: the value is missing");
    const field = value(page, 1);
    await expect(field).toBeFocused();
    await expect(field).toHaveAttribute("aria-invalid", "true");
    // Described by the sentence, so it is read with the field.
    await expect(field).toHaveAccessibleDescription("qty: the value is missing");
    expect("where" in (await view(page))).toBe(false);
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

    // Corrected, it applies, and the mark is gone with the dialog.
    await field.fill("900");
    await field.press("Enter");
    await expect(dialog(page)).toHaveCount(0);
    await expect.poll(() => status(page)).toMatch(/^16 matches/);
  });

  test("a condition without a value needs none", async ({ page }) => {
    await openBuilder(page);
    await join(page).selectOption("or");
    await column(page, 1).selectOption("country");
    await operator(page, 1).selectOption("is_null");
    await expect(value(page, 1)).toBeHidden();
    await dialog(page).getByRole("button", { name: "+ Condition" }).click();
    await column(page, 2).selectOption("country");
    await operator(page, 2).selectOption("eq");
    await value(page, 2).fill("DE");
    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect.poll(() => status(page)).toBe("58 matches · Filter set: country has no value or country is DE");
    expect((await view(page)).where).toEqual({ or: [{ field: "country", op: "is_null" }, DE] });
  });

  test("Escape and Cancel change nothing and give the focus back", async ({ page }) => {
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toMatch(/^66 matches/);
    const before = await view(page);
    await forget(page);

    await grid(page).locator("[data-chip-edit]").press("Enter");
    await value(page, 1).fill("FR");
    await page.keyboard.press("Escape");
    await expect(dialog(page)).toHaveCount(0);
    expect(await focused(page)).toBe("chip-edit");

    await openBuilder(page);
    await condition(page, 1).getByRole("button", { name: "Remove Condition 1" }).click();
    await dialog(page).getByRole("button", { name: "Cancel" }).press("Enter");
    await expect(dialog(page)).toHaveCount(0);
    expect(await focused(page)).toBe("filter-builder");

    expect(await view(page)).toEqual(before);
    expect(await queries(page)).toEqual([]);
  });

  test("with no condition left, Apply removes the nested filter", async ({ page }) => {
    await search(page, "country = DE or qty >= 900");
    await expect.poll(() => status(page)).toMatch(/^66 matches/);
    await grid(page).locator("[data-chip-edit]").click();
    await condition(page, 1).getByRole("button", { name: "Remove Condition 1" }).click();
    await condition(page, 1).getByRole("button", { name: "Remove Condition 1" }).click();
    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect.poll(() => status(page)).toBe(`200 matches · ${DE_OR_QTY} removed`);
    expect("where" in (await view(page))).toBe(false);
    // The chip is gone: the focus is on the toolbar's button.
    await expect(dialog(page)).toHaveCount(0);
    expect(await focused(page)).toBe("filter-builder");
  });

  test("a condition on a hidden column is kept as it is", async ({ page }) => {
    const where = { or: [DE, QTY] };
    await setView(page, { columns: { hidden: ["qty"] }, where });
    await expect.poll(() => status(page)).toBe("66 matches");
    await grid(page).locator("[data-chip-edit]").click();
    await expect(column(page, 2)).toBeDisabled();
    await expect(column(page, 2)).toHaveValue("qty");
    await expect(value(page, 2)).toHaveValue("900");
    await expect(value(page, 2)).toBeDisabled();
    await value(page, 1).fill("FR");
    await dialog(page).getByRole("button", { name: "Apply" }).click();
    await expect.poll(() => status(page)).toMatch(/^\d+ matches · Filter set: country is FR or qty greater or equal 900$/);
    // The number is still a number.
    expect((await view(page)).where).toEqual({ or: [{ ...DE, value: "FR" }, QTY] });
  });

  test("the dialog's keys are its own: the grid behind keeps its cell and its rows", async ({ page }) => {
    // A cell is the grid's active one, as after working in it.
    await grid(page).locator('td[data-row="2"][data-col="1"]').click();
    const active = () =>
      page.evaluate(() => {
        const root = document.querySelector("opengrid-grid").shadowRoot;
        const cell = root.querySelector('td[tabindex="0"], th[tabindex="0"]');
        return {
          cell: cell && `${cell.dataset.row}:${cell.dataset.col}`,
          sorted: root.querySelectorAll("th[aria-sort]").length,
          editor: !!root.querySelector('[part="editor"]'),
          scrollTop: root.querySelector('[part="viewport"]').scrollTop,
        };
      });
    const before = await active();
    expect(before.cell).toBe("2:1");
    await grid(page).locator('[part="filter-builder"]').click();
    await column(page, 1).selectOption("country");
    await value(page, 1).focus();
    await forget(page);
    for (const key of ["ArrowDown", "PageDown", "End", "F2", "a", " "]) await page.keyboard.press(key);
    // The keys went to the field, and to nothing else.
    await expect(value(page, 1)).toHaveValue("a ");
    expect(await active()).toEqual(before);
    expect(await queries(page)).toEqual([]);
  });

  for (const dir of ["ltr", "rtl"]) {
    test(`has no axe violations and stays inside the window (${dir})`, async ({ page }) => {
      await page.evaluate((dir) => document.querySelector("opengrid-grid").setAttribute("dir", dir), dir);
      await search(page, "(country = DE or qty >= 900) and customer = Alpha");
      await expect.poll(() => status(page)).toMatch(/^7 matches/);
      // The chip, with its two buttons.
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
      await grid(page).locator("[data-chip-edit]").click();
      await expect(dialog(page)).toBeVisible();
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

      const box = await dialog(page).boundingBox();
      const viewport = page.viewportSize();
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(viewport.width);
      // Nothing inside is wider than the dialog: no sideways scrolling.
      expect(await dialog(page).evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
      // The fields follow the direction: the column stands at the start.
      const first = await column(page, 1).boundingBox();
      const remove = await condition(page, 1).getByRole("button", { name: "Remove Condition 1" }).boundingBox();
      if (viewport.width > 600) {
        expect(dir === "rtl" ? first.x > remove.x : first.x < remove.x).toBe(true);
      }
    });
  }
});
