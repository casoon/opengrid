import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { shadow, status } from "./helpers.js";

// A source that changes under the grid (issue #328): a notice and a button,
// never rows that move on their own.

const cell = (page, row, col) =>
  shadow(page, (root, { row, col }) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`)?.textContent, { row, col });
const buttons = (page) => shadow(page, (root) => root.querySelectorAll('[part="refresh"]').length);
const where = (page) =>
  shadow(page, (root) => {
    const active = root.activeElement;
    return active ? `${active.tagName.toLowerCase()}:${active.dataset.row ?? ""}:${active.dataset.col ?? ""}` : null;
  });

async function open(page) {
  await page.goto("/tests/e2e/fixtures/grid-live.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
}

const tell = (page) =>
  page.evaluate(() =>
    document.querySelector("opengrid-grid").dispatchEvent(new CustomEvent("opengrid-source-changed", { detail: { name: "customers" }, cancelable: true })),
  );

test("a change is a notice and a button: nothing moves, nothing is said twice", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="2"][data-col="1"]').focus());
  // The data changes behind the grid's back, then the page tells it.
  await page.evaluate(() => window.__reload(0));
  await tell(page);
  await expect.poll(() => status(page)).toContain("The data changed. Refresh to see it.");
  expect(await buttons(page)).toBe(1);
  // Nothing moved: the rows are the old ones and the focus is where it was.
  expect(await cell(page, 0, 0)).toBe("1");
  expect(await where(page)).toBe("td:2:1");

  // More changes while the notice stands are the same notice.
  await tell(page);
  await tell(page);
  expect(await buttons(page)).toBe(1);
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("Refresh reads the change in and the focus follows its record", async ({ page }) => {
  await open(page);
  await shadow(page, (root) => root.querySelector('td[data-row="2"][data-col="1"]').focus());
  // Record 3 is focused; a record 0 arrives before it.
  await page.evaluate(() => window.__reload(0));
  await tell(page);
  await expect.poll(() => buttons(page)).toBe(1);
  // By keyboard: Safari does not focus a button a pointer presses, so the
  // focus has left the grid before the button is activated there.
  await page.locator("opengrid-grid").locator('[part="refresh"]').focus();
  await page.keyboard.press("Enter");
  await expect.poll(() => cell(page, 0, 0)).toBe("0");
  expect(await buttons(page)).toBe(0);
  // The record the focus stood on is one row further down now, and still has it.
  expect(await cell(page, 3, 0)).toBe("3");
  expect(await where(page)).toBe("td:3:1");
});

test("a page that keeps the event from its default gets no notice, and refreshes when it likes", async ({ page }) => {
  await open(page);
  await page.evaluate(() => (window.__prevent = true));
  await page.evaluate(() => window.__reload(0));
  await tell(page);
  expect(await page.evaluate(() => window.__changed)).toEqual([{ name: "customers", cancelable: true }]);
  expect(await buttons(page)).toBe(0);
  expect(await cell(page, 0, 0)).toBe("1");
  await page.evaluate(() => window.__module.refresh(document.querySelector("opengrid-grid")));
  await expect.poll(() => cell(page, 0, 0)).toBe("0");
});

test("a REST provider's change stream tells the grid, and a 4xx ends it", async ({ page }) => {
  await open(page);
  let asked = 0;
  // Another origin, with a token: Firefox and WebKit send a preflight first.
  const cors = {
    "access-control-allow-origin": "*",
    "access-control-allow-headers": "authorization, accept, cache-control",
  };
  await page.route("**/watch/customers", async (route) => {
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers: cors });
      return;
    }
    asked += 1;
    expect(route.request().headers().authorization).toBe("Bearer t0ken");
    await route.fulfill({
      status: 200,
      headers: { ...cors, "content-type": "text/event-stream" },
      body: 'retry: 100\n\n: ready\n\nevent: changed\ndata: {"source":"customers"}\n\n',
    });
  });
  await page.evaluate(() => {
    const provider = createRest();
    function createRest() {
      return window.__rest("http://localhost:8765");
    }
    window.__provider = provider;
  });
  // `token` is part of the provider: build one that has it.
  await page.evaluate(async () => {
    const { createRestProvider } = await import("/packages/opengrid/loader.js");
    window.__provider.terminate();
    window.__provider = createRestProvider({ url: "http://localhost:8765", source: "customers", token: "t0ken", watch: true });
    window.__provider.watch(document.querySelector("opengrid-grid"));
  });
  await expect.poll(() => page.evaluate(() => window.__changed.length)).toBeGreaterThan(0);
  expect((await page.evaluate(() => window.__changed))[0].name).toBe("customers");
  // The stream is taken up again after the server's `retry`.
  await expect.poll(() => asked).toBeGreaterThan(1);

  await page.unroute("**/watch/customers");
  let refused = 0;
  await page.route("**/watch/customers", async (route) => {
    if (route.request().method() === "OPTIONS") {
      await route.fulfill({ status: 204, headers: cors });
      return;
    }
    refused += 1;
    await route.fulfill({ status: 404, headers: cors, body: "{}" });
  });
  const before = refused;
  await page.waitForTimeout(1200);
  // A 4xx is final: at most the one request that was already on its way.
  expect(refused - before).toBeLessThanOrEqual(2);
  await page.evaluate(() => window.__provider.terminate());
});
