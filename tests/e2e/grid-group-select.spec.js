import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Selecting by key (issue #322).
//
// A display position names a header as often as a row and moves with every
// group that opens, so a grouped grid selects records by their `row-key` — and
// without one it selects nothing. The claim under test is the issue's own:
// opening and closing a group never changes what is selected.
//
// The same model holds for every grid with a `row-key`, grouped or not: the
// selection is the records' keys, the event is `{ count, keys }`, and the view
// carries it as `selected`.
//
// The fixture groups the 200 conformance rows by country and customer: DE has
// 52 rows and nine customers, the first of them Alpha with six rows.

const shadow = (page, fn, arg) =>
  page.evaluate(
    ({ fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(
        document.querySelector("opengrid-grid").shadowRoot,
        arg,
      ),
    { fn: fn.toString(), arg },
  );

/** The rows drawn, in display order: what each is, and what it says of a selection. */
const drawn = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr[aria-rowindex]")]
      .map((tr) => ({
        position: Number(tr.querySelector("td[data-row]").getAttribute("data-row")),
        kind: tr.getAttribute("data-kind") ?? "row",
        selected: tr.getAttribute("aria-selected"),
        text: tr.querySelector('td[data-col="0"]').textContent,
      }))
      .sort((a, b) => a.position - b.position),
  );

/** The ids of the data rows drawn as selected, ascending. */
const selectedIds = async (page) =>
  (await drawn(page))
    .filter((row) => row.kind === "row" && row.selected === "true")
    .map((row) => Number(row.text))
    .sort((a, b) => a - b);

const rowcount = (page) =>
  shadow(page, (root) => Number(root.querySelector("table").getAttribute("aria-rowcount")));

const status = (page) => shadow(page, (root) => root.querySelector('[part="status"]').textContent);

const headerCheck = (page) =>
  shadow(page, (root) => root.querySelector('[part="select-mark"]').getAttribute("aria-checked"));

const events = (page) => page.evaluate(() => window.__selections);

/** The last selection the page heard of, its keys ascending. */
const heard = async (page) => {
  const last = (await events(page)).at(-1);
  return last && { ...last, keys: [...last.keys].sort((a, b) => a - b) };
};

async function press(page, position, key, col = 0) {
  await shadow(
    page,
    (root, { position, col }) =>
      root.querySelector(`td[data-row="${position}"][data-col="${col}"]`).focus(),
    { position, col },
  );
  await page.keyboard.press(key);
}

async function open(page, query = "") {
  await page.goto(`/tests/e2e/fixtures/grid-group-select.html${query}`);
  await page.waitForFunction(() => window.__ready === true);
  // Five countries (one of them "no value") and the total.
  await expect.poll(() => rowcount(page)).toBe(7);
}

/** Opens DE and, under it, Alpha: its six rows sit at positions 2 to 7. */
async function openAlpha(page) {
  await press(page, 0, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9);
  await press(page, 1, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9 + 6);
  await expect.poll(async () => (await drawn(page))[2]?.kind).toBe("row");
}

/** Groups by country, customer and flag, and opens DE, Alpha and its `true`. */
async function openThreeLevels(page) {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country,customer,flag"),
  );
  await expect.poll(async () => (await drawn(page))[0]?.text).toBe("country: DE (52 rows)");
  await press(page, 0, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9);
  await press(page, 1, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9 + 3);
  // DE, Alpha, false, true: the two rows of `true` are the ids 61 and 197.
  await press(page, 3, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9 + 3 + 2);
  await expect.poll(async () => (await drawn(page))[4]?.kind).toBe("row");
}

test("Space selects a record by its key, and only data rows say aria-selected", async ({ page }) => {
  await open(page);
  await openAlpha(page);
  const rows = await drawn(page);
  const id = Number(rows[3].text);

  await press(page, 3, " ", 1);
  await expect.poll(() => selectedIds(page)).toEqual([id]);
  // The page hears the record, not a place: no `rows`, no `ranges`.
  expect(await events(page)).toEqual([{ count: 1, keys: [id] }]);

  for (const row of await drawn(page)) {
    if (row.kind === "row") {
      expect(row.selected, `row ${row.text}`).toBe(Number(row.text) === id ? "true" : "false");
    } else {
      // A group header and the total are not records.
      expect(row.selected, row.text).toBeNull();
    }
  }
  expect(
    await shadow(page, (root) => root.querySelector("table").getAttribute("aria-multiselectable")),
  ).toBe("true");
  expect(await headerCheck(page)).toBe("mixed");

  // The selection cell of a row does the same, by the pointer too.
  await shadow(page, (root) =>
    root.querySelector('tr:has(td[data-row="4"]) > td[data-select="row"]').click(),
  );
  await expect.poll(async () => (await heard(page)).count).toBe(2);
  // And `Space` again deselects.
  await press(page, 3, " ", 1);
  await expect.poll(() => selectedIds(page)).toEqual([Number(rows[4].text)]);

  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("a group header selects nothing: Space on it opens and closes", async ({ page }) => {
  await open(page);
  await press(page, 0, " ");
  await expect.poll(() => rowcount(page)).toBe(7 + 9);
  await press(page, 0, " ");
  await expect.poll(() => rowcount(page)).toBe(7);
  // The total: nothing to select, nothing to open.
  await press(page, 5, " ");
  // Its selection cell is part of the header row: a click there toggles too.
  await shadow(page, (root) =>
    root.querySelector('tr:has(td[data-row="0"]) > td[data-select="row"]').click(),
  );
  await expect.poll(() => rowcount(page)).toBe(7 + 9);
  await page.waitForTimeout(150);
  expect(await events(page)).toEqual([]);
  for (const row of await drawn(page)) expect(row.selected).toBeNull();
});

test("two levels: opening and closing a group never changes what is selected", async ({ page }) => {
  await open(page);
  await openAlpha(page);
  const rows = await drawn(page);
  const picked = [Number(rows[2].text), Number(rows[5].text)].sort((a, b) => a - b);
  await press(page, 2, " ");
  await press(page, 5, " ");
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  const said = await events(page);
  expect(said.at(-1)).toEqual({ count: 2, keys: picked });

  // Alpha closes over both, and opens again.
  await press(page, 1, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9);
  expect(await selectedIds(page)).toEqual([]);
  expect(await headerCheck(page)).toBe("mixed");
  await press(page, 1, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9 + 6);
  await expect.poll(() => selectedIds(page)).toEqual(picked);

  // The level above closes over Alpha and its rows, and opens again.
  await press(page, 0, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7);
  await press(page, 0, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9 + 6);
  await expect.poll(() => selectedIds(page)).toEqual(picked);

  // A group that opens *below* moves nothing; one that closes above moves
  // every row — the records stay the selected ones either way.
  await press(page, 8, "Enter");
  await expect.poll(async () => (await drawn(page))[9]?.kind).toBe("row");
  expect(await selectedIds(page)).toEqual(picked);

  // Not one of the six toggles was a selection change.
  expect(await events(page)).toEqual(said);
  expect((await heard(page)).keys).toEqual(picked);
});

test("three levels: opening and closing a group never changes what is selected", async ({
  page,
}) => {
  await open(page);
  await openThreeLevels(page);
  await press(page, 4, " ");
  await press(page, 5, "Shift+ ");
  await expect.poll(() => selectedIds(page)).toEqual([61, 197]);
  const said = await events(page);
  expect((await heard(page)).keys).toEqual([61, 197]);

  // Every level in turn closes over the two rows and opens again: the
  // innermost (flag: true), the middle (Alpha), the outermost (DE).
  for (const [header, closed] of [
    [3, 7 + 9 + 3],
    [1, 7 + 9],
    [0, 7],
  ]) {
    await press(page, header, "Enter");
    await expect.poll(() => rowcount(page)).toBe(closed);
    expect(await selectedIds(page)).toEqual([]);
    await press(page, header, "Enter");
    await expect.poll(() => rowcount(page)).toBe(7 + 9 + 3 + 2);
    await expect.poll(() => selectedIds(page)).toEqual([61, 197]);
  }
  // `flag: false` opens above them: they sit three positions further down.
  await press(page, 2, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 9 + 3 + 2 + 3);
  await expect.poll(() => selectedIds(page)).toEqual([61, 197]);
  expect((await drawn(page)).filter((row) => row.selected === "true").map((row) => row.position)).toEqual(
    [7, 8],
  );

  expect(await events(page)).toEqual(said);
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("Shift+Space takes the data rows of the display list: headers passed over, closed groups left out", async ({
  page,
}) => {
  await open(page);
  await openAlpha(page);
  // DE, Alpha, its six rows, Beta (closed), the third customer — opened here.
  await press(page, 9, "Enter");
  await expect.poll(async () => (await drawn(page))[10]?.kind).toBe("row");
  const rows = await drawn(page);
  expect(rows[8]).toMatchObject({ kind: "group", text: expect.stringMatching(/^customer: Beta/) });

  // From Alpha's second row to the first row of the third customer.
  await press(page, 3, " ");
  await press(page, 10, "Shift+ ");
  const expected = [...rows.slice(3, 8), rows[10]].map((row) => Number(row.text)).sort((a, b) => a - b);
  await expect.poll(async () => (await heard(page)).keys).toEqual(expected);
  expect(await selectedIds(page)).toEqual(expected);
  // Two headers lie in the range, and neither says anything of a selection.
  const after = await drawn(page);
  expect(after[8].selected).toBeNull();
  expect(after[9].selected).toBeNull();

  // Beta opens: its rows were never in the list, so none of them is selected.
  await press(page, 8, "Enter");
  await expect.poll(async () => (await drawn(page))[9]?.kind).toBe("row");
  expect(await selectedIds(page)).toEqual(expected.filter((id) => rows.slice(3, 8).some((row) => Number(row.text) === id)));
  expect((await heard(page)).count).toBe(6);
});

test("Shift+Space reaches past the loaded window, and falls back to a toggle without an anchor in the list", async ({
  page,
}) => {
  await open(page);
  // One level, so a group holds more rows than the window of twelve: DE's 52.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country"),
  );
  await expect.poll(async () => (await drawn(page))[0]?.text).toBe("country: DE (52 rows)");
  await press(page, 0, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 52);
  await expect.poll(async () => (await drawn(page))[1]?.kind).toBe("row");

  // Without an anchor, `Shift` is a plain toggle.
  await press(page, 1, "Shift+ ");
  await expect.poll(async () => (await heard(page))?.count).toBe(1);

  // Scrolled until the row at position 40 is drawn: most of the rows from
  // the anchor to it are loaded no longer.
  const scrollTo = (position) =>
    shadow(
      page,
      (root, position) => {
        const height = root.querySelector("tbody td").getBoundingClientRect().height;
        root.querySelector('[part~="viewport"]').scrollTop = position * height;
      },
      position,
    );
  await scrollTo(46);
  await expect.poll(async () => (await drawn(page)).some((row) => row.position === 40)).toBe(true);
  expect((await drawn(page)).some((row) => row.position === 20)).toBe(false);
  // Without scrolling to the cell: the window stays the one just checked.
  await shadow(page, (root) =>
    root.querySelector('td[data-row="40"][data-col="0"]').focus({ preventScroll: true }),
  );
  await page.keyboard.press("Shift+ ");
  await expect.poll(async () => (await heard(page)).count).toBe(40);
  const de = (await heard(page)).keys;
  expect(new Set(de).size).toBe(40);
  // One query for the range — it lies in one group — and not one per row.
  const ranged = await page.evaluate(() =>
    window.__queries.map((query) => JSON.parse(query)).filter((query) => query.limit === 40),
  );
  expect(ranged).toHaveLength(1);
  expect(ranged[0]).toMatchObject({
    offset: 0,
    filter: { field: "country", op: "eq", value: "DE" },
  });

  // DE — the anchor's group — closes, and FR opens: the anchor has no place
  // in the list, so `Shift` toggles the one row.
  await scrollTo(0);
  await expect.poll(async () => (await drawn(page))[0]?.position).toBe(0);
  await press(page, 0, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7);
  await press(page, 1, "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 46);
  await expect.poll(async () => (await drawn(page))[2]?.kind).toBe("row");
  const fr = Number((await drawn(page))[2].text);
  await press(page, 2, "Shift+ ");
  await expect
    .poll(async () => (await heard(page)).keys)
    .toEqual([...de, fr].sort((a, b) => a - b));
});

test("select-all is every row of the filter, open or not — and the header clears it", async ({
  page,
}) => {
  await open(page);
  // Every group is closed: not one data row is loaded.
  await press(page, 0, "ControlOrMeta+a");
  await expect.poll(async () => (await heard(page))?.count).toBe(200);
  // The keys come in their own order: 2 before 10.
  expect((await events(page)).at(-1)).toEqual({
    count: 200,
    keys: Array.from({ length: 200 }, (_, index) => index + 1),
  });
  expect(await headerCheck(page)).toBe("true");
  // `Ctrl`+`A` again changes nothing, and says nothing.
  await press(page, 0, "ControlOrMeta+a");
  await page.waitForTimeout(200);
  expect(await events(page)).toHaveLength(1);

  // A group opened afterwards shows its rows selected.
  await openAlpha(page);
  const rows = (await drawn(page)).filter((row) => row.kind === "row");
  expect(rows.length).toBeGreaterThan(0);
  for (const row of rows) expect(row.selected).toBe("true");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  // One deselected: the header is mixed, and selects them all again.
  await press(page, 2, " ");
  await expect.poll(() => headerCheck(page)).toBe("mixed");
  await shadow(page, (root) => root.querySelector('[part="select-mark"]').click());
  await expect.poll(async () => (await heard(page)).count).toBe(200);
  await expect.poll(() => status(page)).toContain("200 rows selected");
  await expect.poll(() => headerCheck(page)).toBe("true");
  // All selected: the header clears.
  await shadow(page, (root) => root.querySelector('[part="select-mark"]').focus());
  await page.keyboard.press(" ");
  await expect.poll(async () => (await heard(page)).count).toBe(0);
  expect((await heard(page)).keys).toEqual([]);
  await expect.poll(() => status(page)).toContain("0 rows selected");
  expect(await headerCheck(page)).toBe("false");
  expect(await selectedIds(page)).toEqual([]);
});

test("under a filter, select-all is the filter's rows; another filter keeps the selection", async ({
  page,
}) => {
  await open(page);
  const filter = (country) =>
    shadow(
      page,
      (root, country) => {
        const input = root.querySelector('input[data-col="2"]');
        input.focus();
        input.value = country;
        input.dispatchEvent(new Event("input", { bubbles: true }));
      },
      country,
    ).then(() => page.keyboard.press("Enter"));

  await filter("DE");
  await expect.poll(() => status(page)).toContain("52 matches");
  await press(page, 0, "ControlOrMeta+a");
  await expect.poll(async () => (await heard(page))?.count).toBe(52);
  expect(await headerCheck(page)).toBe("true");
  const said = await events(page);

  // Another filter: the 52 stay selected and are not among the rows shown —
  // "all" would be false, and "Selection cleared" too.
  await filter("FR");
  await expect.poll(() => status(page)).toContain("46 matches");
  expect(await status(page)).not.toContain("Selection cleared");
  expect(await events(page)).toEqual(said);
  expect(await headerCheck(page)).toBe("mixed");

  // The header now adds FR's 46 — they were not all selected.
  await shadow(page, (root) => root.querySelector('[part="select-mark"]').click());
  await expect.poll(async () => (await heard(page)).count).toBe(98);
});

test("sorting keeps a selection by key, and does not say it was cleared", async ({ page }) => {
  await open(page, "?menu");
  await openAlpha(page);
  const rows = await drawn(page);
  const picked = [Number(rows[2].text), Number(rows[3].text)].sort((a, b) => a - b);
  await press(page, 2, " ");
  await press(page, 3, " ");
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  const said = await events(page);

  // By amount, descending — the reader's own sort, from the column menu
  // (`set_view` would drop the selection): the same six rows in another order.
  await shadow(page, (root) => root.querySelector('th[data-col="4"]').focus());
  await page.keyboard.press("Alt+ArrowDown");
  await shadow(page, (root) =>
    root.querySelector('[part="column-menu"] [data-action="sort:desc"]').click(),
  );
  await expect
    .poll(() => shadow(page, (root) => root.querySelector('th[data-col="4"]').getAttribute("aria-sort")))
    .toBe("descending");
  const before = rows.slice(2, 8).map((row) => row.text);
  await expect
    .poll(async () => (await drawn(page)).slice(2, 8).map((row) => row.text))
    .not.toEqual(before);
  expect((await drawn(page)).slice(2, 8).map((row) => row.text).sort()).toEqual([...before].sort());
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  expect(await status(page)).not.toContain("Selection cleared");
  expect(await events(page)).toEqual(said);

  // The anchor was Alpha's second row, and another record sits there now: a
  // range from it would take rows nobody pointed at, so `Shift` only toggles.
  const sorted = await drawn(page);
  expect(sorted[3].text).not.toBe(rows[3].text);
  const target = Number(sorted[6].text);
  await press(page, 6, "Shift+ ");
  const expected = picked.includes(target)
    ? picked.filter((id) => id !== target)
    : [...picked, target].sort((a, b) => a - b);
  await expect.poll(async () => (await heard(page)).keys).toEqual(expected);
});

const view = (page) =>
  page.evaluate(() => window.__module.get_view(document.querySelector("opengrid-grid")));

const setView = (page, view, options) =>
  page.evaluate(
    ({ view, options }) =>
      window.__module.set_view(document.querySelector("opengrid-grid"), view, options),
    { view, options },
  );

test("grouping, regrouping and ungrouping keep a selection by key; another row-key drops it", async ({
  page,
}) => {
  await open(page);
  await openAlpha(page);
  const rows = await drawn(page);
  const picked = [Number(rows[2].text), Number(rows[4].text)].sort((a, b) => a - b);
  await press(page, 2, " ");
  await press(page, 4, " ");
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  const said = await events(page);

  // One level instead of two: the records are the same ones.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "customer"),
  );
  await expect.poll(async () => (await drawn(page))[0]?.text).toMatch(/^customer: Alpha/);
  expect(await headerCheck(page)).toBe("mixed");
  await press(page, 0, "Enter");
  await expect.poll(async () => (await drawn(page))[1]?.kind).toBe("row");
  // Alpha of every country now: the two of DE are the selected ones among
  // the rows the window shows.
  expect((await selectedIds(page)).every((id) => picked.includes(id))).toBe(true);

  // Ungrouped, the grid still selects by key: the same two records, at the
  // rows they sit at now.
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("group-by"));
  await expect.poll(() => rowcount(page)).toBe(201);
  await expect.poll(() => status(page)).toBe("200 matches");
  expect(await headerCheck(page)).toBe("mixed");
  expect((await view(page)).selected).toEqual(picked);
  await shadow(
    page,
    (root, position) => {
      const height = root.querySelector("tbody td").getBoundingClientRect().height;
      root.querySelector('[part~="viewport"]').scrollTop = position * height;
    },
    picked[0] - 1,
  );
  await expect.poll(() => selectedIds(page)).toContain(picked[0]);

  // And grouped again. Not one of the three switches was a selection change,
  // and none said the selection was cleared.
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country,customer"),
  );
  await expect.poll(() => rowcount(page)).toBe(7);
  // The focus is still on the cell it was on: moved away, so that focusing
  // the first group again is a focus the grid hears of.
  await shadow(page, (root) => root.querySelector('th[data-col="1"]').focus());
  await openAlpha(page);
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  expect(await events(page)).toEqual(said);
  expect(await status(page)).not.toContain("Selection cleared");

  // Keys of another field name nothing: dropped, said, and the page is told.
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("row-key", "amount"));
  await expect.poll(() => status(page)).toContain("Selection cleared");
  expect((await events(page)).at(-1)).toEqual({ count: 0, keys: [] });
  expect(await events(page)).toHaveLength(said.length + 1);
  expect(await selectedIds(page)).toEqual([]);
});

test("without a row-key, grouping an ungrouped grid drops its selection of positions, and says so", async ({
  page,
}) => {
  await open(page, "?nokey");
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("group-by"));
  await expect.poll(() => rowcount(page)).toBe(201);
  await press(page, 1, " ");
  // By position: the row's number, and the range it makes.
  await expect
    .poll(async () => (await events(page)).at(-1))
    .toEqual({ rows: [1], ranges: [{ from: 1, to: 1 }], count: 1 });
  expect((await view(page)).selected).toBeUndefined();

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country"),
  );
  await expect.poll(() => rowcount(page)).toBe(7);
  await expect.poll(() => status(page)).toContain("Selection cleared");
  expect((await events(page)).at(-1)).toEqual({ rows: [], ranges: [], count: 0 });
  expect(await headerCheck(page)).toBe("false");
});

