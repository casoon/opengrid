// A real MCP client on an opengrid-mcp server, in memory (#140): what the
// tool tests and the view's e2e test talk to.
import { Client, InMemoryTransport } from "@modelcontextprotocol/client";

import { createServer } from "../src/server.mjs";

export async function openClient(config) {
  const [clientSide, serverSide] = InMemoryTransport.createLinkedPair();
  await createServer(config)().connect(serverSide);
  const client = new Client({ name: "opengrid tests", version: "0" });
  await client.connect(clientSide);
  return client;
}
