#!/usr/bin/env node
// @casoon/opengrid-mcp over stdio (#140): Claude Desktop starts it with
//   npx -y @casoon/opengrid-mcp --config /path/to/opengrid-mcp.json
import { readFileSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { McpServer } from "@modelcontextprotocol/server";
import { serveStdio } from "@modelcontextprotocol/server/stdio";

import { loadConfig } from "./config.mjs";
import { Data } from "./data.mjs";
import { Sessions } from "./sessions.mjs";
import { registerTools } from "./tools.mjs";

const VERSION = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")).version;

/** A server over the configured sources — the factory the transports use. */
export function createServer(config) {
  const data = new Data(config);
  const sessions = new Sessions();
  return () => {
    const server = new McpServer({ name: "opengrid", version: VERSION }, { capabilities: { tools: {}, resources: {} } });
    registerTools(server, data, sessions);
    return server;
  };
}

function configPath(args) {
  const at = args.indexOf("--config");
  if (at >= 0 && args[at + 1]) return args[at + 1];
  return process.env.OPENGRID_MCP_CONFIG ?? "opengrid-mcp.json";
}

// Run as the bin (also through npx's symlink), not when imported by a test.
const main = process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url);
if (main) {
  const factory = createServer(loadConfig(configPath(process.argv.slice(2))));
  serveStdio(factory);
}