test("an ungrouped grid with a row-key selects by key: a sort keeps it, Shift takes the rows between", async ({
  page,
}) => {
  await open(page, "?menu");
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("group-by"));
  await expect.poll(() => rowcount(page)).toBe(201);
  await expect.poll(async () => (await drawn(page))[0]?.text).toBe("1");

  // Rows 2 to 5 are the records 3 to 6.
  await press(page, 2, " ");
  await expect.poll(() => events(page)).toEqual([{ count: 1, keys: [3] }]);
  await press(page, 5, "Shift+ ");
  await expect.poll(async () => (await events(page)).at(-1)).toEqual({ count: 4, keys: [3, 4, 5, 6] });
  expect(await selectedIds(page)).toEqual([3, 4, 5, 6]);
  expect(await headerCheck(page)).toBe("mixed");
  const said = await events(page);

  // Sorted the other way round, the four are somewhere else — and selected.
  await shadow(page, (root) => root.querySelector('th[data-col="0"]').focus());
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page))[0]?.text).toBe("200");
  expect(await selectedIds(page)).toEqual([]);
  expect(await status(page)).not.toContain("Selection cleared");
  expect(await events(page)).toEqual(said);
  expect((await view(page)).selected).toEqual([3, 4, 5, 6]);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page))[0]?.text).toBe("1");
  await expect.poll(() => selectedIds(page)).toEqual([3, 4, 5, 6]);

  // The anchor's row holds another record after a sort: `Shift` only toggles.
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page))[0]?.text).toBe("200");
  await press(page, 7, "Shift+ ");
  await expect.poll(async () => (await events(page)).at(-1)).toEqual({ count: 5, keys: [3, 4, 5, 6, 193] });

  // Every row of the filter, and the header clears it again.
  await press(page, 0, "ControlOrMeta+a");
  await expect.poll(async () => (await events(page)).at(-1).count).toBe(200);
  await expect.poll(() => headerCheck(page)).toBe("true");
  await shadow(page, (root) => root.querySelector('[part="select-mark"]').click());
  await expect.poll(async () => (await events(page)).at(-1)).toEqual({ count: 0, keys: [] });
  await expect.poll(() => status(page)).toContain("0 rows selected");
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});

