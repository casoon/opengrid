import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow } from "./helpers.js";

// Muted rows (issue #534): `row-muted="<field>"` names a boolean field, and a
// row whose value is `true` is drawn in the muted ink, carries the part
// `row-muted` and is described with the page's word for it.
//
// The fixture has sixty records, "Item 01" to "Item 60"; every third one is
// inactive. A row is muted exactly when its number divides by three, so a test
// reads the record from the row's own text and needs no table of positions.

const URL = "/tests/e2e/fixtures/grid-muted-rows.html";
const THEMES = ["base", "paper", "violet", "orange", "dark"];

async function open(page, query = "") {
  await page.goto(`${URL}${query}`);
  await page.waitForFunction(() => window.__ready === true);
  await expect.poll(() => drawn(page).then((rows) => rows.length)).toBeGreaterThan(0);
}

/** The rows drawn now, in display order, with what a muted row carries. */
const drawn = (page) =>
  shadow(page, (root) =>
    [...root.querySelectorAll("tbody tr")]
      .filter((tr) => tr.style.transform && !tr.style.display && tr.querySelector("td[data-col]"))
      .sort((a, b) => a.getAttribute("aria-rowindex") - b.getAttribute("aria-rowindex"))
      .map((tr) => ({
        kind: tr.dataset.kind ?? "row",
        item: Number(/Item (\d\d)/.exec(tr.textContent)?.[1] ?? NaN),
        text: tr.querySelector("td[data-col]").textContent,
        muted: tr.hasAttribute("data-muted"),
        part: tr.getAttribute("part"),
        described: tr.getAttribute("aria-describedby"),
        selected: tr.getAttribute("aria-selected"),
      })),
  );

/** Every drawn record row is muted exactly when its number divides by three. */
async function expectFollowsRecord(page) {
  const rows = (await drawn(page)).filter((row) => row.kind === "row");
  expect(rows.length).toBeGreaterThan(3);
  for (const row of rows) {
    expect(row.muted, `Item ${row.item} muted`).toBe(row.item % 3 === 0);
    expect(row.part, `Item ${row.item} part`).toBe(row.muted ? "row row-muted" : "row");
    expect(row.described, `Item ${row.item} description`).toBe(row.muted ? "og-row-muted" : null);
  }
  return rows;
}

const scrollTo = (page, top) =>
  shadow(page, (root, top) => (root.querySelector('[part="viewport"]').scrollTop = top), top);

const focusCell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), { row, col });

/** A colour as the page computes it, whatever notation the engine gives it. */
const SRGB = `
  const toRgba = (css) => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = css;
    context.fillRect(0, 0, 1, 1);
    const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
    return [r, g, b, a / 255];
  };
  const luminance = ([r, g, b]) => {
    const channel = (value) => {
      const v = value / 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
  };
`;

/**
 * The contrast of a cell's text against what is behind it: every translucent
 * layer from the page down to the cell composited, the way it is drawn.
 */
const contrastOf = (page, selector) =>
  shadow(
    page,
    new Function(
      "root",
      "selector",
      `${SRGB}
      const cell = root.querySelector(selector);
      let behind = [255, 255, 255];
      const chain = [];
      for (let node = cell; node; node = node.parentElement ?? node.getRootNode().host) chain.push(node);
      for (const node of chain.reverse()) {
        const [r, g, b, a] = toRgba(getComputedStyle(node).backgroundColor);
        behind = [r * a + behind[0] * (1 - a), g * a + behind[1] * (1 - a), b * a + behind[2] * (1 - a)];
      }
      const ink = toRgba(getComputedStyle(cell).color);
      const [high, low] = [luminance(ink), luminance(behind)].sort((x, y) => y - x);
      return { ratio: (high + 0.05) / (low + 0.05), ink: getComputedStyle(cell).color };`,
    ),
    selector,
  );

