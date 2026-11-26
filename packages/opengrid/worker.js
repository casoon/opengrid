/**
 * opengrid engine worker: the local engine in a module worker.
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
 *   rows  { type:"rows", id, name, rows, schema }  -> { type:"loaded", id }  (issue #182)
 *   columns { type:"columns", id, name, schema, rows, pack } -> { type:"loaded", id }  (issue #445)
 *   columns_begin / columns_chunk / columns_end / columns_abort: the same rows in packs, as
 *   load_begin / load_chunk / load_end / load_abort are for CSV
 *   query { type:"query", id, query }          -> { id, result }
 *   stats { type:"stats", id }                 -> { id, result }   (issue #70)
 *   describe { type:"describe", id, name }     -> { id, result }   (issue #528): the source's schema as JSON
 *   unload { type:"unload", id, name }         -> { id, result }   (issue #459): whether there was one
 *   pivot { type:"pivot", id, pivot }          -> { id, result }   (issue #28)
 *
 * Any request that throws answers `{ id, type:"error", message }`. The `bytes`
 * of `load` arrive as a transferable `ArrayBuffer`; a `result` is the engine's
 * binary result form (E35, `execute_columns`) and travels back the same way —
 * transferred, not copied. Nothing engine-specific is serialised here.
 */

// The packaged engine glue, imported statically so that a bundler follows it and
// the `.wasm` beside it; `init` names another one with `moduleUrl`.
import * as packaged from "./engine/opengrid_wasm.js";

// The collator, for a query that sorts by a locale (rule S4a): `collation.js` and the module
// behind it are fetched by the first such query, and never otherwise — so a page that serves
// the package's files itself and leaves them out loses locale sorts, not the worker. The same
// test is in `loader.js`; the two must agree.
const NAMES_A_LOCALE = /"collation"\s*:\s*"(?!binary")/;
let collatorGiven = false;

async function withCollator(engine, json, run) {
  if (!collatorGiven && NAMES_A_LOCALE.test(json)) {
    let collation;
    try {
      collation = await import("./collation.js");
    } catch (error) {
      throw Object.assign(
        new Error(`a sort by a locale needs collation.js, and it did not load (${error?.message ?? error})`),
        { code: "module_not_loaded" },
      );
    }
    await collation.giveCollator(engine);
    collatorGiven = true;
  }
  return run();
}

/** The single engine instance, created on `init` and reused for every query. */
let engine;

self.onmessage = async ({ data }) => {
  try {
    switch (data.type) {
      case "init": {
        // wasm-bindgen `--target web`: the default export initialises the
        // module; the named `Engine` class is created afterwards.
        const module = data.moduleUrl ? await import(/* @vite-ignore */ data.moduleUrl) : packaged;
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
      case "load_begin": {
        engine.begin_csv(data.name, data.schema, data.size);
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "load_chunk": {
        engine.push_csv(new Uint8Array(data.bytes));
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "load_end": {
        engine.end_csv();
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "load_abort": {
        engine.abort_csv();
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "columns": {
        // The rows of a page as one pack of columns (issue #445); a pack that does not hold, or an
        // engine module without the calls, answers with an error, and the page takes the JSON route.
        engine.begin_columns(data.name, data.schema, data.rows);
        engine.push_columns(new Uint8Array(data.pack));
        engine.end_columns();
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "columns_begin": {
        engine.begin_columns(data.name, data.schema, data.rows);
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "columns_chunk": {
        engine.push_columns(new Uint8Array(data.pack));
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "columns_end": {
        engine.end_columns();
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "columns_abort": {
        engine.abort_columns();
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "rows": {
        engine.load_rows(data.name, data.rows, data.schema);
        self.postMessage({ type: "loaded", id: data.id });
        break;
      }
      case "unload": {
        self.postMessage({ id: data.id, result: engine.unload(data.name) });
        break;
      }
      case "describe": {
        self.postMessage({ id: data.id, result: engine.describe(data.name) });
        break;
      }
      case "stats": {
        // What the engine holds: its memory and each source's size, as JSON.
        self.postMessage({ id: data.id, result: engine.stats() });
        break;
      }
      case "query": {
        const result = await withCollator(engine, data.query, () =>
          engine.execute_columns(data.query),
        );
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
