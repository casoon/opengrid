import { test, expect } from "@playwright/test";

// The production build of a bundler (#238): examples/bundler is built by Vite
// with its defaults — minified, production mode — and calls `loadOpengrid()` and
// `createWorkerProvider()` without a single URL. The bundler has to find the
// element module, the engine, the worker and both `.wasm` files on its own.

const PAGE = "/examples/bundler/dist/index.html";

test("a Vite production build loads the element module and the worker's engine without any URL", async ({
  page,
}) => {
  const problems = [];
  page.on("console", (message) => {
    if (message.type() === "error" || message.type() === "warning") problems.push(message.text());
  });
  page.on("pageerror", (error) => problems.push(error.message));
  const wasm = [];
  page.on("response", (response) => {
    if (response.url().endsWith(".wasm")) wasm.push(response);
  });

  await page.goto(PAGE);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid")?.shadowRoot?.querySelector("td[data-row]"),
  );

  // The module loaded: not the plain-table fallback.
  expect(await page.evaluate(() => window.__fallback)).toBe(false);
  // And the worker's engine answered: the rows are the worker's.
  const cells = await page.evaluate(() =>
    [...document.querySelector("opengrid-grid").shadowRoot.querySelectorAll("tbody tr")].map((tr) =>
      [...tr.querySelectorAll("td[data-row]")].map((td) => td.textContent.trim()).join(","),
    ),
  );
  expect(cells.filter((row) => row.replaceAll(",", ""))).toEqual(["1,A1,10", "2,B2,20", "3,C3,30"]);

  // Both modules reached the build's output, with the type a browser streams.
  expect(wasm).toHaveLength(2);
  for (const response of wasm) {
    expect(response.status()).toBe(200);
    expect(response.headers()["content-type"]).toBe("application/wasm");
  }
  expect(problems).toEqual([]);
});
