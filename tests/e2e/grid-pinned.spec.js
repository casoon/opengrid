import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Pinned columns (issue #316): columns kept at the start or the end of the row
// while the others scroll sideways.
//
// What is under test is the contract of docs/api.md §Pinned columns: one
// `<table>` in the order it is read, the pinned cells standing still in both
// directions, the reader's keys and menu with their announcements, the view,
// the pins letting go when they would take more than half of the grid — and
// **WCAG 2.4.11**: a cell that takes the focus is never under a pinned column.

/** Runs `fn(root, arg)` in the shadow root of the grid `selector` names. */
const inside = (page, selector, fn, arg) =>
  page.evaluate(
    ({ selector, fn, arg }) =>
      new Function("root", "arg", `return (${fn})(root, arg)`)(document.querySelector(selector).shadowRoot, arg),
    { selector, fn: fn.toString(), arg },
  );

const status = (page, selector) => inside(page, selector, (root) => root.querySelector('[part="status"]').textContent);

/** The column names in the order the header shows them. */
const headers = (page, selector) =>
  inside(page, selector, (root) =>
    [...root.querySelectorAll("thead th[data-col]")].map((th) => th.querySelector("span").textContent),
  );

const settled = (page, selector) =>
  page.waitForFunction((selector) => {
    const root = document.querySelector(selector)?.shadowRoot;
    const line = root?.querySelector('[part="status"]');
    return !!line && line.getAttribute("data-state") !== "loading" && !!root.querySelector("td[data-row]");
  }, selector);

const focusHeader = (page, selector, name) =>
  inside(
    page,
    selector,
    (root, name) =>
      [...root.querySelectorAll("thead th[data-col]")].find((th) => th.querySelector("span").textContent === name).focus(),
    name,
  );

/** The name of the header that has the focus. */
const focusedHeader = (page, selector) =>
  inside(page, selector, (root) => root.activeElement?.closest("th")?.querySelector("span")?.textContent ?? null);

const scrollTo = async (page, selector, left) => {
  await inside(page, selector, (root, left) => (root.querySelector('[part~="viewport"]').scrollLeft = left), left);
  // The filter row follows in the scroll event's own task.
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
};

/**
 * Where everything stands: the viewport, every header with the cell of the
 * first row under it and its filter field, and what is pinned by the sheet.
 */
const layout = (page, selector) =>
  inside(page, selector, (root) => {
    const box = (element) => {
      const rect = element.getBoundingClientRect();
      return [Math.round(rect.left), Math.round(rect.right)];
    };
    const viewport = root.querySelector('[part~="viewport"]');
    const row = root.querySelector('td[data-row="0"]').closest("tr");
    const edge = viewport.getBoundingClientRect().left + viewport.clientLeft;
    const select = root.querySelector("th[data-select]");
    return {
      viewport: [Math.round(edge), Math.round(edge + viewport.clientWidth)],
      scroll: viewport.scrollLeft,
      select: select && { header: box(select), cell: box(row.querySelector("td[data-select]")) },
      columns: Object.fromEntries(
        [...root.querySelectorAll("th[data-col]")].map((th) => {
          const operator = root.querySelector(`[part="filter-operator"][data-col="${th.dataset.col}"]`);
          return [
            th.querySelector("span").textContent,
            {
              header: box(th),
              cell: box(row.querySelector(`td[data-col="${th.dataset.col}"]`)),
              field: operator && box(operator.parentElement),
              sticky: getComputedStyle(th).position === "sticky",
            },
          ];
        }),
      ),
    };
  });

const near = (a, b) => Math.abs(a[0] - b[0]) <= 1 && Math.abs(a[1] - b[1]) <= 1;

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-pinned.html");
  await page.waitForFunction(() => window.__opengridReady);
  for (const selector of ["#pinned", "#rtl", "#actions", "#grouped"]) await settled(page, selector);
  // The page's presentation is checked against the types of the first result
  // and drawn again; wait for that second answer.
  await page.waitForFunction(() =>
    ["pinned", "rtl", "grouped"].every(
      (id) =>
        document.getElementById(id).shadowRoot.querySelector('th[data-col="1"]') &&
        window.__queries.filter((query) => query.id === id).length >= 2,
    ),
  );
  for (const selector of ["#pinned", "#rtl", "#actions", "#grouped"]) await settled(page, selector);
});

