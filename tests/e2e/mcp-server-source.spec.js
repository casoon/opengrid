import { test, expect } from "@playwright/test";
import { join } from "node:path";

// A source of an opengrid-server in @casoon/opengrid-mcp (#150), against the
// real e2e server (playwright.config.js starts it): the same tools answer the
// same as over the same CSV in the MCP server itself — and the server's rules
// hold (its token, its allowed fields).

const repo = process.cwd();
const { openClient } = await import(join(repo, "packages/opengrid-mcp/test/serve.mjs"));
const COLUMNS = ["id", "customer", "country", "amount", "qty"];
const ON_SERVER = { server: { url: "http://127.0.0.1:8082", token: "e2e-token" }, columns: COLUMNS };
const IN_FILE = {
  csv: join(repo, "crates/opengrid-conformance/data/orders.csv"),
  schema: join(repo, "crates/opengrid-conformance/data/orders.schema.json"),
  columns: COLUMNS,
};

async function both() {
  const [server, file] = await Promise.all([
    openClient({ sources: { orders: ON_SERVER } }),
    openClient({ sources: { orders: IN_FILE } }),
  ]);
  const call = (client, name, args) => client.callTool({ name, arguments: args });
  return { server, file, call };
}

test("open and set_view count on the server what the file counts", async () => {
  const { server, file, call } = await both();
  const opened = [];
  for (const client of [server, file]) {
    opened.push((await call(client, "opengrid_open", { source: "orders" })).structuredContent);
  }
  expect(opened[0].total).toBe(opened[1].total);
  expect(opened[0].columns.map((column) => column.name)).toEqual(COLUMNS);

  const view = { filters: [{ column: "country", op: "eq", value: "DE" }], sort: [{ field: "amount", direction: "desc" }] };
  const counts = [];
  for (const [client, { sessionId }] of [[server, opened[0]], [file, opened[1]]]) {
    counts.push((await call(client, "opengrid_set_view", { sessionId, view })).structuredContent.total);
  }
  expect(counts[0]).toBe(counts[1]);
  expect(counts[0]).toBeGreaterThan(0);
});

test("the selection's rows come from the server, as the file's", async () => {
  const { server, file, call } = await both();
  const rows = [];
  for (const client of [server, file]) {
    const { sessionId } = (await call(client, "opengrid_open", { source: "orders" })).structuredContent;
    const query = { source: "orders", select: ["id", "country", "amount"], sort: [{ field: "id", direction: "desc" }] };
    await call(client, "opengrid_sync", { sessionId, revision: 1, change: { query, selected: [0, 2, 3] } });
    rows.push((await call(client, "opengrid_selection", { sessionId })).structuredContent.rows);
  }
  expect(rows[0]).toEqual(rows[1]);
  expect(rows[0]).toHaveLength(3);
});

test("the server's allowed fields hold: note is not one of them", async () => {
  const { server, call } = await both();
  const { sessionId } = (await call(server, "opengrid_open", { source: "orders" })).structuredContent;
  const refused = await call(server, "opengrid_query", {
    sessionId,
    query: { source: "orders", select: ["note"] },
  });
  expect(refused.isError).toBe(true);
  expect(JSON.parse(refused.content[0].text).error.code).toBe("validation");
});

test("a token the server refuses stops the start, with a sentence", async () => {
  await expect(
    openClient({ sources: { orders: { server: { url: "http://127.0.0.1:8082", token: "wrong" } } } }),
  ).rejects.toThrow(/opengrid-mcp: source orders: http:\/\/127\.0\.0\.1:8082 refused/);
  await expect(
    openClient({ sources: { orders: { server: { url: "http://127.0.0.1:9", token: "x" } } } }),
  ).rejects.toThrow(/cannot be reached/);
});
