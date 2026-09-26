// The engine the package ships under `engine/`, imported the way docs/api.md
// shows it. Compiled only against the packed package (scripts/typecheck-package.sh):
// the module is build output, and the subpath resolves only through the
// package's `exports` — a missing `"./engine/*"` entry is an error here.

import init, { Engine, Planner } from "@casoon/opengrid/engine/opengrid_wasm.js";
import {
  createHybridProvider,
  createLocalProvider,
  createRestProvider,
} from "@casoon/opengrid";

export async function tab(): Promise<void> {
  await init();
  // The generated classes fit what the providers take.
  const local = createLocalProvider(new Engine());
  await local.load("orders", new Uint8Array(), "{}");
  const remote = createRestProvider({ url: "https://example.test", source: "orders" });
  const described = await remote.describe();
  const planner = new Planner(
    JSON.stringify(described.schema),
    JSON.stringify(described.capabilities),
    "auto",
  );
  createHybridProvider({ remote, planner });
}