/** What the look's own tokens resolve to, as the page computes colours. */
const token = (page, name) =>
  page.evaluate((name) => {
    const host = document.querySelector("opengrid-grid");
    const probe = document.createElement("span");
    probe.style.color = `var(${name})`;
    // In the shadow root, where the tokens are drawn: a light child is not rendered.
    host.shadowRoot.append(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  }, name);

const colorOf = (page, selector) =>
  shadow(page, (root, selector) => getComputedStyle(root.querySelector(selector)).color, selector);

// ── The field, by the rows ───────────────────────────────────────────────

test("a row is muted by a field that is no column", async ({ page }) => {
  await open(page);
  // The field is fetched with the rows and drawn nowhere: no header, no cell.
  expect(await shadow(page, (root) => [...root.querySelectorAll("thead th")].map((th) => th.textContent).join(" "))).not.toMatch(/inactive/i);
  expect(await shadow(page, (root) => [...root.querySelectorAll("tbody td")].some((td) => /^(true|false)$/.test(td.textContent)))).toBe(false);
  const rows = await expectFollowsRecord(page);
  expect(rows.some((row) => row.muted)).toBe(true);
  expect(rows.some((row) => !row.muted)).toBe(true);
});

test("a muted row is drawn in the muted ink, an ordinary one in the ink", async ({ page }) => {
  await open(page);
  const mutedInk = await token(page, "--og-ink-muted");
  const ink = await token(page, "--og-ink");
  // Row 2 is Item 03, row 0 is Item 01.
  expect(await colorOf(page, 'td[data-row="2"][data-col="1"]')).toBe(mutedInk);
  expect(await colorOf(page, 'td[data-row="2"][data-col="0"]')).toBe(mutedInk);
  expect(await colorOf(page, 'td[data-row="0"][data-col="1"]')).toBe(ink);
});

test("the description is the page's word, and the default is `inactive`", async ({ page }) => {
  await open(page);
  const words = () =>
    shadow(page, (root) => {
      const row = root.querySelector("tbody tr[data-muted]");
      const node = root.getElementById(row.getAttribute("aria-describedby"));
      return { text: node.textContent, lang: node.getAttribute("lang") };
    });
  expect((await words()).text).toBe("inactive");

  await open(page, "?words");
  expect((await words()).text).toBe("archived");
  // A node nothing refers to is not kept: the words of a grid with no muted row.
  await open(page, "?row-muted=");
  expect(await shadow(page, (root) => root.getElementById("og-row-muted"))).toBeNull();
});

test("set_texts changes the words of rows already drawn", async ({ page }) => {
  await open(page);
  await page.evaluate(() => window.__module.set_texts(document.querySelector("opengrid-grid"), { rowMuted: "hidden from the customer" }));
  await expect
    .poll(() => shadow(page, (root) => root.getElementById("og-row-muted")?.textContent))
    .toBe("hidden from the customer");
  await expectFollowsRecord(page);
});

test("the state follows the record when rows are recycled", async ({ page }) => {
  // A pool of ten: with twelve, a slot would hold a record of the same
  // divisibility wherever the window stands, and a stale mark could not show.
  await open(page, "?window-size=10");
  expect((await expectFollowsRecord(page))[0].item).toBe(1);
  let before = 1;
  const windows = new Set();
  for (const top of [400, 800, 1200, 1600, 2000]) {
    await scrollTo(page, top);
    await expect.poll(async () => (await drawn(page))[0].item).not.toBe(before);
    before = (await drawn(page))[0].item;
    windows.add(before % 3);
    await expectFollowsRecord(page);
  }
  // At least one window starts where the first slot held another kind of record.
  expect(windows.size).toBeGreaterThan(1);
  // Back to the top: the same slots, the first rows again.
  await scrollTo(page, 0);
  await expect.poll(async () => (await drawn(page))[0].item).toBe(1);
  await expectFollowsRecord(page);
});

test("a shown column works the same, and the cell is drawn as any other", async ({ page }) => {
  await open(page, "?columns=name,qty,inactive");
  const rows = await expectFollowsRecord(page);
  expect(rows.some((row) => row.muted)).toBe(true);
  expect(await shadow(page, (root) => root.querySelector('td[data-row="2"][data-col="2"]').textContent)).toContain("true");
});

test("a page of the pager follows the record", async ({ page }) => {
  await open(page, "?page-size=10");
  await expectFollowsRecord(page);
  await shadow(page, (root) => root.querySelector('[part="page-next"]').click());
  await expect.poll(async () => (await drawn(page))[0].item).toBe(11);
  await expectFollowsRecord(page);
});

test("a grouped grid mutes its records and no group header", async ({ page }) => {
  await open(page, "?columns=region,name,qty&group-by=region");
  await focusCell(page, 0, 0);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page)).filter((row) => row.kind === "row").length).toBeGreaterThan(3);
  await expectFollowsRecord(page);
  for (const row of (await drawn(page)).filter((row) => row.kind !== "row")) {
    expect(row.muted).toBe(false);
    expect(row.part).toBe("row");
    expect(row.described).toBeNull();
  }
  // Collapse it and open the other one: slots that were muted rows are headers
  // and records of another group now.
  await focusCell(page, 0, 0);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page)).filter((row) => row.kind === "row").length).toBe(0);
  await focusCell(page, 1, 0);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page)).filter((row) => row.kind === "row").length).toBeGreaterThan(3);
  await expectFollowsRecord(page);
});