test("the page pins: at the start first, at the end last, in one table", async ({ page }) => {
  // `amount` is declared fourth and pinned at the end: it is the last column,
  // in the header, in the rows and in the query — the DOM is the reading order.
  const order = ["id", "customer", "country", "qty", "ordered_on", "amount"];
  expect(await headers(page, "#pinned")).toEqual(order);
  const dom = await inside(page, "#pinned", (root) => ({
    tables: root.querySelectorAll('table[role="grid"]').length,
    cells: [...root.querySelector('td[data-row="0"]').closest("tr").children].map(
      (cell) => cell.dataset.col ?? "select",
    ),
    colcount: root.querySelector("table").getAttribute("aria-colcount"),
  }));
  expect(dom).toEqual({ tables: 1, cells: ["select", "0", "1", "2", "3", "4", "5"], colcount: "7" });
  const queries = await page.evaluate(() => window.__queries.filter((query) => query.id === "pinned"));
  expect(queries.at(-1).select).toEqual(order);
  expect(await page.evaluate(() => window.__opengridModule.get_query(document.getElementById("pinned")).select)).toEqual(
    order,
  );
  // What the page configured is not the reader's: the view holds no pin.
  expect(
    await page.evaluate(() => window.__opengridModule.get_view(document.getElementById("pinned")).columns.pinned),
  ).toEqual({});
  // And a grid without a pin has nothing in its sheet: it draws as it did.
  await page.evaluate(() => window.__opengridModule.set_columns(document.getElementById("actions"), { id: {} }));
  await expect
    .poll(() => inside(page, "#actions", (root) => root.adoptedStyleSheets[2].cssRules.length))
    .toBe(0);
});

