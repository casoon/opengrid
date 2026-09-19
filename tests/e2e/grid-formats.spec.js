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
