import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Grouping (plan point 62).
//
// The claim under test is the one decided as F1: grouping and virtualization
// do **not** exclude each other, because the display list is arithmetic over
// the group counts — one truth about `aria-rowcount`, computed rather than
// counted. And F2: while grouped, the grid is a `treegrid`.
//
// The fixture groups the 200 conformance rows by `country`: DE 52, FR 46,
// GB 49, US 47 and six rows without a country.

const root = (page) =>
  page.evaluateHandle(() => document.querySelector("opengrid-grid").shadowRoot);

/** The rows currently drawn, in display order. */
async function drawn(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.style.display)
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .map((tr) => ({
        kind: tr.dataset.kind ?? "row",
        level: tr.getAttribute("aria-level"),
        expanded: tr.getAttribute("aria-expanded"),
        index: Number(tr.getAttribute("aria-rowindex")),
        text: tr.querySelector("td[data-col]")?.textContent ?? "",
      }));
  });
}

async function rowcount(page) {
  return page.evaluate(() =>
    Number(
      document
        .querySelector("opengrid-grid")
        .shadowRoot.querySelector("table")
        .getAttribute("aria-rowcount"),
    ),
  );
}

async function status(page) {
  return page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[part="status"]')
      .textContent.trim(),
  );
}

/** Focuses a cell and presses a key on it, the way the keyboard would. */
async function press(page, selector, key) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
  await page.keyboard.press(key);
}

async function settled(page) {
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    const line = root?.querySelector('[part="status"]');
    return !!line && line.getAttribute("data-state") !== "loading" && !!root.querySelector("td[data-row]");
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-group.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await settled(page);
  await expect.poll(() => rowcount(page)).toBe(7);
});

test("a grouped grid is a treegrid of group headers", async ({ page }) => {
  expect(
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector("table").getAttribute("role"),
    ),
  ).toBe("treegrid");

  const rows = await drawn(page);
  expect(rows.map((row) => row.text)).toEqual([
    "country: DE (52 rows)",
    "country: FR (46 rows)",
    "country: GB (49 rows)",
    "country: US (47 rows)",
    // NULL last (S3), and named: an empty header is silence to a reader.
    "country: (no value) (6 rows)",
    // The grand total of point 63, the last position of the list.
    "Total (200 rows)",
  ]);
  for (const row of rows.slice(0, -1)) {
    expect(row).toMatchObject({ kind: "group", level: "1", expanded: "false" });
  }
  expect(rows.at(-1).kind).toBe("total");
});

test("the status line counts rows, not display positions", async ({ page }) => {
  // Five headers and 200 rows: "205 matches" would be false.
  expect(await status(page)).toBe("200 matches");
});

