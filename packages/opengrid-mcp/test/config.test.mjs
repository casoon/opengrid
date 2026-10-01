// The operator's configuration (#140, #150): what it accepts and what it says.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { loadConfig } from "../src/config.mjs";

function write(config) {
  const dir = mkdtempSync(join(tmpdir(), "opengrid-mcp-"));
  const path = join(dir, "opengrid-mcp.json");
  writeFileSync(path, JSON.stringify(config));
  return { dir, path };
}

test("a file source's paths are relative to the configuration", () => {
  const { dir, path } = write({ sources: { orders: { csv: "data/o.csv", schema: "data/o.schema.json" } } });
  const { sources } = loadConfig(path);
  assert.equal(sources.orders.csv, join(dir, "data/o.csv"));
});

test("a server source names a URL, a source and where its token is", () => {
  const { path } = write({
    sources: { remote: { server: { url: "https://grid.example/api", source: "orders", tokenEnv: "ORDERS_TOKEN" } } },
  });
  assert.deepEqual(loadConfig(path).sources.remote.server, {
    url: "https://grid.example/api",
    source: "orders",
    tokenEnv: "ORDERS_TOKEN",
  });
});

test("what a configuration may not say is named", () => {
  const refused = (config, pattern) => {
    const { path } = write(config);
    assert.throws(() => loadConfig(path), pattern);
  };
  // The server narrows; a second allow-list here would only disagree with it.
  refused({ sources: { r: { server: { url: "https://x.example" }, fields: ["id"] } } }, /sources\.r/);
  refused({ sources: { r: { server: { url: "https://x.example", token: "a", tokenEnv: "B" } } } }, /token or tokenEnv/);
  refused({ sources: { r: { server: { url: "not a url" } } } }, /sources\.r/);
  refused({ sources: {} }, /at least one source/);
});
