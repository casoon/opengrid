import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// `<opengrid-grid>` virtualization (plan point 17).
//
// 200 rows come from the real engine, but only `window-size="8"` DOM rows exist:
// scrolling moves the window and recycles the rows. This spec proves the DoD:
// the DOM data-row count stays constant across a long scroll, `aria-rowindex`
// tracks the logical position, a focused cell survives a scroll that would
// recycle its slot, and `Ctrl+End`/`Ctrl+Home` reach the last/first row.

/** Facts about the rendered shadow root. */
async function facts(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const table = root.querySelector("table");
    const viewport = root.querySelector('[part="viewport"]');
    return {
      rowcount: table.getAttribute("aria-rowcount"),
      colcount: table.getAttribute("aria-colcount"),
      // `td[data-row]` counts cells (pool × columns); the row slots are the
      // `<tr>`s that carry an `aria-rowindex`.
      dataRowCells: root.querySelectorAll("td[data-row]").length,
      rows: root.querySelectorAll("tbody tr[aria-rowindex]").length,
      distinctRows: new Set(
        [...root.querySelectorAll("td[data-row]")].map((td) =>
          td.getAttribute("data-row"),
        ),
      ).size,
      scrollHeight: viewport.scrollHeight,
      clientHeight: viewport.clientHeight,
    };
  });
}

/** The `aria-rowindex` of the `<tr>` that renders logical row `row`. */
async function rowindexFor(page, row) {
  return page.evaluate((row) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const cell = root.querySelector(`td[data-row="${row}"]`);
    return cell?.closest("tr")?.getAttribute("aria-rowindex") ?? null;
  }, row);
}

/** The inner focused element, or null. */
async function activeCell(page) {
  return page.evaluate(() => {
    const element = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!element) return null;
    return {
      tag: element.tagName.toLowerCase(),
      row: element.getAttribute("data-row"),
      col: element.getAttribute("data-col"),
    };
  });
}

/** Sets the viewport scroll offset. */
async function scrollTo(page, top) {
  await page.evaluate((top) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('[part="viewport"]').scrollTop = top;
  }, top);
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-virtual.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("DOM data-row count stays constant across a long scroll", async ({
  page,
}) => {
  const before = await facts(page);
  expect(before.rows).toBe(8);
  expect(before.dataRowCells).toBe(32);
  expect(before.distinctRows).toBe(8);
  expect(before.rowcount).toBe("201");
  expect(before.scrollHeight).toBeGreaterThan(before.clientHeight);

  await scrollTo(page, 4000);
  await expect.poll(() => rowindexFor(page, 125)).toBe("127");

  const after = await facts(page);
  expect(after.rows).toBe(8);
  expect(after.dataRowCells).toBe(32);
  expect(after.distinctRows).toBe(8);

  // And back to the top: still exactly the pool.
  await scrollTo(page, 0);
  await expect.poll(() => rowindexFor(page, 0)).toBe("2");
  expect((await facts(page)).rows).toBe(8);
});

test("aria-rowindex stays correct after scrolling into the middle", async ({
  page,
}) => {
  await scrollTo(page, 4000);
  // Row 125 is visible near the middle of the window: 1-based including header.
  await expect.poll(() => rowindexFor(page, 125)).toBe("127");
  await expect.poll(() => rowindexFor(page, 119)).toBe("121");

  // The last logical row still carries its documented index.
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot
      .querySelector('th[data-col="0"]')
      .focus();
  });
  await page.keyboard.press("Control+End");
  await expect.poll(() => rowindexFor(page, 199)).toBe("201");
});

test("a focused data cell survives a scroll that recycles its slot", async ({
  page,
}) => {
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('td[data-row="3"][data-col="0"]').focus();
  });
  expect(await activeCell(page)).toMatchObject({ row: "3", col: "0" });

  // Scroll far enough that the window leaves row 3; without pinning its slot
  // would be recycled and the focused node rewritten.
  await scrollTo(page, 4000);
  await expect.poll(() => rowindexFor(page, 125)).toBe("127");

  // The focused node still renders row 3 and still holds the focus.
  expect(await rowindexFor(page, 3)).toBe("5");
  expect(await activeCell(page)).toMatchObject({ row: "3", col: "0" });
  // The pool is still full: the pinned row does not add a node.
  expect((await facts(page)).distinctRows).toBe(8);
});

