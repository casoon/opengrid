// What the server refuses to hold or hand out (#242): sessions past a cap, a
// query past a window, a view that is not a view.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { openClient } from "./serve.mjs";
import { Sessions } from "../src/sessions.mjs";
import { QUERY_LIMIT_MAX } from "../src/tools.mjs";

test("sessions past the cap go, the least recently used first", () => {
  const sessions = new Sessions(2);
  const open = () => sessions.open("orders", {}, {}, 0);
  const first = open();
  const second = open();
  // Reading the first makes the second the oldest.
  assert.ok(sessions.get(first.id));
  const third = open();
  assert.equal(sessions.get(second.id), undefined);
  assert.ok(sessions.get(first.id));
  assert.ok(sessions.get(third.id));
});

test("opengrid_query hands out a window at most, whatever limit it is asked for", async (t) => {
  const bodies = [];
  const server = createHttpServer((request, response) => {
    response.setHeader("Content-Type", "application/json");
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      if (request.method === "GET") {
        response.end(JSON.stringify({ schema: { fields: [{ name: "id", type: "int64", nullable: false }] } }));
      } else {
        bodies.push(JSON.parse(body));
        response.end(JSON.stringify({ columns: [], row_count: 0, total_count: 7 }));
      }
    });
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  t.after(() => server.close());
  const client = await openClient({
    sources: { remote: { server: { url: `http://127.0.0.1:${server.address().port}` } } },
  });
  const call = async (name, args) => client.callTool({ name, arguments: args });
  const { sessionId } = (await call("opengrid_open", { source: "remote" })).structuredContent;

  for (const limit of [10_000, undefined]) {
    bodies.length = 0;
    await call("opengrid_query", { sessionId, query: { source: "remote", select: ["id"], limit } });
    assert.equal(bodies.at(-1).limit, QUERY_LIMIT_MAX);
  }
  bodies.length = 0;
  await call("opengrid_query", { sessionId, query: { source: "remote", select: ["id"], limit: 40 } });
  assert.equal(bodies.at(-1).limit, 40, "a window under the cap is left alone");
});

test("a view has a shape: an operator or a direction outside the lists is refused", async () => {
  const client = await openClient({
    sources: {
      orders: {
        csv: new URL("../../../tests/e2e/fixtures/grid-virtual.csv", import.meta.url).pathname,
        schema: new URL("../../../crates/opengrid-conformance/data/orders.schema.json", import.meta.url).pathname,
      },
    },
  });
  const call = async (name, args) => client.callTool({ name, arguments: args });
  const { sessionId } = (await call("opengrid_open", { source: "orders" })).structuredContent;
  const refused = await call("opengrid_set_view", {
    sessionId,
    view: { filters: [{ column: "country", op: "equals", value: "DE" }] },
  });
  assert.equal(refused.isError, true);
  assert.match(refused.content[0].text, /op/);
  const sideways = await call("opengrid_set_view", { sessionId, view: { sort: [{ field: "id", direction: "up" }] } });
  assert.equal(sideways.isError, true);
});
