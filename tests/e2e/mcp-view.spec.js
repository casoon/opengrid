import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { join } from "node:path";

// The grid as an MCP App (#140): the view's resource in a sandboxed iframe
// under the hosts' default CSP, its tool calls answered by a real
// opengrid-mcp server in Node — what Claude Desktop does with it.

const repo = process.cwd();
const { openClient } = await import(join(repo, "packages/opengrid-mcp/test/serve.mjs"));
const CONFIG = {
  sources: {
    orders: {
      csv: join(repo, "tests/e2e/fixtures/grid-virtual.csv"),
      schema: join(repo, "crates/opengrid-conformance/data/orders.schema.json"),
      columns: ["id", "customer", "country", "amount", "qty"],
      title: "Orders",
    },
  },
};

let client;
let sessionId;

const view = (page) => page.frameLocator('iframe[title="opengrid"]');
const frame = (page) => page.frames().find((candidate) => candidate !== page.mainFrame());

/** Records every text of the grid's status line inside the view. */
async function record(page) {
  await frame(page).evaluate(() => {
    const root = document.querySelector("opengrid-grid").shadowRoot;
    window.__said = [];
    const read = () => {
      const text = root.querySelector('[part="status"]')?.textContent.trim();
      if (text && window.__said.at(-1) !== text) window.__said.push(text);
    };
    new MutationObserver(read).observe(root, { subtree: true, childList: true, characterData: true });
  });
}
const status = (page) => view(page).locator('opengrid-grid [part="status"]');

test.beforeEach(async ({ page }) => {
  client = await openClient(CONFIG);
  await page.exposeFunction("__callTool", (name, args) => client.callTool({ name, arguments: args }));
  await page.goto("/tests/e2e/fixtures/mcp-host.html");
  await page.waitForFunction(() => window.__hostReady === true);
  const resource = await client.readResource({ uri: "ui://opengrid/grid" });
  await page.evaluate((html) => window.__startHost({ html }), resource.contents[0].text);
  // The model opens the grid; the host hands the result to the view.
  const opened = await client.callTool({ name: "opengrid_open", arguments: { source: "orders" } });
  sessionId = opened.structuredContent.sessionId;
  await page.evaluate((result) => window.__deliver(result), opened);
  await expect(view(page).locator("opengrid-grid tbody tr").first()).toBeVisible();
});

test("the grid runs in the host's sandbox, its rows from the server", async ({ page }) => {
  await expect(view(page).locator("opengrid-grid table")).toHaveAttribute("aria-rowcount", "201");
  await expect(status(page)).toContainText("200 matches");
  await expect(view(page).locator("opengrid-grid")).toHaveAttribute("label", "Orders");
});

test("a view the model sets reaches the rendered grid, said once", async ({ page }) => {
  await record(page);
  const set = await client.callTool({
    name: "opengrid_set_view",
    arguments: { sessionId, view: { filters: [{ column: "country", op: "eq", value: "DE" }] } },
  });
  expect(set.structuredContent.total).toBe(52);
  // The view picks it up on its next sync (visible: within a few seconds).
  await expect(status(page)).toHaveText("52 matches · Filtered by the assistant", { timeout: 10_000 });
  await page.waitForTimeout(300);
  // Said once: one utterance carries the notice, and no loading line does.
  const said = await frame(page).evaluate(() => window.__said);
  expect(said.filter((text) => text.includes("Filtered by the assistant"))).toEqual([
    "52 matches · Filtered by the assistant",
  ]);
});

test("what the reader does reaches the server and the model's context", async ({ page }) => {
  await view(page).locator('opengrid-grid th[data-col="3"]').focus();
  await page.keyboard.press("Enter");
  await expect
    .poll(async () => (await client.callTool({ name: "opengrid_describe", arguments: { sessionId } })).structuredContent.view.sort)
    .toEqual([{ field: "amount", direction: "asc" }]);
  await expect
    .poll(() => page.evaluate(() => window.__host.contexts.at(-1)?.content?.[0]?.text ?? ""))
    .toContain("Sort: amount asc");
});

test("a selection can be handed to the model, by keyboard", async ({ page }) => {
  const button = view(page).getByRole("button", { name: /Analyse the selection/ });
  await expect(button).toBeHidden();
  await view(page).locator('opengrid-grid td[data-row="0"][data-col="0"]').focus();
  await page.keyboard.press(" ");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Shift+ ");
  await expect(button).toHaveText("Analyse the selection (2)");

  const rows = (await client.callTool({ name: "opengrid_selection", arguments: { sessionId } })).structuredContent.rows;
  expect(rows).toHaveLength(2);

  await button.focus();
  await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__host.messages.length)).toBe(1);
  const message = await page.evaluate(() => window.__host.messages[0].content[0].text);
  expect(message).toBe("Analyse the 2 selected rows of orders.");
  const context = await page.evaluate(() => window.__host.contexts.at(-1).content[0].text);
  expect(context).toContain(sessionId);
});

test("the view passes axe", async ({ page }) => {
  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations).toEqual([]);
});

test("the host's theme and language reach the grid", async ({ page }) => {
  await page.evaluate(() => window.__bridge.sendHostContextChange({ theme: "dark", locale: "de-DE" }));
  await expect(view(page).locator("opengrid-grid")).toHaveAttribute("theme", "dark");
  await expect(view(page).locator("html")).toHaveAttribute("lang", "de");
  // The view's own words follow; the grid's are the page's to set.
  await view(page).locator('opengrid-grid td[data-row="0"][data-col="0"]').focus();
  await page.keyboard.press(" ");
  await expect(view(page).getByRole("button", { name: /Auswahl analysieren/ })).toHaveText("Auswahl analysieren (1)");
});