test("aria-rowcount is the display list, before and after opening", async ({ page }) => {
  // Five headers plus the header row.
  expect(await rowcount(page)).toBe(7);

  await press(page, 'td[data-row="1"][data-col="0"]', "Enter");
  // FR opens: 46 more positions (five headers, the total, the header row).
  await expect.poll(() => rowcount(page)).toBe(7 + 46);

  const rows = await drawn(page);
  expect(rows[1]).toMatchObject({ kind: "group", expanded: "true" });
  // The first row under FR is a data row one level deeper, and it is from FR.
  expect(rows[2]).toMatchObject({ kind: "row", level: "2", index: 4 });

  await press(page, 'td[data-row="1"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(7);
});

test("the focus stays on the group that was toggled", async ({ page }) => {
  await press(page, 'td[data-row="2"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 49);

  const focused = await page.evaluate(() => {
    const cell = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    return { row: cell?.getAttribute("data-row"), text: cell?.textContent };
  });
  expect(focused).toEqual({ row: "2", text: "country: GB (49 rows)" });
});

test("right opens and left closes, on the group's first cell", async ({ page }) => {
  // The treegrid keys (F2). Right on an open group and left on a closed one
  // fall through to moving, so neither key is a dead end.
  await press(page, 'td[data-row="0"][data-col="0"]', "ArrowRight");
  await expect.poll(() => rowcount(page)).toBe(7 + 52);

  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => rowcount(page)).toBe(7);

  // Closed now: another left moves nowhere and opens nothing.
  await page.keyboard.press("ArrowLeft");
  await page.waitForTimeout(150);
  expect(await rowcount(page)).toBe(7);
});

test("NULL and the empty string are two groups with two names", async ({ page }) => {
  // S10 and S14, and S1 underneath: the NULL group's rows are fetched with
  // `is_null`, because `eq null` is unknown and would match nothing.
  await page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    host.setAttribute("columns", "id,note");
    host.setAttribute("group-by", "note");
  });
  await expect.poll(() => status(page)).toBe("200 matches");

  const names = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('tr[data-kind="group"]')].map(
      (tr) => tr.querySelector("td[data-col]").textContent,
    ),
  );
  // The empty-string group sorts first (binary, S4); drawn in the window.
  expect(names[0]).toMatch(/^note: \(empty\) \(\d+ rows\)$/);

  // And opening it shows rows whose note really is empty — not the NULL rows.
  await press(page, 'td[data-row="0"][data-col="0"]', "Enter");
  await expect.poll(async () => (await drawn(page))[1]?.kind).toBe("row");
  const notes = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll('tr:not([data-kind]) td[data-col="1"]')]
      .filter((td) => td.closest("tr").style.transform)
      .map((td) => td.textContent),
  );
  expect(notes.length).toBeGreaterThan(0);
  for (const note of notes) expect(note).toBe("");
});

test("an open NULL group shows its rows, and they are its rows", async ({ page }) => {
  // `eq null` would be unknown and match nothing (S1): an open NULL group would
  // count six rows and show none. The window holds eight positions, so after
  // five headers three of the six are drawn — every one of them without a
  // country.
  await press(page, 'td[data-row="4"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 6);
  await expect.poll(async () => (await drawn(page)).filter((row) => row.kind === "row").length).toBeGreaterThan(0);

  const countries = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.dataset.kind)
      .map((tr) => tr.querySelector('td[data-col="2"]').textContent),
  );
  expect(countries.length).toBe(3);
  for (const country of countries) expect(country).toBe("");
});

test("a window asks one query per group it touches", async ({ page }) => {
  // The "1 + K queries" of the prototype: not one per row.
  await press(page, 'td[data-row="0"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(7 + 52);
  await press(page, 'td[data-row="1"][data-col="0"]', "ArrowDown");

  await page.evaluate(() => {
    window.__queries.length = 0;
  });
  // Scroll to where the end of DE and the start of the next groups meet.
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const row = parseFloat(getComputedStyle(root.querySelector("tbody td")).height);
    root.querySelector('[part="viewport"]').scrollTop = 50 * row;
  });
  await expect.poll(() => page.evaluate(() => window.__queries.length)).toBeGreaterThan(0);
  await page.waitForTimeout(200);

  const queries = await page.evaluate(() => window.__queries.map((q) => JSON.parse(q)));
  // Only DE is open, so however the window falls, its rows come from one
  // query — and no group query is repeated for a scroll.
  expect(queries.filter((q) => q.group)).toEqual([]);
  expect(queries.filter((q) => !q.group).length).toBe(1);
});

test("scrolling a grouped grid adds no DOM rows", async ({ page }) => {
  // Point 17's promise, kept under grouping: headers and rows share the pool.
  for (const row of [0, 53, 100]) {
    await page.evaluate((row) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      const cell = root.querySelector('td[data-row="0"][data-col="0"]');
      if (cell && cell.closest("tr").getAttribute("aria-expanded") === "false") cell.click();
    }, row);
  }
  await expect.poll(() => rowcount(page)).toBe(7 + 52);
  const before = await page.evaluate(
    () => document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr").length,
  );
  for (const top of [400, 1200, 2000, 0]) {
    await page.evaluate((top) => {
      document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]').scrollTop = top;
    }, top);
    await page.waitForTimeout(80);
    expect(
      await page.evaluate(
        () => document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr").length,
      ),
    ).toBe(before);
  }
});

