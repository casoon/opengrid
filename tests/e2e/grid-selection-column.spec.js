import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The selection column (plan point 61).
//
// The selection itself is point 35 and unchanged: `Space` on any data cell
// selects, `Ctrl`+`A` selects every matching row, sorting and filtering drop it.
// What this point adds is a column that *shows* it and a mouse path to it — and
// the three things that can go wrong when a column is added to a grid: the
// counting, the promise of "all", and the row the focus is standing on.

const press = (page, key, options = {}) =>
  page.evaluate(
    ({ key, options }) => {
      const root = document.querySelector("opengrid-grid").shadowRoot;
      root.activeElement.dispatchEvent(
        new KeyboardEvent("keydown", { key, bubbles: true, composed: true, ...options }),
      );
    },
    { key, options },
  );

async function focusIn(page, selector) {
  await page.evaluate((selector) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector(selector).focus();
  }, selector);
}

async function activeCell(page) {
  return page.evaluate(() => {
    const cell = document.querySelector("opengrid-grid").shadowRoot.activeElement;
    if (!cell) return null;
    return {
      tag: cell.tagName.toLowerCase(),
      // The marker sits on the cell; the focus may be on the widget inside it.
      select: cell.closest("[data-select]")?.getAttribute("data-select") ?? null,
      role: cell.getAttribute("role"),
      col: cell.getAttribute("data-col"),
      row: cell.getAttribute("data-row"),
    };
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-selection-column.html");
  await page.waitForFunction(() => window.__opengridReady);
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-grid")?.shadowRoot;
    return !!root?.querySelector("td[data-row]");
  });
});

test("the selection column counts as a column", async ({ page }) => {
  // A reader told there are four columns would look for the fifth in vain.
  const counted = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      colcount: root.querySelector("table").getAttribute("aria-colcount"),
      headerCells: root.querySelectorAll("thead th").length,
      firstRowCells: root.querySelector("tbody tr").querySelectorAll("td").length,
    };
  });
  // Four data columns in the fixture, plus the selection column.
  expect(counted.colcount).toBe("5");
  expect(counted.headerCells).toBe(5);
  expect(counted.firstRowCells).toBe(5);
});

test("data-col keeps meaning the schema column", async ({ page }) => {
  // The selection column is addressed by `data-select`. Renumbering `data-col`
  // instead would have moved every format, filter and presentation by one.
  const first = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const row = root.querySelector("tbody tr");
    const cells = [...row.querySelectorAll("td")];
    return {
      firstIsSelect: cells[0].getAttribute("data-select"),
      firstHasCol: cells[0].hasAttribute("data-col"),
      secondCol: cells[1].getAttribute("data-col"),
    };
  });
  expect(first).toEqual({ firstIsSelect: "row", firstHasCol: false, secondCol: "0" });
});

test("the arrows reach the selection column and Home starts there", async ({ page }) => {
  await focusIn(page, 'td[data-row="0"][data-col="0"]');
  await press(page, "ArrowLeft");
  expect(await activeCell(page)).toMatchObject({ tag: "td", select: "row" });

  // And back out again.
  await press(page, "ArrowRight");
  expect(await activeCell(page)).toMatchObject({ tag: "td", col: "0" });

  // `Home` is the start of the row, which is now the selection cell.
  await press(page, "Home");
  expect(await activeCell(page)).toMatchObject({ tag: "td", select: "row" });

  // Up from there is the column's own header, not the first data header.
  // The focusable thing there is the checkbox inside the header cell: a
  // `columnheader` may not carry `aria-checked`, and the grid pattern lets the
  // single widget in a cell be the focusable element.
  await press(page, "ArrowUp");
  expect(await activeCell(page)).toMatchObject({
    tag: "span",
    role: "checkbox",
    select: "all",
  });
});

test("Space on the selection cell selects the row", async ({ page }) => {
  await focusIn(page, 'td[data-row="1"][data-col="0"]');
  await press(page, "Home");
  await press(page, " ");

  const state = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const row = root.querySelector('td[data-row="1"]').closest("tr");
    return {
      selected: row.getAttribute("aria-selected"),
      mark: row.querySelector('[part="select-mark"]').textContent,
    };
  });
  expect(state.selected).toBe("true");
  expect(state.mark).not.toBe("");
});

test("the header selects every matching row, not the loaded page", async ({ page }) => {
  // The fixture holds 200 rows in a window of 8. A header that said "all" about
  // what is on screen would be a smaller promise than `Ctrl`+`A` already makes.
  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[data-select="all"]')
      .click(),
  );

  const after = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    return {
      checked: root
        .querySelector('[data-select="all"] > [part="select-mark"]')
        .getAttribute("aria-checked"),
      loaded: root.querySelectorAll("tbody tr").length,
      selectedRows: root.querySelectorAll('tr[aria-selected="true"]').length,
    };
  });
  expect(after.checked).toBe("true");
  // Every *loaded* row shows as selected …
  expect(after.selectedRows).toBe(after.loaded);

  // … and the event says how many rows that really is. Cleared first, so the
  // next click is a select rather than a deselect.
  const count = await page.evaluate(
    () =>
      new Promise((resolve) => {
        const host = document.querySelector("opengrid-grid");
        const box = host.shadowRoot.querySelector('[data-select="all"]');
        box.click(); // clears — everything is selected at this point
        host.addEventListener("opengrid-selection-change", (e) => resolve(e.detail.count), {
          once: true,
        });
        box.click();
      }),
  );
  expect(count).toBe(200);
});

