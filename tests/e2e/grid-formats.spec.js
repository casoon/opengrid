import { test, expect } from "@playwright/test";

// Per-column display formatting (plan point 42).
//
// The point is not that numbers get thousands separators — it is that the
// **query does not change**. A grid that sorted or filtered by what it printed
// would have stopped honouring rules S4, S8 and S9.

/** The text of one column of the rendered rows. */
function column(page, col) {
  return page
    .locator(`opengrid-grid tbody tr td[data-col="${col}"]`)
    .allTextContents();
}

/** Every query the fixture's provider has seen. */
function queries(page) {
  return page.evaluate(() => window.__queries ?? []);
}

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-formats.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

test("a column is formatted for the eye, not for the query", async ({ page }) => {
  await open(page);

  // `amount` is a decimal, formatted as German currency.
  const shown = await column(page, 2);
  expect(shown[0]).toMatch(/ €$/);
  expect(shown[0]).toContain(",");

  // The query asked for the plain column — no formatting reached it.
  const asked = await queries(page);
  expect(asked.length).toBeGreaterThan(0);
  expect(JSON.stringify(asked)).not.toContain("€");
});

test("changing the format redraws without asking the source again", async ({ page }) => {
  await open(page);
  const before = (await queries(page)).length;

  await page.evaluate(() => window.__setFormats({ amount: { kind: "number", locale: "en-US" } }));
  await expect.poll(() => column(page, 2).then((cells) => cells[0])).not.toContain("€");

  expect((await queries(page)).length).toBe(before);
});

test("an unformatted column keeps the project's own notation", async ({ page }) => {
  await open(page);
  // `id` has no format: an integer stays an integer, exactly as the engine
  // wrote it.
  const ids = await column(page, 0);
  expect(ids[0]).toMatch(/^\d+$/);
});

test("a formatter that throws leaves the value visible", async ({ page }) => {
  await open(page);
  await page.evaluate(() =>
    window.__setFormats({
      amount: () => {
        throw new Error("nope");
      },
    }),
  );
  await expect
    .poll(() => column(page, 2).then((cells) => cells[0]))
    .toMatch(/\d/, "a broken formatter must not blank the cell");
});

// An `Intl` format without a `locale` reads as the page does (issue #232): the
// explicit `locale`, then the `lang` of the page, then the browser's. A page in
// German in an English browser shows "30,00" while the screen reader speaks
// German. The fixture's browser is en-US, so the two are told apart by the
// decimal separator.
test.describe("the page's language", () => {
  const first = (page) => column(page, 2).then((cells) => cells[0]);
  const set = (page, formats) => page.evaluate((formats) => window.__setFormats(formats), formats);

  test("an Intl format without a locale follows the page's lang", async ({ page }) => {
    await open(page);
    await page.evaluate(() => (document.documentElement.lang = "de"));
    await set(page, { amount: { kind: "number", minimumFractionDigits: 2 } });
    await expect.poll(() => first(page)).toMatch(/^\d+,\d\d$/);
  });

  test("an explicit locale beats the page's lang", async ({ page }) => {
    await open(page);
    await page.evaluate(() => (document.documentElement.lang = "en"));
    await set(page, { amount: { kind: "number", locale: "de-DE", minimumFractionDigits: 2 } });
    await expect.poll(() => first(page)).toMatch(/^\d+,\d\d$/);
    // ... and a lang that changes later does not move it.
    await page.evaluate(() => (document.documentElement.lang = "fr"));
    await page.evaluate(() => new Promise((resolve) => setTimeout(resolve, 100)));
    expect(await first(page)).toMatch(/^\d+,\d\d$/);
  });

  test("a change of lang relocalizes without a new set_formats", async ({ page }) => {
    await open(page);
    await set(page, { amount: { kind: "number", minimumFractionDigits: 2 } });
    await expect.poll(() => first(page)).toMatch(/^\d+\.\d\d$/);
    const before = (await queries(page)).length;

    await page.evaluate(() => (document.documentElement.lang = "de"));
    await expect.poll(() => first(page)).toMatch(/^\d+,\d\d$/);
    // The nearest `lang` wins: an ancestor of the grid, then back again.
    await page.evaluate(() => document.querySelector("main").setAttribute("lang", "en"));
    await expect.poll(() => first(page)).toMatch(/^\d+\.\d\d$/);
    await page.evaluate(() => document.querySelector("main").removeAttribute("lang"));
    await expect.poll(() => first(page)).toMatch(/^\d+,\d\d$/);
    // Display only: the source was never asked again.
    expect((await queries(page)).length).toBe(before);
  });
});