test("a group header or the total in the slot of a muted record is not muted", async ({ page }) => {
  // Grouped by the field itself: the group `true` holds nothing but muted
  // records. Closed again, its slots draw the total and a header.
  await open(page, "?columns=inactive,name,qty&group-by=inactive");
  await focusCell(page, 1, 0);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page)).filter((row) => row.kind === "row").length).toBeGreaterThan(3);
  const open_ = await drawn(page);
  expect(open_.filter((row) => row.kind === "row").every((row) => row.muted)).toBe(true);
  expect(open_.filter((row) => row.kind !== "row").every((row) => !row.muted)).toBe(true);
  await focusCell(page, 1, 0);
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page)).filter((row) => row.kind === "row").length).toBe(0);
  const closed = await drawn(page);
  expect(closed.map((row) => row.kind)).toEqual(["group", "group", "total"]);
  for (const row of closed) {
    expect(row, row.kind).toMatchObject({ muted: false, part: "row", described: null });
  }
});

test("a tree mutes its nodes, the field being no column", async ({ page }) => {
  await open(page, "?tree=parent_id");
  // The roots are Items 01 to 06.
  const roots = await expectFollowsRecord(page);
  expect(roots.map((row) => row.item)).toEqual([1, 2, 3, 4, 5, 6]);
  expect(roots.filter((row) => row.muted).map((row) => row.item)).toEqual([3, 6]);
  // Open Item 01: its children (7, 13, 19, …) come in, and follow their records.
  await focusCell(page, 0, 0);
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await drawn(page)).length).toBeGreaterThan(6);
  const opened = await expectFollowsRecord(page);
  expect(opened.slice(0, 3).map((row) => row.item)).toEqual([1, 7, 13]);
  // A scroll cuts another window out of the loaded levels, and it follows too.
  await scrollTo(page, 10000);
  // (The row the focus stands on keeps its slot, so the first row stays Item 01.)
  await expect.poll(async () => (await drawn(page)).some((row) => row.item === 4)).toBe(true);
  await expectFollowsRecord(page);
});

test("the attribute set or removed at run time asks again", async ({ page }) => {
  await open(page, "?row-muted=");
  expect((await drawn(page)).some((row) => row.muted)).toBe(false);
  await page.evaluate(() => document.querySelector("opengrid-grid").setAttribute("row-muted", "inactive"));
  await expect.poll(async () => (await drawn(page)).some((row) => row.muted)).toBe(true);
  await expectFollowsRecord(page);
  await page.evaluate(() => document.querySelector("opengrid-grid").removeAttribute("row-muted"));
  await expect.poll(async () => (await drawn(page)).some((row) => row.muted)).toBe(false);
  expect((await drawn(page)).every((row) => row.part === "row" && row.described === null)).toBe(true);
});

test("a value that is not `true` mutes nothing", async ({ page }) => {
  // `qty` is a number, not a boolean: nothing is muted by it.
  await open(page, "?row-muted=qty");
  expect((await drawn(page)).some((row) => row.muted)).toBe(false);
});

// ── Presentation only ────────────────────────────────────────────────────

test("a muted row stays selectable, and its selection and focus still show", async ({ page }) => {
  await open(page);
  // Item 03 is row 2, Item 01 is row 0.
  await focusCell(page, 2, 0);
  await page.keyboard.press(" ");
  await focusCell(page, 0, 0);
  await page.keyboard.press(" ");
  await expect
    .poll(() => shadow(page, (root) => root.querySelectorAll('tbody tr[data-selected="true"]').length))
    .toBe(2);
  const look = (row) =>
    shadow(
      page,
      (root, row) => {
        const tr = root.querySelector(`td[data-row="${row}"]`).parentElement;
        const style = getComputedStyle(tr);
        return { selected: tr.getAttribute("aria-selected"), background: style.backgroundColor, bar: style.boxShadow };
      },
      row,
    );
  const muted = await look(2);
  const plain = await look(0);
  expect(muted.selected).toBe("true");
  // The tint and the bar are the selection's, whatever the row.
  expect(muted.background).toBe(plain.background);
  expect(muted.bar).toBe(plain.bar);
  expect(muted.bar).not.toBe("none");
  // The row is still muted, and said so.
  const rows = await drawn(page);
  expect(rows[2]).toMatchObject({ item: 3, muted: true, selected: "true", described: "og-row-muted" });

  // The focus ring is the cell's own.
  await focusCell(page, 2, 1);
  const ring = await shadow(page, (root) => {
    const style = getComputedStyle(root.querySelector('td[data-row="2"][data-col="1"]'));
    return { width: style.outlineWidth, style: style.outlineStyle };
  });
  expect(ring.style).toBe("solid");
  expect(ring.width).not.toBe("0px");
});

test("a hovered muted row is tinted like any hovered row", async ({ page }) => {
  await open(page);
  const hover = async (row) => {
    await page.locator("opengrid-grid").locator(`td[data-row="${row}"][data-col="1"]`).hover();
    return shadow(page, (root, row) => getComputedStyle(root.querySelector(`td[data-row="${row}"]`).parentElement).backgroundColor, row);
  };
  const plain = await hover(0);
  const muted = await hover(2);
  expect(muted).toBe(plain);
  expect(muted).not.toBe("rgba(0, 0, 0, 0)");
});

