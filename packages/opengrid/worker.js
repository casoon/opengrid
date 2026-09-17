/**
 * opengrid engine worker (plan point 19,
 * plan/spezifikation/04-local-engine.md §Worker).
 *
 * A **module worker** that owns exactly one WASM `Engine`. The main thread
 * keeps the DOM, events and rendering; this worker runs ingest, filter, sort,
 * group and aggregate, so a large query never blocks the UI. No pool, no
 * SharedArrayBuffer (V1, §Multithreading erst später).
 *
 * Message protocol (all replies carry the request `id` so they can be matched):
 *
 *   init  { type:"init", id, moduleUrl, wasmUrl } -> { type:"ready", id }
 *   load  { type:"load", id, name, bytes, schema }
 *                                              -> { type:"loaded", id }
 *   query { type:"query", id, query }          -> { id, result }
 *
 * Any request that throws answers `{ id, type:"error", message }`. The `bytes`
 * of `load` arrive as a transferable `ArrayBuffer`; the response strings are the
 * wire JSON of the engine (E6/E13), so nothing engine-specific is serialised
 * here.
 */

/** The single engine instance, created on `init` and reused for every query. */
let engine;

self.onmessage = async ({ data }) => {
  try {
    switch (data.type) {
      case "init": {
        // wasm-bindgen `--target web`: the default export initialises the
        // module; the named `Engine` class is created afterwards.
        const module = await import(data.moduleUrl);
        await module.default(data.wasmUrl);
        engine = new module.Engine();
        self.postMessage({ type: "ready", id: data.id });
        break;
      }
      case "load": {
        engine.load_csv(data.name, new Uint8Array(data.bytes), data.schema);
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "query": {
        self.postMessage({ id: data.id, result: engine.execute(data.query) });
        break;
      }
      default:
        throw new Error(`unknown message type ${data.type}`);
    }
  } catch (error) {
    // The error path is a message, not a crash: the provider rejects the
    // matching promise and the page decides what to show.
    self.postMessage({
      id: data.id,
      type: "error",
      message: error?.message ?? String(error),
    });
  }
};