for (const [selector, sign] of [
  ["#pinned", 1],
  ["#rtl", -1],
]) {
  test(`${selector}: the pinned columns stand still while the others scroll`, async ({ page }) => {
    const before = await layout(page, selector);
    const [start, end] = sign === 1 ? [0, 1] : [1, 0];
    // The selection column and `id` at the start edge, `amount` at the end one.
    expect(before.select.header[start]).toBe(before.viewport[start]);
    expect(before.columns.id.header[start]).toBe(before.select.header[end]);
    expect(before.columns.amount.header[end]).toBe(before.viewport[end]);
    expect(before.columns.id.sticky && before.columns.amount.sticky).toBe(true);
    expect(before.columns.customer.sticky).toBe(false);

    for (const offset of [130, 277, 5000]) {
      await scrollTo(page, selector, sign * offset);
      const now = await layout(page, selector);
      expect(Math.abs(now.scroll), `scrolled to ${offset}`).toBeGreaterThan(100);
      for (const part of ["header", "cell"]) {
        expect(now.select[part], `selection ${part} at ${offset}`).toEqual(before.select[part]);
        expect(now.columns.id[part], `id ${part} at ${offset}`).toEqual(before.columns.id[part]);
        expect(now.columns.amount[part], `amount ${part} at ${offset}`).toEqual(before.columns.amount[part]);
      }
      // The others moved by what was scrolled, and every filter field is still
      // above its column — the pinned ones too.
      expect(now.columns.customer.header[0] - before.columns.customer.header[0]).toBe(-now.scroll);
      for (const [name, column] of Object.entries(now.columns)) {
        expect(near(column.header, column.cell), `${name}: cell under its header at ${offset}`).toBe(true);
        expect(near(column.header, column.field), `${name}: field above its column at ${offset}`).toBe(true);
      }
    }

    // A pinned cell and a pinned column's filter field are in sight already:
    // a click on the one and the focus on the other scroll nothing — a browser
    // left alone goes to the place they have in the row.
    await scrollTo(page, selector, sign * 200);
    // A click at the cell's place on the screen, as a reader makes it:
    // `locator.click()` first scrolls the element into view, and WebKit takes
    // the place a `sticky` cell has in its row for that — a scroll no reader
    // causes.
    const box = await page.locator(`${selector} td[data-row="1"][data-col="0"]`).boundingBox();
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    expect(await inside(page, selector, (root) => root.activeElement.dataset.col)).toBe("0");
    await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    expect((await layout(page, selector)).scroll).toBe(sign * 200);
    await inside(page, selector, (root) => root.querySelector('input[data-col="5"]').focus());
    await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    expect((await layout(page, selector)).scroll).toBe(sign * 200);
  });

  test(`${selector}: a cell that takes the focus is never under a pinned column`, async ({ page }) => {
    // WCAG 2.4.11. From every scroll position, into every column that scrolls,
    // from either side and with Home and End: the focused cell ends up whole
    // between the pinned columns.
    const columns = ["id", "customer", "country", "qty", "ordered_on", "amount"];
    const focusCell = (col) =>
      inside(page, selector, (root, col) => root.querySelector(`td[data-row="0"][data-col="${col}"]`).focus(), col);
    const focused = () =>
      inside(page, selector, (root) => {
        const cell = root.activeElement;
        const rect = cell.getBoundingClientRect();
        const edges = (col) => {
          const box = root.querySelector(`th[data-col="${col}"]`).getBoundingClientRect();
          return [box.left, box.right];
        };
        return { col: Number(cell.dataset.col), left: rect.left, right: rect.right, id: edges(0), amount: edges(5) };
      });
    const clear = async (col, how) => {
      const cell = await focused();
      expect(cell.col, how).toBe(col);
      // Between the inner edges of `id` and `amount`, whichever side they are.
      const from = Math.min(cell.id[1], cell.amount[1]);
      const to = Math.max(cell.id[0], cell.amount[0]);
      expect(cell.left, `${columns[col]} ${how}: its left edge`).toBeGreaterThanOrEqual(from - 0.5);
      expect(cell.right, `${columns[col]} ${how}: its right edge`).toBeLessThanOrEqual(to + 0.5);
    };
    const [next, previous] = sign === 1 ? ["ArrowRight", "ArrowLeft"] : ["ArrowLeft", "ArrowRight"];
    // The same for the filter row: a field that takes the focus stands clear
    // of the pinned columns' fields.
    const fieldClear = async (col, offset) => {
        await scrollTo(page, selector, sign * offset);
        await inside(page, selector, (root, col) => root.querySelector(`input[data-col="${col}"]`).focus(), col);
        // WebKit scrolls to a focused field in the next frame, Chromium at once.
        await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
        const field = await inside(
          page,
          selector,
          (root, col) => {
            const edges = (col) => {
              const box = root.querySelector(`[part="filter-operator"][data-col="${col}"]`).parentElement.getBoundingClientRect();
              return [box.left, box.right];
            };
            // The control itself: its group ends in a gap of its own.
            const own = root.querySelector(`input[data-col="${col}"]`).getBoundingClientRect();
            return { own: [own.left, own.right], id: edges(0), amount: edges(5) };
          },
          col,
        );
        const from = Math.min(field.id[1], field.amount[1]);
        const to = Math.max(field.id[0], field.amount[0]);
        expect(field.own[0], `${columns[col]}'s field, scrolled to ${offset}: its left edge`).toBeGreaterThanOrEqual(from - 0.5);
        expect(field.own[1], `${columns[col]}'s field, scrolled to ${offset}: its right edge`).toBeLessThanOrEqual(to + 0.5);
    };

    for (const offset of [0, 97, 260, 5000]) {
      for (const col of [1, 2, 3, 4]) {
        await focusCell(col - 1);
        await scrollTo(page, selector, sign * offset);
        await page.keyboard.press(next);
        await clear(col, `from the cell before, scrolled to ${offset}`);

        await focusCell(col + 1);
        await scrollTo(page, selector, sign * offset);
        await page.keyboard.press(previous);
        await clear(col, `from the cell after, scrolled to ${offset}`);
      }
      for (const col of [1, 2, 3, 4]) await fieldClear(col, offset);
      // A pinned cell is in sight wherever the grid is: focusing it scrolls nothing.
      await focusCell(2);
      await scrollTo(page, selector, sign * offset);
      const scrolled = (await layout(page, selector)).scroll;
      await page.keyboard.press("End");
      expect((await focused()).col).toBe(5);
      expect((await layout(page, selector)).scroll, `End, scrolled to ${offset}`).toBe(scrolled);
      await page.keyboard.press("Home");
      expect((await layout(page, selector)).scroll, `Home, scrolled to ${offset}`).toBe(scrolled);
    }
  });
}