test("Ctrl+End focuses the last logical row, Ctrl+Home the first", async ({
  page,
}) => {
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot
      .querySelector('th[data-col="0"]')
      .focus();
  });

  await page.keyboard.press("Control+End");
  await expect.poll(() => activeCell(page)).toMatchObject({ row: "199", col: "3" });
  await expect.poll(() => rowindexFor(page, 199)).toBe("201");

  await page.keyboard.press("Control+Home");
  await expect.poll(() => activeCell(page)).toMatchObject({ tag: "th", col: "0" });
  await expect.poll(() => rowindexFor(page, 0)).toBe("2");
});

test("PageDown moves the window by a viewport", async ({ page }) => {
  await page.evaluate(() => {
    document.querySelector("opengrid-grid").shadowRoot
      .querySelector('th[data-col="0"]')
      .focus();
  });

  await page.keyboard.press("PageDown");
  // The 160px viewport shows five 32px rows, so the first page lands on row 4.
  await expect.poll(() => activeCell(page)).toMatchObject({ row: "4" });
  await page.keyboard.press("PageDown");
  await expect.poll(() => activeCell(page)).toMatchObject({ row: "9" });
  await expect.poll(() => rowindexFor(page, 9)).toBe("11");
});

test("has no axe violations", async ({ page }) => {
  await scrollTo(page, 4000);
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});

/**
 * What the grid writes into the shadow root while `act` runs: Text nodes
 * replaced or changed inside the data cells (`childList`, `characterData`) and
 * attributes written to the body's rows and cells, by `TAG.name` (a write of
 * the same value counts too).
 */
async function writesDuring(page, act) {
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const seen = { text: 0, attributes: [] };
    const tally = (record) => {
      if (record.type === "attributes") {
        if (record.target.closest("tbody")) {
          const name = `${record.target.tagName}.${record.attributeName}`;
          if (!seen.attributes.includes(name)) seen.attributes.push(name);
        }
        return;
      }
      const target = record.target.nodeType === 1 ? record.target : record.target.parentElement;
      if (target?.closest("td[data-row]")) seen.text += 1;
    };
    const observer = new MutationObserver((records) => records.forEach(tally));
    window.__tally = tally;
    observer.observe(root, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
    });
    window.__writes = { seen, observer };
  });
  await act();
  return page.evaluate(async () => {
    await new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)));
    const { seen, observer } = window.__writes;
    observer.takeRecords().forEach(window.__tally);
    observer.disconnect();
    return seen;
  });
}

test("a frame that changes no row writes only what changed", async ({ page }) => {
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    root.querySelector('td[data-row="3"][data-col="0"]').focus();
  });

  // Selecting a row redraws the frame, but every cell already shows the right
  // text. Replacing a Text node for the same characters costs a layout.
  const same = await writesDuring(page, async () => {
    await page.keyboard.press(" ");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            document
              .querySelector("opengrid-grid")
              .shadowRoot.querySelectorAll('tr[aria-selected="true"]').length,
        ),
      )
      .toBe(1);
  });
  expect(same.text).toBe(0);
  // And of the attributes only the two the selection changes: the rest of the
  // set a row is patched with is already as it should be. (The presentation
  // markers are the element's, not the frame's.)
  expect(
    same.attributes
      .filter((name) => !/data-(align|mono|emphasis|muted)|aria-readonly/.test(name))
      .sort(),
  ).toEqual(["TR.aria-selected", "TR.data-selected"]);

  // Not vacuous: a scroll recycles rows, and their text does change.
  const moved = await writesDuring(page, async () => {
    await scrollTo(page, 4000);
    await expect.poll(() => rowindexFor(page, 125)).toBe("127");
  });
  expect(moved.text).toBeGreaterThan(0);
});