test("the view carries the selection as selected, and selecting is no view change", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    window.__views = [];
    document.addEventListener("opengrid-view-change", (event) => window.__views.push(event.detail.view));
  });
  // Nothing selected: the field is not there.
  expect("selected" in (await view(page))).toBe(false);

  await openAlpha(page);
  const rows = await drawn(page);
  const picked = [Number(rows[2].text), Number(rows[3].text)].sort((a, b) => a - b);
  const before = (await page.evaluate(() => window.__views)).length;
  await press(page, 2, " ");
  await press(page, 3, " ");
  await expect.poll(async () => (await heard(page))?.count).toBe(2);
  expect((await view(page)).selected).toEqual(picked);
  // Two selections, two selection events — and not one view change.
  expect(await page.evaluate(() => window.__views)).toHaveLength(before);

  // Setting the view it has costs nothing and says nothing.
  const saved = await view(page);
  await page.evaluate(() => window.__queries.splice(0));
  const said = await events(page);
  await setView(page, saved);
  await page.waitForTimeout(200);
  expect(await page.evaluate(() => window.__queries)).toEqual([]);
  expect(await events(page)).toEqual(said);
});

test("set_view restores the selection after asking the source, and says what is gone", async ({
  page,
}) => {
  await open(page);
  await openAlpha(page);
  const rows = await drawn(page);
  const picked = [Number(rows[2].text), Number(rows[3].text)].sort((a, b) => a - b);
  await press(page, 2, " ");
  await press(page, 3, " ");
  await expect.poll(async () => (await heard(page))?.count).toBe(2);
  const saved = await view(page);

  // Deselected, and the saved view brings both back: no key is missing, so
  // nothing is said about the selection.
  await press(page, 2, " ");
  await press(page, 3, " ");
  await expect.poll(async () => (await heard(page)).count).toBe(0);
  await page.evaluate(() => window.__queries.splice(0));
  await setView(page, saved);
  await expect.poll(async () => (await events(page)).at(-1)).toEqual({ count: 2, keys: picked });
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  expect(await status(page)).not.toMatch(/Selection cleared|no longer exist/);
  expect((await view(page)).selected).toEqual(picked);
  // The source was asked for exactly these keys, by the key column.
  const asked = await page.evaluate(() =>
    window.__queries.map((query) => JSON.parse(query)).filter((query) => query.filter?.op === "in"),
  );
  expect(asked).toHaveLength(1);
  expect(asked[0]).toMatchObject({ select: ["id"], filter: { field: "id", op: "in", value: picked } });

  // Keys are a page's word. 9999 names no record; "x", 1.5 and true are not
  // even ids — none of the three is asked for, and none is selected.
  await page.evaluate(() => window.__queries.splice(0));
  await setView(
    page,
    { ...saved, selected: [picked[0], 9999, "x", 1.5, true, 7] },
    { notice: "Saved view applied" },
  );
  await expect
    .poll(() => status(page))
    .toBe("200 matches · Saved view applied · 4 selected rows no longer exist");
  expect((await events(page)).at(-1)).toEqual({ count: 2, keys: [7, picked[0]].sort((a, b) => a - b) });
  const checked = await page.evaluate(() =>
    window.__queries.map((query) => JSON.parse(query)).filter((query) => query.filter?.op === "in"),
  );
  expect(checked.map((query) => query.filter.value)).toEqual([[7, picked[0], 9999].sort((a, b) => a - b)]);
  await expect.poll(() => selectedIds(page)).toEqual([picked[0]]);

  // A view without `selected` selects nothing: dropped, and said.
  const { selected, ...bare } = saved;
  expect(selected).toEqual(picked);
  await setView(page, { ...bare, density: "compact" });
  await expect.poll(() => status(page)).toContain("Selection cleared");
  expect((await events(page)).at(-1)).toEqual({ count: 0, keys: [] });
  expect("selected" in (await view(page))).toBe(false);
});

