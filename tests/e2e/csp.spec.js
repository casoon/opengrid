import { test, expect } from "@playwright/test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

// A strict Content-Security-Policy (issue #139): no JavaScript built at run
// time. Every MCP Apps host runs its views under one, and so do careful pages.

test("Intl formats work on a page without 'unsafe-eval'", async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-csp.html");
  await page.waitForFunction(() => window.__ready === true);
  await expect(page.locator("opengrid-grid tbody tr").first()).toBeVisible();

  const amount = await page.locator('opengrid-grid tbody tr td[data-col="1"]').first().textContent();
  expect(amount).toMatch(/\s€$/);
  // `dateStyle: "long"` in German: "30. Juli 2025".
  const date = await page.locator('opengrid-grid tbody tr td[data-col="2"]').first().textContent();
  expect(date).toMatch(/^\d{1,2}\. \p{L}+ \d{4}$/u);
  expect(await page.evaluate(() => window.__violations)).toEqual([]);
});

test("the element module builds no code at run time", () => {
  // The glue wasm-bindgen writes, and the snippets beside it.
  const pkg = join(process.cwd(), "packages/opengrid/pkg");
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.endsWith(".js")) files.push(path);
    }
  };
  walk(pkg);
  expect(files.length).toBeGreaterThan(0);
  for (const file of files) {
    const source = readFileSync(file, "utf8");
    expect(source, file).not.toMatch(/new Function\s*\(|\beval\s*\(/);
  }
});
