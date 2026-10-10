import { test, expect } from "@playwright/test";

// A tree exported flat (issue #166, rule T8), on the conformance org chart:
// every node, depth-first, siblings by name as the grid sorts them — not only
// the open ones — with its level and its path of keys.

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/grid-tree.html");
  await page.waitForFunction(() => window.__opengridReady && window.__opengridModule);
  await page.waitForFunction(
    () => !!document.querySelector("opengrid-grid").shadowRoot?.querySelector("td[data-row]"),
  );
});

/**
 * `get_query()` of the grid, exported through `exportRows` in pieces of 3 —
 * through a provider that answers JSON, or one that answers in the binary
 * form, as a worker does.
 */
async function exported(page, options, binary = false) {
  return page.evaluate(
    async ({ options, binary }) => {
      const { exportRows } = await import("/packages/opengrid/loader.js");
      const query = window.__opengridModule.get_query(document.querySelector("opengrid-grid"));
      const provider = binary ? window.__binaryProvider : window.__provider;
      const blob = await exportRows(provider, query, { chunkSize: 3, ...options });
      return { query, text: await blob.text() };
    },
    { options, binary },
  );
}

test("get_query asks for the whole tree, flat", async ({ page }) => {
  const { query } = await exported(page, { format: "json" });
  expect(query.tree).toEqual({ key: "id", parent: "parent_id", flat: true });
});

const CSV = [
  "name,region,revenue,level,path",
  "Orphan,US,7,1,9",
  "Partners,US,10,1,7",
  "Dave,US,5,2,7 / 8",
  "Sales,EU,0,1,1",
  "North,EU,100,2,1 / 2",
  "Alice,EU,30,3,1 / 2 / 4",
  "Eve,EU,3,4,1 / 2 / 4 / 10",
  "Bob,EU,,3,1 / 2 / 5",
  "South,EU,50,2,1 / 3",
  "Carol,EU,40,3,1 / 3 / 6",
  "",
].join("\r\n");

test("a worker's binary answers export the tree the same way", async ({ page }) => {
  const { text } = await exported(page, { format: "csv", bom: false }, true);
  expect(text).toBe(CSV);
});

test("a CSV holds every node depth-first, with its level and its path", async ({ page }) => {
  const { text } = await exported(page, { format: "csv", bom: false });
  expect(text).toBe(
    [
      "name,region,revenue,level,path",
      "Orphan,US,7,1,9",
      "Partners,US,10,1,7",
      "Dave,US,5,2,7 / 8",
      "Sales,EU,0,1,1",
      "North,EU,100,2,1 / 2",
      "Alice,EU,30,3,1 / 2 / 4",
      "Eve,EU,3,4,1 / 2 / 4 / 10",
      "Bob,EU,,3,1 / 2 / 5",
      "South,EU,50,2,1 / 3",
      "Carol,EU,40,3,1 / 3 / 6",
      "",
    ].join("\r\n"),
  );
});

test("a JSON export writes the path as an array, and a filter marks the context", async ({
  page,
}) => {
  await page.evaluate(() =>
    window.__opengridModule.set_view(document.querySelector("opengrid-grid"), {
      filters: [{ column: "name", op: "eq", value: "Eve" }],
    }),
  );
  await page.waitForFunction(() =>
    window.__opengridModule.get_query(document.querySelector("opengrid-grid"))?.filter,
  );
  const { text } = await exported(page, { format: "json" });
  expect(JSON.parse(text)).toEqual([
    { name: "Sales", region: "EU", revenue: 0, level: 1, path: [1], match: false },
    { name: "North", region: "EU", revenue: 100, level: 2, path: [1, 2], match: false },
    { name: "Alice", region: "EU", revenue: 30, level: 3, path: [1, 2, 4], match: false },
    { name: "Eve", region: "EU", revenue: 3, level: 4, path: [1, 2, 4, 10], match: true },
  ]);
});