test("two levels: a group opens onto its subgroups, and those onto rows", async ({ page }) => {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country,customer"),
  );
  await expect.poll(() => rowcount(page)).toBe(7);

  await press(page, 'td[data-row="0"][data-col="0"]', "Enter");
  // DE's customers arrive as level-2 headers.
  await expect.poll(async () => (await drawn(page))[1]?.level).toBe("2");
  const second = (await drawn(page))[1];
  expect(second).toMatchObject({ kind: "group", expanded: "false" });
  expect(second.text).toMatch(/^customer: /);

  await press(page, 'td[data-row="1"][data-col="0"]', "Enter");
  await expect.poll(async () => (await drawn(page))[2]?.kind).toBe("row");
  expect((await drawn(page))[2].level).toBe("3");
});

test("paging and grouping together are refused and named", async ({ page }) => {
  await page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    host.removeAttribute("group-by");
    host.setAttribute("page-size", "10");
    host.setAttribute("group-by", "country");
  });
  await expect.poll(() => status(page)).toMatch(/Cannot group by country.*page-size/);
  expect(
    await page.evaluate(() =>
      document.querySelector("opengrid-grid").shadowRoot.querySelector("table").getAttribute("role"),
    ),
  ).toBe("grid");
});

test("a group-by of a column the grid does not show is refused", async ({ page }) => {
  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "ordered_on"),
  );
  await expect.poll(() => status(page)).toContain("Cannot group by ordered_on");
});

test("a grouping and its open groups travel in the view", async ({ page }) => {
  await press(page, 'td[data-row="4"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(13);

  const view = await page.evaluate(() =>
    window.__opengridModule.get_view(document.querySelector("opengrid-grid")),
  );
  expect(view.group).toEqual(["country"]);
  // The NULL group is open — and NULL is a key like any other.
  expect(view.expanded).toEqual([[null]]);

  // Close it, then restore the view: it is open again.
  await press(page, 'td[data-row="4"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(7);
  await page.evaluate(
    (view) => window.__opengridModule.set_view(document.querySelector("opengrid-grid"), view),
    view,
  );
  await expect.poll(() => rowcount(page)).toBe(13);
});

test("opening and closing is said once each, and survives the result", async ({ page }) => {
  // Recorded as a sequence (phase E (o)): said twice, never said, or said and
  // then overwritten by the next result are the three failures that matter.
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const line = root.querySelector('[part="status"]');
    window.__said = [];
    new MutationObserver(() => {
      const text = line.textContent.trim();
      if (window.__said.at(-1) !== text) window.__said.push(text);
    }).observe(line, { childList: true, characterData: true, subtree: true });
  });

  await press(page, 'td[data-row="0"][data-col="0"]', "Enter");
  await expect.poll(() => rowcount(page)).toBe(59);
  await page.waitForTimeout(200);
  await page.keyboard.press("Enter");
  await expect.poll(() => rowcount(page)).toBe(7);
  await page.waitForTimeout(200);

  const said = await page.evaluate(() => window.__said);
  expect(said.filter((text) => text.includes("DE expanded, 52 rows"))).toHaveLength(1);
  expect(said.filter((text) => text.includes("DE collapsed"))).toHaveLength(1);
  // No "Loading" in between: a toggle re-asks the window, not the result.
  expect(said.filter((text) => /loading/i.test(text))).toEqual([]);
  // The last word is the collapse, riding with the count — not overwritten.
  expect(said.at(-1)).toBe("200 matches · DE collapsed");
});

test("has no axe violations, closed and open, on two levels", async ({ page }) => {
  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

  await page.evaluate(() =>
    document.querySelector("opengrid-grid").setAttribute("group-by", "country,customer"),
  );
  await expect.poll(() => rowcount(page)).toBe(7);
  await press(page, 'td[data-row="0"][data-col="0"]', "Enter");
  await expect.poll(async () => (await drawn(page))[1]?.level).toBe("2");
  await press(page, 'td[data-row="1"][data-col="0"]', "Enter");
  await expect.poll(async () => (await drawn(page))[2]?.kind).toBe("row");

  expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
});
