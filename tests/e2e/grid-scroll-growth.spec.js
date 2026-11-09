import { test, expect } from "@playwright/test";

// Issue #236: "heap growth over a long scroll". The heap itself is not something a
// test can read without a timer and a garbage collector deciding the outcome
// (grid-lifecycle.spec.js proves what a removed grid lets go of, with `gc()`).
// What can be read exactly is what a leaking scroll handler would leave in the
// page: nodes. The pool is fixed, so a scroll over the whole result — down, and
// back up — must end with as many nodes, and as many listeners' worth of
// children, as it began with.

const nodes = (page) =>
  page.evaluate(() => {
    const host = document.querySelector("opengrid-grid");
    return {
      shadow: host.shadowRoot.querySelectorAll("*").length,
      light: host.querySelectorAll("*").length,
      document: document.querySelectorAll("*").length,
      sheets: host.shadowRoot.adoptedStyleSheets.length + host.shadowRoot.styleSheets.length,
    };
  });

/** Scrolls to `top` and waits until the window has followed. */
async function scrollTo(page, top) {
  await page.evaluate((top) => {
    document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]').scrollTop = top;
  }, top);
  await page.waitForFunction((top) => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    const first = Math.floor(top / 32);
    return !!root.querySelector(`tbody tr[aria-rowindex] td[data-row="${first}"]`);
  }, top);
}

test("a scroll over the whole result and back leaves the page with as many nodes as it had", async ({
  page,
}) => {
  await page.goto("/tests/e2e/fixtures/grid-virtual.html");
  await page.evaluate(() => window.__opengridReady);
  await page.waitForSelector("opengrid-grid td[data-row]");

  // One round first: the first scroll may create what the grid keeps after it
  // (a status text, the focus ring's neighbours).
  await scrollTo(page, 3000);
  await scrollTo(page, 0);
  const before = await nodes(page);
  expect(before.shadow).toBeGreaterThan(30);

  const max = await page.evaluate(() => {
    const viewport = document.querySelector("opengrid-grid").shadowRoot.querySelector('[part="viewport"]');
    return viewport.scrollHeight - viewport.clientHeight;
  });
  for (let step = 0; step <= 120; step++) await scrollTo(page, Math.round((max * step) / 120));
  for (let step = 120; step >= 0; step--) await scrollTo(page, Math.round((max * step) / 120));

  expect(await nodes(page)).toEqual(before);
});
