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
 *   stats { type:"stats", id }                 -> { id, result }   (issue #70)
 *   pivot { type:"pivot", id, pivot }          -> { id, result }   (issue #28)
 *
 * Any request that throws answers `{ id, type:"error", message }`. The `bytes`
 * of `load` arrive as a transferable `ArrayBuffer`; a `result` is the engine's
 * binary result form (E35, `execute_columns`) and travels back the same way —
 * transferred, not copied. Nothing engine-specific is serialised here.
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
      case "stats": {
        // What the engine holds: its memory and each source's size, as JSON.
        self.postMessage({ id: data.id, result: engine.stats() });
        break;
      }
      case "query": {
        const result = engine.execute_columns(data.query);
        self.postMessage({ id: data.id, result }, [result.buffer]);
        break;
      }
      case "pivot": {
        const result = engine.pivot_columns(data.pivot);
        self.postMessage({ id: data.id, result }, [result.buffer]);
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
