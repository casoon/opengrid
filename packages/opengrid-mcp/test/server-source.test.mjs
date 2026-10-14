// A source of an opengrid-server that cannot be reached when the MCP server
// starts (#140): the other sources serve, and it says so until its server
// answers. The server here is a stand-in: the real one is in the e2e spec
// (tests/e2e/mcp-server-source.spec.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { fileURLToPath } from "node:url";
import { openClient } from "./serve.mjs";

const repo = (path) => fileURLToPath(new URL(`../../../${path}`, import.meta.url));
const SCHEMA = { fields: [{ name: "id", type: "int64", nullable: false }] };

/** A stand-in opengrid-server: its source, and a count of 7 for any query. */
function standIn() {
  return createHttpServer((request, response) => {
    response.setHeader("Content-Type", "application/json");
    if (request.method === "GET" && request.url === "/source/remote") {
      response.end(JSON.stringify({ schema: SCHEMA }));
    } else if (request.method === "POST" && request.url === "/query/remote") {
      request.resume();
      request.on("end", () => response.end(JSON.stringify({ columns: [], row_count: 0, total_count: 7 })));
    } else {
      response.statusCode = 404;
      response.end("{}");
    }
  });
}

/** A port nothing listens on — yet. */
async function freePort() {
  const probe = createHttpServer();
  await new Promise((done) => probe.listen(0, "127.0.0.1", done));
  const { port } = probe.address();
  await new Promise((done) => probe.close(done));
  return port;
}

test("a server source that is down keeps the server from nothing, and serves once it is up", async (t) => {
  const port = await freePort();
  const client = await openClient({
    sources: {
      orders: {
        csv: repo("tests/e2e/fixtures/grid-virtual.csv"),
        schema: repo("crates/opengrid-conformance/data/orders.schema.json"),
      },
      remote: { server: { url: `http://127.0.0.1:${port}` } },
    },
  });
  const call = async (name, args) => client.callTool({ name, arguments: args });

  const { tools } = await client.listTools();
  assert.match(tools.find((tool) => tool.name === "opengrid_open").description, /orders, remote/);
  assert.equal((await call("opengrid_open", { source: "orders" })).structuredContent.total, 200);

  const down = await call("opengrid_open", { source: "remote" });
  assert.equal(down.isError, true);
  const { error } = JSON.parse(down.content[0].text);
  assert.equal(error.code, "unavailable");
  assert.equal(error.path, "source");

  const server = standIn();
  await new Promise((done) => server.listen(port, "127.0.0.1", done));
  t.after(() => server.close());
  const up = (await call("opengrid_open", { source: "remote" })).structuredContent;
  assert.equal(up.total, 7);
  assert.deepEqual(up.columns.map((column) => column.name), ["id"]);
});