test("a selected that is not a list of keys refuses the view; a failed check is said and selects nothing", async ({
  page,
}) => {
  await open(page);
  const saved = await view(page);
  await setView(page, { ...saved, density: "compact", selected: [[1], { id: 2 }] });
  await expect.poll(() => status(page)).toContain("selected is not a list of keys");
  // Not applied at all: the density is the old one.
  expect((await view(page)).density).toBe("normal");
  expect(await events(page)).toEqual([]);

  // The source fails the check: an error with the result, and no selection.
  await page.evaluate(() => (window.__failChecks = true));
  await setView(page, { ...saved, selected: [1, 2] });
  await expect.poll(() => status(page)).toMatch(/^200 matches · .*check refused/);
  expect(await events(page)).toEqual([]);
  expect("selected" in (await view(page))).toBe(false);
  expect(await headerCheck(page)).toBe("false");
});

test("an ungrouped grid restores a view's selection too", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("group-by"));
  await expect.poll(() => rowcount(page)).toBe(201);
  const saved = await view(page);
  await setView(page, { ...saved, selected: [2, 4, 500] });
  await expect.poll(async () => (await events(page)).at(-1)).toEqual({ count: 2, keys: [2, 4] });
  await expect.poll(() => selectedIds(page)).toEqual([2, 4]);
  expect(await status(page)).toBe("200 matches · 1 selected rows no longer exist");
});

