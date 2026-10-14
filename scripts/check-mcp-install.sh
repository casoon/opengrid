#!/usr/bin/env bash
# Installs the **packed** `@casoon/opengrid-mcp` the way `npx` would (#140) and
# talks to it over stdio, as a host does: it starts on a configuration, lists
# its tools, opens a source and serves the grid's view. No host renders the
# view here — that stays a check by hand.
#
# The `@casoon/opengrid` it depends on comes from target/npm-package when that
# tarball has the version asked for (a release of both), else from npm.
#
# Usage: bash scripts/check-mcp-install.sh   (after scripts/pack-mcp.sh)
set -euo pipefail

cd "$(dirname "$0")/.."
root="$PWD"
dest="$root/target/npm-package"
work="$root/target/mcp-install"
tarball="$(ls "$dest"/casoon-opengrid-mcp-*.tgz 2>/dev/null | head -1)"

[[ -n "$tarball" ]] || {
    echo "no packed tarball in target/npm-package — run scripts/pack-mcp.sh first" >&2
    exit 1
}

wanted="$(node -p 'JSON.parse(process.argv[1]).dependencies["@casoon/opengrid"]' "$(tar -xzOf "$tarball" package/package.json)")"
install=("$tarball")
if [[ -f "$dest/casoon-opengrid-$wanted.tgz" ]]; then
    install+=("$dest/casoon-opengrid-$wanted.tgz")
fi

rm -rf "$work"
mkdir -p "$work"
echo '{ "name": "mcp-install-check", "private": true, "type": "module" }' > "$work/package.json"
(cd "$work" && npm install "${install[@]}" --no-audit --no-fund >/dev/null)

cat > "$work/opengrid-mcp.json" <<JSON
{ "sources": { "orders": {
    "csv": "$root/crates/opengrid-conformance/data/orders.csv",
    "schema": "$root/crates/opengrid-conformance/data/orders.schema.json",
    "columns": ["id", "customer", "country", "amount"] } } }
JSON

cat > "$work/check.mjs" <<'JS'
import { Client } from "@modelcontextprotocol/client";
import { StdioClientTransport } from "@modelcontextprotocol/client/stdio";

const transport = new StdioClientTransport({
  command: "node_modules/.bin/opengrid-mcp",
  args: ["--config", "opengrid-mcp.json"],
  stderr: "inherit",
});
const client = new Client({ name: "mcp-install-check", version: "0" });
await client.connect(transport);
const fail = (message) => {
  throw new Error(`mcp install: ${message}`);
};

const { tools } = await client.listTools();
if (tools.length !== 6) fail(`${tools.length} tools, not 6`);
const opened = await client.callTool({ name: "opengrid_open", arguments: { source: "orders" } });
if (opened.isError || !(opened.structuredContent.total > 0)) fail(`opengrid_open: ${JSON.stringify(opened)}`);
const [view] = (await client.readResource({ uri: "ui://opengrid/grid" })).contents;
if (!view.text.includes("<script")) fail("the grid's view has no script");
const version = client.getServerVersion().version;
console.log(`mcp install: ok — ${version}, ${tools.length} tools, ${opened.structuredContent.total} rows, view ${view.text.length} bytes`);
await client.close();
JS
(cd "$work" && node check.mjs)
