// Every tool of @casoon/opengrid-mcp through a real MCP client (#140, point 18):
// valid, invalid, and at its limits. The data are the conformance orders —
// 200 rows, the same the e2e fixtures use.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { openClient } from "./serve.mjs";

const repo = (path) => fileURLToPath(new URL(`../../../${path}`, import.meta.url));
const CONFIG = {
  sources: {
    orders: {
      csv: repo("tests/e2e/fixtures/grid-virtual.csv"),
      schema: repo("crates/opengrid-conformance/data/orders.schema.json"),
      columns: ["id", "customer", "country", "amount", "qty"],
      // `note` and the rest exist, but no query may name them.
      fields: ["id", "customer", "country", "amount", "qty", "ordered_on"],
    },
  },
};

async function connect() {
  const client = await openClient(CONFIG);
  const call = async (name, args) => client.callTool({ name, arguments: args });
  return { client, call };
}

const errorOf = (result) => {
  assert.equal(result.isError, true, JSON.stringify(result));
  return JSON.parse(result.content[0].text).error;
};

test("the tools, and who may call them", async () => {
  const { client } = await connect();
  const { tools } = await client.listTools();
  const visibility = Object.fromEntries(tools.map((tool) => [tool.name, tool._meta?.ui?.visibility ?? ["model", "app"]]));
  assert.deepEqual(Object.keys(visibility).sort(), [
    "opengrid_describe",
    "opengrid_open",
    "opengrid_query",
    "opengrid_selection",
    "opengrid_set_view",
    "opengrid_sync",
  ]);
  // The rows travel only to the grid.
  assert.deepEqual(visibility.opengrid_query, ["app"]);
  assert.deepEqual(visibility.opengrid_sync, ["app"]);
  assert.equal(tools.find((tool) => tool.name === "opengrid_open")._meta.ui.resourceUri, "ui://opengrid/grid");
});

test("opening a source answers with columns and a count, never rows", async () => {
  const { call } = await connect();
  const opened = (await call("opengrid_open", { source: "orders" })).structuredContent;
  assert.equal(opened.total, 200);
  assert.deepEqual(
    opened.columns.map((column) => column.name),
    ["id", "customer", "country", "amount", "qty"],
  );
  assert.equal(opened.rows, undefined);
  assert.match(opened.sessionId, /^[0-9a-f-]{36}$/);

  assert.equal(errorOf(await call("opengrid_open", { source: "nope" })).code, "unknown_source");
});

test("set_view answers with the new count; a view with an unknown column is refused whole", async () => {
  const { call } = await connect();
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const view = { filters: [{ column: "country", op: "eq", value: "DE" }], sort: [{ field: "amount", direction: "desc" }] };
  const set = (await call("opengrid_set_view", { sessionId, view })).structuredContent;
  assert.equal(set.total, 52, "the 52 orders from DE");

  // `note` exists in the source but is not a column of the grid.
  const refused = errorOf(
    await call("opengrid_set_view", { sessionId, view: { sort: [{ field: "note", direction: "asc" }] } }),
  );
  assert.equal(refused.code, "validation");
  assert.match(refused.message, /note/);
  const after = (await call("opengrid_describe", { sessionId })).structuredContent;
  assert.deepEqual(after.view, view, "the refused view changed nothing");
  assert.equal(after.total, 52);

  const typed = errorOf(
    await call("opengrid_set_view", { sessionId, view: { filters: [{ column: "qty", op: "gte", value: "many" }] } }),
  );
  assert.match(typed.message, /qty.*many/);
});

test("describe says what the reader sees, without rows", async () => {
  const { call } = await connect();
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const described = (await call("opengrid_describe", { sessionId })).structuredContent;
  assert.equal(described.total, 200);
  assert.equal(described.selected, 0);
  assert.deepEqual(described.query.select, ["id", "customer", "country", "amount", "qty"]);
  assert.equal(errorOf(await call("opengrid_describe", { sessionId: "nope" })).code, "unknown_session");
});

test("query runs the grid's queries, inside the session's source and allowed fields", async () => {
  const { call } = await connect();
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const result = JSON.parse(
    (await call("opengrid_query", { sessionId, query: { source: "orders", select: ["id"], limit: 3 } })).content[0].text,
  );
  assert.equal(result.row_count, 3);
  assert.equal(result.total_count, 200);

  const hidden = errorOf(
    await call("opengrid_query", { sessionId, query: { source: "orders", select: ["note"] } }),
  );
  assert.equal(hidden.code, "validation", "a field outside the allow-list is unknown");
  const other = errorOf(await call("opengrid_query", { sessionId, query: { source: "customers", select: ["id"] } }));
  assert.equal(other.path, "query.source");
});

test("sync takes the reader's change and hands out the model's view", async () => {
  const { call } = await connect();
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const first = (await call("opengrid_sync", { sessionId, revision: 1 })).structuredContent;
  assert.equal(first.revision, 1);

  const reader = { sort: [{ field: "qty", direction: "asc" }] };
  const query = { source: "orders", select: ["id", "qty"], sort: [{ field: "qty", direction: "asc" }] };
  const changed = (
        await call("opengrid_sync", { sessionId, revision: 1, change: { view: reader, query, selected: [0, 1, 2] } })
  ).structuredContent;
  assert.equal(changed.revision, 2);
  assert.equal(changed.origin, "reader");

  await call("opengrid_set_view", { sessionId, view: { group: ["country"] } });
  const model = (await call("opengrid_sync", { sessionId, revision: 2 })).structuredContent;
  assert.equal(model.revision, 3);
  assert.equal(model.origin, "model");
  assert.deepEqual(model.view, { group: ["country"] });
  // A new view drops the selection, as in the grid.
  assert.equal((await call("opengrid_describe", { sessionId })).structuredContent.selected, 0);
});

test("selection reads the selected positions under the grid's query, cut at the limit", async () => {
  const { call } = await connect();
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const query = { source: "orders", select: ["id", "qty"], sort: [{ field: "id", direction: "desc" }] };
    await call("opengrid_sync", { sessionId, revision: 1, change: { query, selected: [0, 1, 7] } });

  const all = (await call("opengrid_selection", { sessionId })).structuredContent;
  assert.deepEqual(
    all.rows.map((row) => row.id),
    [200, 199, 193],
    "positions under the query's sort, as objects",
  );
  assert.deepEqual(Object.keys(all.rows[0]), ["id", "qty"]);
  assert.equal(all.truncated, false);

  const cut = (await call("opengrid_selection", { sessionId, limit: 2 })).structuredContent;
  assert.equal(cut.rows.length, 2);
  assert.equal(cut.truncated, true);
  assert.equal(cut.selected, 3);

  const tooMany = await call("opengrid_selection", { sessionId, limit: 501 });
  assert.equal(tooMany.isError, true, "500 at most");
});

test("the count of the reader's query is the server's", async () => {
  const { call } = await connect();
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const query = {
    source: "orders",
    select: ["id", "country"],
    filter: { field: "country", op: "eq", value: "DE" },
    sort: [{ field: "id", direction: "asc" }],
  };
  const synced = (await call("opengrid_sync", { sessionId, revision: 1, change: { query } })).structuredContent;
  assert.equal(synced.total, 52);
  // A query outside the allow-list is not taken.
  const refused = await call("opengrid_sync", {
    sessionId,
    revision: 2,
    change: { query: { source: "orders", select: ["note"] } },
  });
  assert.equal(refused.isError, true);
});