test("the reader pins with the keyboard, one step at a time, and hears it", async ({ page }) => {
  const views = () => page.evaluate(() => window.__views.filter((view) => view.id === "pinned").map((view) => view.pinned));
  const asked = () => page.evaluate(() => window.__queries.filter((query) => query.id === "pinned").length);
  const queries = await asked();

  // Towards the start: `customer` joins `id` there, after it.
  await focusHeader(page, "#pinned", "customer");
  await page.keyboard.press("Alt+Shift+ArrowLeft");
  await expect.poll(() => status(page, "#pinned")).toContain("customer pinned at the start");
  expect(await headers(page, "#pinned")).toEqual(["id", "customer", "country", "qty", "ordered_on", "amount"]);
  expect((await layout(page, "#pinned")).columns.customer.sticky).toBe(true);
  // The focus followed the column, and the page heard one view change.
  await expect.poll(() => focusedHeader(page, "#pinned")).toBe("customer");
  expect(await views()).toEqual([{ customer: "start" }]);
  // `customer` stood second already: no new order, no query.
  expect(await asked()).toBe(queries);

  // Again: nothing changes, and the key says where the column is.
  await page.keyboard.press("Alt+Shift+ArrowLeft");
  await expect.poll(() => status(page, "#pinned")).toContain("customer pinned at the start");
  expect(await views()).toHaveLength(1);

  // One step towards the end lets go, the next pins there: one query each
  // time the column moves.
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => status(page, "#pinned")).toContain("customer no longer pinned");
  expect((await layout(page, "#pinned")).columns.customer.sticky).toBe(false);
  expect(await views()).toEqual([{ customer: "start" }, {}]);

  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => headers(page, "#pinned")).toEqual(["id", "country", "qty", "ordered_on", "customer", "amount"]);
  await expect.poll(() => status(page, "#pinned")).toContain("customer pinned at the end");
  await expect.poll(() => focusedHeader(page, "#pinned")).toBe("customer");
  expect(await views()).toEqual([{ customer: "start" }, {}, { customer: "end" }]);
  expect(await asked()).toBe(queries + 1);
  const pinned = await layout(page, "#pinned");
  expect(pinned.columns.customer.sticky).toBe(true);
  expect(pinned.columns.amount.header[1]).toBe(pinned.viewport[1]);
  expect(pinned.columns.customer.header[1]).toBe(pinned.columns.amount.header[0]);

  // A pin of the page's can be taken away: the view says `none`.
  await focusHeader(page, "#pinned", "id");
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => status(page, "#pinned")).toContain("id no longer pinned");
  expect((await views()).at(-1)).toEqual({ customer: "end", id: "none" });
  const released = await layout(page, "#pinned");
  expect(released.columns.id.sticky).toBe(false);
  // Without a column pinned at the start the selection column scrolls too.
  expect(await inside(page, "#pinned", (root) => getComputedStyle(root.querySelector("th[data-select]")).position)).toBe(
    "static",
  );
});

test("right to left the arrows change places: the left one pins towards the end", async ({ page }) => {
  await focusHeader(page, "#rtl", "customer");
  await page.keyboard.press("Alt+Shift+ArrowLeft");
  await expect.poll(() => status(page, "#rtl")).toContain("customer pinned at the end");
  await expect.poll(() => headers(page, "#rtl")).toEqual(["id", "country", "qty", "ordered_on", "customer", "amount"]);
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => status(page, "#rtl")).toContain("customer no longer pinned");
});

