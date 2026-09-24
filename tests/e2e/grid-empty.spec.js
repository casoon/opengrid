import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// The empty state (plan point 68). Two situations, two sentences: no row
// matches the filters — with a way out — or the source has no rows at all,
// where a reset would promise what it cannot do. And the panel is silent: the
// status line already says "No matches".

async function panel(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const panel = root.querySelector('[part="empty"]');
    const reset = root.querySelector('[part="empty-reset"]');
    return {
      shown: !panel.hidden && panel.getBoundingClientRect().height > 0,
      text: root.querySelector('[part="empty-text"]').textContent,
      reset: !reset.hidden,
      live: panel.getAttribute("aria-live") ?? panel.getAttribute("role"),
    };
  });
}

async function status(page) {
  return page.evaluate(() =>
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]').textContent.trim(),
  );
}

async function filterToNothing(page) {
  await page.evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const select = root.querySelector('select[data-col="2"]');
    select.value = "eq";
    select.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    const input = root.querySelector('input[data-col="2"]');
    input.value = "XX";
    input.focus();
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, composed: true }));
  });
}

test.describe("with rows in the source", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/tests/e2e/fixtures/grid-empty.html");
    await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
    await expect.poll(() => status(page)).toBe("200 matches");
  });

  test("there is no panel while there are rows", async ({ page }) => {
    expect((await panel(page)).shown).toBe(false);
  });

  test("a filter that matches nothing shows the panel with a way out", async ({ page }) => {
    await filterToNothing(page);
    await expect.poll(() => status(page)).toBe("No matches");
    expect(await panel(page)).toEqual({
      shown: true,
      text: "No row matches these filters.",
      reset: true,
      live: null,
    });
  });

  test("the reset works from the keyboard and is the same as Remove all", async ({ page }) => {
    // The grid's keys do not reach the panel: a button inside the viewport kept
    // its native Enter — the trap the pager (38) and the chips (65) fell into.
    await filterToNothing(page);
    await expect.poll(() => status(page)).toBe("No matches");
    await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="empty-reset"]').focus());
    await page.keyboard.press("Enter");
    await expect.poll(() => status(page)).toMatch(/^200 matches/);
    expect((await panel(page)).shown).toBe(false);
    // The button is gone with the rows back; the focus is at the top of the
    // grid, not lost to the document.
    expect(
      await page.evaluate(() => document.querySelector("opengrid-grid").shadowRoot.activeElement?.tagName),
    ).toBe("TH");
  });

  test("'No matches' is said once — the panel adds no second voice", async ({ page }) => {
    await page.evaluate(() => {
      const line = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="status"]');
      window.__said = [];
      new MutationObserver(() => {
        const text = line.textContent.trim();
        if (window.__said.at(-1) !== text) window.__said.push(text);
      }).observe(line, { childList: true, characterData: true, subtree: true });
    });
    await filterToNothing(page);
    await expect.poll(() => status(page)).toBe("No matches");
    await page.waitForTimeout(200);
    const said = await page.evaluate(() => window.__said);
    expect(said.filter((text) => text.includes("No matches"))).toHaveLength(1);
  });

  test("has no axe violations with the panel shown", async ({ page }) => {
    await filterToNothing(page);
    await expect.poll(() => status(page)).toBe("No matches");
    expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
  });
});

test("an empty source says so, and offers no reset", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-empty.html?source=empty");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await expect.poll(() => status(page)).toBe("No matches");
  expect(await panel(page)).toEqual({ shown: true, text: "There are no rows.", reset: false, live: null });
});
