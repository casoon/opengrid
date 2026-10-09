import { test, expect } from "@playwright/test";

// `<opengrid-grid>` under a strict style policy: `style-src 'self'` and
// `script-src 'self' 'wasm-unsafe-eval'`, no 'unsafe-inline', no nonce — the
// policy of an application that hashes or files every style and script of its
// own. A `<style>` element in the shadow root or a `style` attribute set with
// `setAttribute` is refused there, silently for the reader: the grid draws
// without its look, its widths and its row positions. Constructed sheets and
// the CSSOM are not governed by `style-src`, and that is what the grid uses.

/** Every refusal the page saw, from the event and from the console. */
async function open(page) {
  const refused = [];
  page.on("console", (message) => {
    if (message.type() === "error" && /Content Security Policy/i.test(message.text())) {
      refused.push(message.text());
    }
  });
  // Before any of the page's own code: an init script is not the page's, so
  // the policy does not refuse it, and it sees the very first violation.
  await page.addInitScript(() => {
    window.__violations = [];
    document.addEventListener("securitypolicyviolation", (event) =>
      window.__violations.push(`${event.violatedDirective} ${event.blockedURI}`),
    );
  });
  await page.goto("/tests/e2e/fixtures/grid-csp-strict.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();
  return async () => [...(await page.evaluate(() => window.__violations)), ...refused];
}

/** Runs `fn(root, arg)` in the page, on the grid's shadow root. */
function shadow(page, fn, arg) {
  // Composed here rather than with `new Function` in the page: the page's
  // policy has no 'unsafe-eval', and Playwright's own evaluation is exempt.
  return page.evaluate(
    `(${fn.toString()})(document.querySelector("opengrid-grid").shadowRoot, ${JSON.stringify(arg ?? null)})`,
  );
}

test("the grid draws, sizes and scrolls without a single refused style", async ({ page }) => {
  const violations = await open(page);

  // The look arrived: the grid's sheet sets the row height on the host. The
  // tbody's position is read from its inline style, which a refused `style`
  // attribute leaves empty: WebKit computes `static` for a row group whatever
  // it is given, with or without a policy.
  const look = await shadow(page, (root) => ({
    rowHeight: getComputedStyle(root.host).getPropertyValue("--og-row-height").trim(),
    position: root.querySelector("tbody").style.position,
  }));
  expect(look).toEqual({ rowHeight: "42px", position: "relative" });
  const row = 42;

  // The configured widths reach the header and the values under it alike.
  const widths = await shadow(page, (root) =>
    ["0", "1"].map((col) => {
      const th = root.querySelector(`th[data-col="${col}"]`).getBoundingClientRect();
      const td = root.querySelector(`td[data-row="0"][data-col="${col}"]`).getBoundingClientRect();
      return { th: Math.round(th.width), td: Math.round(td.width), left: Math.round(td.left - th.left) };
    }),
  );
  expect(widths).toEqual([
    { th: 140, td: 140, left: 0 },
    { th: 200, td: 200, left: 0 },
  ]);

  // The body is as tall as all its rows, so the viewport scrolls …
  const sized = await shadow(page, (root) => {
    const viewport = root.querySelector('[part="viewport"]');
    return { scroll: viewport.scrollHeight, client: viewport.clientHeight };
  });
  expect(sized.scroll).toBeGreaterThan(200 * row);
  expect(sized.scroll).toBeGreaterThan(sized.client);

  // … and a row far down is drawn where it belongs, below its predecessor.
  await shadow(
    page,
    (root, top) => {
      root.querySelector('[part="viewport"]').scrollTop = top;
    },
    150 * row,
  );
  await expect
    .poll(() => shadow(page, (root) => !!root.querySelector('td[data-row="152"]')))
    .toBe(true);
  const placed = await shadow(page, (root) => {
    const tr = (row) => root.querySelector(`td[data-row="${row}"]`).closest("tr");
    const shift = (row) => new DOMMatrix(getComputedStyle(tr(row)).transform).m42;
    const top = (row) => tr(row).getBoundingClientRect().top;
    const hidden = [...root.querySelectorAll("tbody tr")].filter(
      (row) => getComputedStyle(row).display === "none",
    ).length;
    return { offset: shift(152), step: Math.round(top(153) - top(152)), hidden };
  });
  expect(placed).toEqual({ offset: 152 * row, step: row, hidden: 0 });

  expect(await violations()).toEqual([]);
});

test("a marks sheet edits under the policy: choices, read-only, the event", async ({ page }) => {
  const violations = await open(page);
  const focus = (row, col) =>
    shadow(page, (root, [row, col]) => root.querySelector(`td[data-row="${row}"][data-col="${col}"]`).focus(), [row, col]);

  // A read-only key opens nothing.
  await focus(0, 0);
  await page.keyboard.press("Enter");
  expect(await shadow(page, (root) => root.querySelector('[part="editor"]'))).toBeNull();

  // A listed column opens a select, and the commit reaches the page.
  await focus(1, 1);
  await page.keyboard.press("Enter");
  const editor = page.locator('opengrid-grid [part="editor"]');
  await expect(editor).toHaveJSProperty("tagName", "SELECT");
  await editor.selectOption("Gamma");
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__changes)).toEqual([
    expect.objectContaining({ row: 1, column: "customer", value: "Gamma" }),
  ]);
  expect(await violations()).toEqual([]);
});