test("the column menu pins too, and shows where the column is", async ({ page }) => {
  const group = () =>
    inside(page, "#pinned", (root) => {
      const items = [...root.querySelectorAll('[part="column-menu"] [data-action^="pin:"]')];
      return {
        label: root.getElementById(items[0]?.closest('[role="group"]').getAttribute("aria-labelledby"))?.textContent,
        items: items.map((item) => [item.dataset.action, item.textContent, item.getAttribute("aria-checked")]),
      };
    });
  await focusHeader(page, "#pinned", "customer");
  await page.keyboard.press("Alt+ArrowDown");
  expect(await group()).toEqual({
    label: "Pin",
    items: [
      ["pin:none", "Not pinned", "true"],
      ["pin:start", "At the start", "false"],
      ["pin:end", "At the end", "false"],
    ],
  });
  // With the menu's own keys: to the entry, and Enter.
  for (let step = 0; step < 20; step += 1) {
    const action = await inside(page, "#pinned", (root) => root.activeElement?.dataset.action);
    if (action === "pin:end") break;
    await page.keyboard.press("ArrowDown");
  }
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page, "#pinned")).toContain("customer pinned at the end");
  await expect.poll(() => headers(page, "#pinned")).toEqual(["id", "country", "qty", "ordered_on", "customer", "amount"]);
  // The focus is back on the header of the column, where it stands now.
  await expect.poll(() => focusedHeader(page, "#pinned")).toBe("customer");

  await page.keyboard.press("Alt+ArrowDown");
  expect((await group()).items.map(([, , checked]) => checked)).toEqual(["false", "false", "true"]);
});

test("pinning keeps the view: sort, filter and search stay", async ({ page }) => {
  // A filter on `country`, a sort by `qty`, then `customer` pinned at the end.
  await inside(page, "#pinned", (root) => {
    const field = root.querySelector('input[data-col="2"]');
    field.focus();
    field.value = "DE";
  });
  await page.keyboard.press("Enter");
  await expect.poll(() => status(page, "#pinned")).toContain("52 matches");
  await focusHeader(page, "#pinned", "qty");
  await page.keyboard.press("Enter");
  await settled(page, "#pinned");

  await focusHeader(page, "#pinned", "customer");
  await page.keyboard.press("Alt+Shift+ArrowRight");
  await expect.poll(() => headers(page, "#pinned")).toEqual(["id", "country", "qty", "ordered_on", "customer", "amount"]);
  await expect.poll(() => status(page, "#pinned")).toContain("customer pinned at the end");
  expect(await status(page, "#pinned")).toContain("52 matches");
  const kept = await inside(page, "#pinned", (root) => ({
    filter: root.querySelector('input[data-col="1"]').value,
    sort: [...root.querySelectorAll("thead th[aria-sort]")]
      .filter((th) => th.getAttribute("aria-sort") !== "none")
      .map((th) => [th.querySelector("span").textContent, th.getAttribute("aria-sort")]),
  }));
  expect(kept).toEqual({ filter: "DE", sort: [["qty", "ascending"]] });
  const view = await page.evaluate(() => window.__opengridModule.get_view(document.getElementById("pinned")));
  expect(view.filters).toEqual([{ column: "country", op: "contains", value: "DE" }]);
  expect(view.columns.pinned).toEqual({ customer: "end" });
});

test("the pins are in the view: set_view restores them, and refuses one that is none", async ({ page }) => {
  const setView = (view) =>
    page.evaluate((view) => window.__opengridModule.set_view(document.getElementById("pinned"), view), view);
  await setView({ columns: { pinned: { id: "none", customer: "start", amount: "none", ordered_on: "end" } } });
  await expect.poll(() => headers(page, "#pinned")).toEqual(["customer", "id", "country", "amount", "qty", "ordered_on"]);
  await settled(page, "#pinned");
  const now = await layout(page, "#pinned");
  expect(Object.entries(now.columns).filter(([, column]) => column.sticky).map(([name]) => name)).toEqual([
    "customer",
    "ordered_on",
  ]);
  expect(
    await page.evaluate(() => window.__opengridModule.get_view(document.getElementById("pinned")).columns.pinned),
  ).toEqual({ amount: "none", customer: "start", id: "none", ordered_on: "end" });

  // A view without pins leaves them to the page again.
  await setView({});
  await expect.poll(() => headers(page, "#pinned")).toEqual(["id", "customer", "country", "qty", "ordered_on", "amount"]);

  await setView({ columns: { pinned: { id: "left" } } });
  await expect.poll(() => status(page, "#pinned")).toContain("id: left is not a pin");
  expect(await headers(page, "#pinned")).toEqual(["id", "customer", "country", "qty", "ordered_on", "amount"]);
});