test("under the page's filter: select-all is its rows, a view's key it hides names no record, and changing it drops the selection", async ({
  page,
}) => {
  // Issue #330: `filter` is the page's, and-ed onto every query — the ones
  // that ask for keys among them. It says which records this grid has.
  await open(page);
  const pageFilter = (country) =>
    page.evaluate((country) => {
      window.__queries.length = 0;
      document
        .querySelector("opengrid-grid")
        .setAttribute("filter", JSON.stringify({ field: "country", op: "eq", value: country }));
    }, country);
  await pageFilter("DE");
  await expect.poll(() => status(page)).toBe("52 matches");

  await press(page, 0, "ControlOrMeta+a");
  await expect.poll(async () => (await heard(page))?.count).toBe(52);
  expect(await headerCheck(page)).toBe("true");
  // The keys were asked under the page's filter.
  const all = await page.evaluate(() =>
    window.__queries.map((query) => JSON.parse(query)).filter((query) => query.limit === 52),
  );
  expect(all).toHaveLength(1);
  expect(all[0].filter).toEqual({ field: "country", op: "eq", value: "DE" });

  // 61 is an order from DE, 4 one from FR: the page's filter hides it, so
  // for this grid it is no record.
  const saved = await view(page);
  await page.evaluate(() => (window.__queries.length = 0));
  await setView(page, { ...saved, selected: [61, 4] });
  await expect.poll(() => status(page)).toBe("52 matches · 1 selected rows no longer exist");
  expect((await events(page)).at(-1)).toEqual({ count: 1, keys: [61] });
  const checked = await page.evaluate(() =>
    window.__queries.map((query) => JSON.parse(query)).filter((query) => JSON.stringify(query).includes('"in"')),
  );
  expect(checked.map((query) => query.filter)).toEqual([
    { and: [{ field: "country", op: "eq", value: "DE" }, { field: "id", op: "in", value: [4, 61] }] },
  ]);
  // One selected of 52, and the header knows: the page's filter is not a
  // reader's filter the record could be hidden by.
  expect(await headerCheck(page)).toBe("mixed");

  // Another page filter: other records, so the selection goes — and is said.
  await pageFilter("FR");
  await expect.poll(() => status(page)).toBe("46 matches · Selection cleared");
  expect((await events(page)).at(-1)).toEqual({ count: 0, keys: [] });
  expect("selected" in (await view(page))).toBe(false);
  expect(await headerCheck(page)).toBe("false");
});