test("the header shows a mixed state for a partial selection", async ({ page }) => {
  await focusIn(page, 'td[data-row="0"][data-col="0"]');
  await press(page, " ");

  await expect
    .poll(() =>
      page.evaluate(() =>
        document
          .querySelector("opengrid-grid")
          .shadowRoot.querySelector('[data-select="all"]')
          .querySelector('[part="select-mark"]')
          .getAttribute("aria-checked"),
      ),
    )
    .toBe("mixed");
});

test("the row the focus is standing on shows its own selection", async ({ page }) => {
  // The renderer pins the focused row's slot and skips rewriting it (point 17)
  // — and that row is exactly the one being selected. Three features have
  // already been caught by this; the mark is the fourth.
  await focusIn(page, 'td[data-row="2"][data-col="1"]');
  await press(page, " ");

  const pinned = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const row = root.querySelector('td[data-row="2"]').closest("tr");
    return {
      focusedHere: root.activeElement?.getAttribute("data-row"),
      selected: row.getAttribute("aria-selected"),
      mark: row.querySelector('[part="select-mark"]').textContent,
    };
  });
  expect(pinned.focusedHere).toBe("2");
  expect(pinned.selected).toBe("true");
  expect(pinned.mark, "the pinned row's mark has to be written too").not.toBe("");
});

test("the selection column's header is named in words", async ({ page }) => {
  // "✓" read aloud is not a promise anybody can act on.
  const name = await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const box = root.querySelector('[data-select="all"] > [part="select-mark"]');
    return {
      label: box.getAttribute("aria-label"),
      role: box.getAttribute("role"),
      cellRole: box.closest("th").hasAttribute("aria-checked"),
      rowMarkHidden: root
        .querySelector('td[data-select="row"] [part="select-mark"]')
        .getAttribute("aria-hidden"),
      lang: box.closest("[lang]")?.getAttribute("lang"),
    };
  });
  expect(name.label).toMatch(/select all/i);
  // A checkbox, and the cell around it is not pretending to be one.
  expect(name.role).toBe("checkbox");
  expect(name.cellRole).toBe(false);
  // The per-row mark stays decoration: the row says `aria-selected`, and a
  // second voice per row would double every announcement.
  expect(name.rowMarkHidden).toBe("true");
  // Our sentence, so it carries our language — and the data does not.
  expect(name.lang).toBe("en");
});

test("every control of the column meets the minimum target size", async ({ page }) => {
  // WCAG 2.2 §2.5.8 is about the **target** — what a pointer can hit — not
  // about the drawn box. The header's checkbox is drawn 16px because that is
  // the design; what has to be 24px is the area that responds. Measured by
  // hit-testing the corners of a 24x24 square around each control, which is
  // the only way to see a hit area that a bounding box does not report.
  const probes = await page.evaluate(() => {
    const SIZE = 24;
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const controls = [
      root.querySelector('[data-select="all"]'),
      root.querySelector('td[data-select="row"]'),
    ];
    return controls.map((control) => {
      const box = control.getBoundingClientRect();
      const cx = box.left + box.width / 2;
      const cy = box.top + box.height / 2;
      const half = SIZE / 2 - 0.5;
      const corners = [
        [cx - half, cy - half],
        [cx + half, cy - half],
        [cx - half, cy + half],
        [cx + half, cy + half],
      ];
      return {
        what: control.getAttribute("data-select"),
        // Every corner of the 24x24 square lands on the control (or inside it).
        // Strictly the control or something inside it. Accepting the cell
        // *around* it would make this pass for a 16px checkbox sitting in a
        // 44px header — and clicking that cell does nothing, because the
        // handler looks for `data-select` upwards, not downwards.
        hits: corners.every(([x, y]) => {
          const at = root.elementFromPoint(x, y);
          return !!at && (at === control || control.contains(at));
        }),
      };
    });
  });

  expect(probes.length).toBe(2);
  for (const probe of probes) {
    expect(probe.hits, `${probe.what} is smaller than 24x24`).toBe(true);
  }
});

test("has no axe violations, selected and unselected", async ({ page }) => {
  const first = await new AxeBuilder({ page }).analyze();
  expect(first.violations).toEqual([]);

  await page.evaluate(() =>
    document
      .querySelector("opengrid-grid")
      .shadowRoot.querySelector('[data-select="all"]')
      .click(),
  );
  const second = await new AxeBuilder({ page }).analyze();
  expect(second.violations).toEqual([]);
});