test("a pin the configuration cannot mean is reported, and nothing of the call applied", async ({ page }) => {
  await page.evaluate(() =>
    window.__opengridModule.set_columns(document.getElementById("pinned"), {
      id: { pin: "left" },
      qty: { pin: "start", width: 300 },
    }),
  );
  await expect.poll(() => status(page, "#pinned")).toContain("id: left is not a pin");
  await settled(page, "#pinned");
  // Refused whole: `qty` is not pinned either, and the pins of the accepted
  // configuration hold.
  expect(await headers(page, "#pinned")).toEqual(["id", "customer", "country", "qty", "ordered_on", "amount"]);
  expect(await status(page, "#pinned")).toContain("id: left is not a pin");
});

test("a column moves among the columns pinned like it, and stops at their edge", async ({ page }) => {
  await focusHeader(page, "#pinned", "id");
  await page.keyboard.press("Control+ArrowRight");
  await expect.poll(() => status(page, "#pinned")).toContain("id is already at the end");
  await focusHeader(page, "#pinned", "customer");
  await page.keyboard.press("Control+ArrowLeft");
  await expect.poll(() => status(page, "#pinned")).toContain("customer is already at the end");
  expect(await headers(page, "#pinned")).toEqual(["id", "customer", "country", "qty", "ordered_on", "amount"]);
});

test("too narrow, the pins let go and say so once; with room they hold again", async ({ page }) => {
  // Everything the status line says from here on, each text once per change.
  await inside(page, "#pinned", (root) => {
    const line = root.querySelector('[part="status"]');
    window.__said = [];
    new MutationObserver(() => window.__said.push(line.textContent)).observe(line, {
      childList: true,
      characterData: true,
      subtree: true,
    });
  });
  const sentence = "Too narrow for the pinned columns: they scroll with the others";
  const said = () =>
    page.evaluate(
      (sentence) =>
        // How often the sentence came into the line: a text that has it after one that had not.
        window.__said.filter((text, index, all) => text.includes(sentence) && !(all[index - 1] ?? "").includes(sentence))
          .length,
      sentence,
    );
  const width = (px) => page.evaluate((px) => (document.getElementById("pinned").style.width = `${px}px`), px);
  const sticky = async () => {
    const { columns } = await layout(page, "#pinned");
    return [columns.id.sticky, columns.amount.sticky];
  };
  const padding = () =>
    inside(page, "#pinned", (root) => getComputedStyle(root.querySelector('[part~="viewport"]')).scrollPaddingInlineStart);

  expect(await sticky()).toEqual([true, true]);
  expect(await padding()).toBe("174px");

  // 44 + 130 + 130 px pinned are more than half of 560.
  await expect.poll(sticky).toEqual([true, true]);
  await width(560);
  await expect.poll(sticky).toEqual([false, false]);
  await expect.poll(() => status(page, "#pinned")).toContain(sentence);
  expect(await padding()).toBe("auto");
  // Narrower still: nothing new to say.
  await width(520);
  await width(400);
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  await expect.poll(sticky).toEqual([false, false]);
  expect(await said()).toBe(1);
  // Not after other results have taken its place in the line, either: it is
  // said when the pins let go, not whenever a narrow grid is laid out.
  await focusHeader(page, "#pinned", "qty");
  for (const sort of ["ascending", "descending"]) {
    await page.keyboard.press("Enter");
    await expect
      .poll(() => inside(page, "#pinned", (root) => root.activeElement?.closest("th")?.getAttribute("aria-sort")))
      .toBe(sort);
    await settled(page, "#pinned");
  }
  await expect.poll(() => status(page, "#pinned")).not.toContain(sentence);
  await width(380);
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  expect(await status(page, "#pinned")).not.toContain(sentence);
  // The setting stayed: the view is what it was, and with room the pins hold.
  expect(await headers(page, "#pinned")).toEqual(["id", "customer", "country", "qty", "ordered_on", "amount"]);
  await width(920);
  await expect.poll(sticky).toEqual([true, true]);
  expect(await padding()).toBe("174px");
});