test("the print table says nothing of the selection, and preparing it leaves the selection alone", async ({
  page,
}) => {
  // Issue #334: paper has nothing to select. The fixture shows the selection
  // column, and two records are selected.
  await open(page);
  await openAlpha(page);
  const rows = await drawn(page);
  const picked = [Number(rows[2].text), Number(rows[3].text)].sort((a, b) => a - b);
  await press(page, 2, " ");
  await press(page, 3, " ");
  await expect.poll(() => selectedIds(page)).toEqual(picked);
  const said = await events(page);

  await page.evaluate(() => window.__module.prepare_print(document.querySelector("opengrid-grid")));
  const printed = await shadow(page, (root) => {
    const table = root.querySelector("[data-print]").shadowRoot.querySelector("table");
    return {
      head: [...table.querySelectorAll("thead th")].map((th) => th.textContent),
      marked: table.querySelectorAll(
        "[aria-selected], [data-selected], [data-select], [aria-checked], [aria-multiselectable]",
      ).length,
      rows: table.querySelectorAll("tbody tr").length,
    };
  });
  // The shown columns and nothing before them: no selection column.
  expect(printed.head).toEqual(["id", "customer", "country", "flag", "amount"]);
  expect(printed.marked).toBe(0);
  expect(printed.rows).toBeGreaterThan(200);

  // The selection is what it was: on screen, for the page, in the view.
  expect(await selectedIds(page)).toEqual(picked);
  expect(await events(page)).toEqual(said);
  expect((await view(page)).selected).toEqual(picked);
  expect(await headerCheck(page)).toBe("mixed");
});

test("without a row-key a grouped grid selects nothing, as before", async ({ page }) => {
  await open(page, "?nokey");
  await openAlpha(page);
  await press(page, 2, " ");
  await press(page, 3, "Shift+ ");
  await press(page, 2, "ControlOrMeta+a");
  await shadow(page, (root) => root.querySelector('[part="select-mark"]').click());
  await shadow(page, (root) =>
    root.querySelector('tr:has(td[data-row="2"]) > td[data-select="row"]').click(),
  );
  await page.waitForTimeout(200);
  expect(await events(page)).toEqual([]);
  for (const row of await drawn(page)) expect(row.selected).toBeNull();
  expect(
    await shadow(page, (root) => root.querySelector("table").hasAttribute("aria-multiselectable")),
  ).toBe(false);
  expect(await headerCheck(page)).toBe("false");
});