test("a muted row can still be edited", async ({ page }) => {
  await open(page);
  await focusCell(page, 2, 0);
  await page.keyboard.press("Enter");
  expect(await shadow(page, (root) => root.querySelector('[part="editor"]') !== null)).toBe(true);
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.type("Renamed");
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__changes.length)).toBe(1);
  expect(await page.evaluate(() => window.__changes[0])).toMatchObject({ column: "name", value: "Renamed", key: 3 });
  // Still the same record, and still muted.
  expect((await drawn(page))[2]).toMatchObject({ muted: true });
});

test("editing the field of a shown column mutes or un-mutes the row at once", async ({ page }) => {
  await open(page, "?columns=name,qty,inactive");
  expect((await drawn(page))[0].muted).toBe(false);
  await focusCell(page, 0, 2);
  await page.keyboard.press("Enter");
  expect(await shadow(page, (root) => root.querySelector('[part="editor"]') !== null)).toBe(true);
  // A boolean edits as a checkbox: `Space` ticks it.
  await page.keyboard.press(" ");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await drawn(page))[0].muted).toBe(true);
});

// ── The five looks, and forced colours ───────────────────────────────────

for (const theme of THEMES) {
  test(`muted text keeps 4.5:1 in the ${theme} look — plain, hovered and selected`, async ({ page }) => {
    await open(page, `?theme=${theme}`);
    const muted = 'td[data-row="2"][data-col="1"]';
    const plain = await contrastOf(page, muted);
    expect(plain.ratio, `plain, ${plain.ink}`).toBeGreaterThanOrEqual(4.5);

    await page.locator("opengrid-grid").locator(muted).hover();
    const hovered = await contrastOf(page, muted);
    expect(hovered.ratio, `hovered, ${hovered.ink}`).toBeGreaterThanOrEqual(4.5);

    // Selected, with the focus on the row: the tint and the crosshair together.
    await focusCell(page, 2, 1);
    await page.keyboard.press(" ");
    await expect
      .poll(() => shadow(page, (root) => root.querySelector('td[data-row="2"]').parentElement.dataset.selected))
      .toBe("true");
    const selected = await contrastOf(page, muted);
    expect(selected.ratio, `selected and focused, ${selected.ink}`).toBeGreaterThanOrEqual(4.5);

    // The muted ink is what the look says, not a colour of the row's own.
    expect(selected.ink).toBe(await token(page, "--og-ink-muted"));
    // It differs from the ink, or a muted row would not be muted.
    expect(selected.ink).not.toBe(await token(page, "--og-ink"));
  });

  test(`the ${theme} look has no axe violations with muted rows, selected and not`, async ({ page }) => {
    await open(page, `?theme=${theme}&columns=name,qty,inactive`);
    await focusCell(page, 2, 0);
    await page.keyboard.press(" ");
    await focusCell(page, 3, 0);
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
}

test.describe("under a forced palette", () => {
  test.use({ forcedColors: "active" });

  test("a muted row says it in the letters, and keeps its selection bar", async ({ page, browserName }) => {
    test.skip(
      browserName === "webkit",
      "WebKit matches forced-colors under emulation but forces no colour, not even a plain div's",
    );
    await open(page);
    const style = (row, name) =>
      shadow(page, (root, { row, name }) => getComputedStyle(root.querySelector(`td[data-row="${row}"][data-col="1"]`))[name], { row, name });
    // The muted ink is the ink here: colour cannot tell the rows apart.
    expect(await colorOf(page, 'td[data-row="2"][data-col="1"]')).toBe(await colorOf(page, 'td[data-row="0"][data-col="1"]'));
    expect(await style(2, "fontStyle")).toBe("italic");
    expect(await style(0, "fontStyle")).toBe("normal");

    await focusCell(page, 2, 0);
    await page.keyboard.press(" ");
    await expect
      .poll(() => shadow(page, (root) => root.querySelector('td[data-row="2"]').parentElement.dataset.selected))
      .toBe("true");
    const bar = await shadow(page, (root) => {
      const style = getComputedStyle(root.querySelector('tbody tr[data-selected="true"] > :first-child'));
      return { width: style.borderInlineStartWidth, style: style.borderInlineStartStyle };
    });
    expect(bar).toEqual({ width: "4px", style: "solid" });
  });

  test("has no axe violations", async ({ page }) => {
    await open(page, "?columns=name,qty,inactive");
    const { violations } = await new AxeBuilder({ page }).analyze();
    expect(violations).toEqual([]);
  });
});