test("the action column is pinned along at the end", async ({ page }) => {
  const before = await layout(page, "#actions");
  expect(before.columns.Open.sticky && before.columns.ordered_on.sticky).toBe(true);
  expect(before.columns.Open.header[1]).toBe(before.viewport[1]);
  expect(before.columns.ordered_on.header[1]).toBe(before.columns.Open.header[0]);
  expect(before.columns.id.sticky).toBe(false);
  await scrollTo(page, "#actions", 120);
  const now = await layout(page, "#actions");
  expect(now.scroll).toBe(120);
  expect(now.columns.Open.cell).toEqual(before.columns.Open.cell);
  expect(now.columns.ordered_on.cell).toEqual(before.columns.ordered_on.cell);
  expect(now.columns.id.cell[0]).toBe(before.columns.id.cell[0] - 120);
});

test("a group's label runs across the row, and keeps to its pinned column while scrolled", async ({ page }) => {
  const label = () =>
    inside(page, "#grouped", (root) => {
      const cell = root.querySelector('tr[data-kind="group"] td[data-col="0"]');
      const rect = cell.getBoundingClientRect();
      const viewport = root.querySelector('[part~="viewport"]').getBoundingClientRect();
      return {
        text: cell.textContent,
        left: Math.round(rect.left - viewport.left),
        overflow: getComputedStyle(cell).overflowX,
        cut: cell.scrollWidth > cell.clientWidth,
        ground: getComputedStyle(cell).backgroundColor,
      };
    });
  const before = await label();
  // Unscrolled it is drawn as without a pin: over the cells beside it.
  expect(before).toMatchObject({ left: 0, overflow: "visible", cut: true });
  expect(before.text).toContain("country:");

  await scrollTo(page, "#grouped", 180);
  const scrolled = await label();
  // It stays where it was, inside its column, on the row's own ground.
  expect(scrolled).toMatchObject({ left: 0, overflow: "hidden", text: before.text });
  expect(scrolled.ground).not.toBe("rgba(0, 0, 0, 0)");
  // The whole label is there again when its cell has the focus.
  await inside(page, "#grouped", (root) => root.querySelector('tr[data-kind="group"] td[data-col="0"]').focus());
  expect(
    await inside(page, "#grouped", (root) => getComputedStyle(root.activeElement).whiteSpace),
  ).toBe("normal");
});

test("on paper nothing is pinned", async ({ page }) => {
  await scrollTo(page, "#pinned", 200);
  const rows = await page.evaluate(() => window.__opengridModule.prepare_print(document.getElementById("pinned")));
  expect(rows).toBe(200);
  await page.emulateMedia({ media: "print" });
  const paper = await inside(page, "#pinned", (root) => {
    const table = root.querySelector("[data-print]").shadowRoot.querySelector("table");
    return {
      // The grid's own cells, should the page be printed unprepared.
      grid: getComputedStyle(root.querySelector('th[data-col="0"]')).position,
      header: [...table.querySelectorAll("thead th")].map((th) => th.textContent),
      positions: [...new Set([...table.querySelectorAll("th, td")].map((cell) => getComputedStyle(cell).position))],
    };
  });
  expect(paper).toEqual({
    grid: "static",
    header: ["id", "customer", "country", "qty", "ordered_on", "amount"],
    positions: ["static"],
  });
});

test("the page has no axe violations, scrolled and with a pin changed", async ({ page }) => {
  await scrollTo(page, "#pinned", 200);
  await scrollTo(page, "#rtl", -200);
  await focusHeader(page, "#pinned", "customer");
  await page.keyboard.press("Alt+Shift+ArrowLeft");
  await expect.poll(() => status(page, "#pinned")).toContain("customer pinned at the start");
  await page.keyboard.press("Alt+ArrowDown");
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});
