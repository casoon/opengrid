/**
 * @casoon/opengrid loader: the small amount of JavaScript the elements need.
 *
 * The browser loads exactly this file plus the WASM module. `loader.js` does
 * three small things and no domain logic:
 *
 *   1. load the WASM module (`wasm-bindgen --target web` glue),
 *   2. register the custom elements (the Rust side does the rendering),
 *   3. offer a DOM fallback when WASM is unavailable, so a page never breaks
 *      because of a missing `.wasm`.
 *
 * The fallback is intentionally the same empty skeleton the Rust element
 * renders; the Rust element replaces it once the table carries data.
 *
 * Point 19 adds the two providers. Both expose the same tiny shape the elements
 * consume (`load`, `execute`, `terminate`), so a page switches between the
 * Worker and the main thread without touching the grid:
 *
 *   createWorkerProvider()                        engine in a module worker
 *   createLocalProvider(engine)                   engine on the main thread
 *   createRestProvider({ url, source, token })    opengrid-server over HTTP
 *
 * A provider's `execute` answers the result JSON **or** the binary result form
 * (decision E35, a `Uint8Array`): the elements and `exportRows` read both. The
 * tab and worker providers answer bytes — the worker hands them over without a
 * copy — and the REST providers ask the server for them and fall back to JSON
 * when it answers JSON.
 *
 * Every provider's `execute(queryJson, mode, { signal })` may take an
 * `AbortSignal` as its third argument. The providers that talk HTTP
 * hand it to `fetch`, so an aborted export leaves no request running; the tab
 * and the worker cannot stop a query that has started and ignore it — the
 * caller drops their answer.
 *
 * The Worker provider starts lazily on the first `load`/`execute` and stays the
 * single worker of V1 (no pool, no SharedArrayBuffer).
 */

// The packaged glue (its path is fixed by the package), imported statically: a bundler
// follows the import, and the `new URL("…_bg.wasm", import.meta.url)` inside the
// glue, so the `.wasm` ends up in its output. `moduleUrl` replaces it.
import * as packaged from "./pkg/opengrid_web_components.js";

/** Set once the first `loadOpengrid` call has run. */
let loading;

/**
 * Loads the WASM module and registers the elements.
 *
 * @param {object} [options]
 * @param {URL|string} [options.moduleUrl] wasm-bindgen glue module (`--target web`).
 * @param {string} [options.wasmUrl] explicit `.wasm` URL, if it is not next to the glue.
 * @returns {Promise<{fallback: boolean, module?: object}>} whether the DOM fallback was
 *   installed, and the loaded WASM module so a page can call its exports —
 *   `set_provider` and `set_texts` (call it first: it rebuilds the component).
 */
export function loadOpengrid(options = {}) {
  if (!loading) {
    loading = start(options);
  }
  return loading;
}

async function start({ moduleUrl, wasmUrl } = {}) {
  try {
    const module = moduleUrl ? await import(/* @vite-ignore */ moduleUrl) : packaged;
    // wasm-bindgen `--target web` exports the initialiser as the default.
    await module.default(wasmUrl);
    module.register();
    return { fallback: false, module };
  } catch (error) {
    console.warn("[opengrid] WASM unavailable, falling back to the DOM renderer", error);
    installFallback();
    return { fallback: true };
  }
}

/**
 * Defines the elements in plain DOM, mirroring the Rust skeleton: an open
 * shadow root, a native `<table>` with a `<caption>`, and `label` -> `aria-label`.
 *
 * A page that wrote its data as a table inside the element (issue #84) keeps
 * showing it when the module cannot load, which is when it is needed most: the
 * stand-in then renders only a `<slot>`. Without such content it renders the
 * skeleton, as before. Never both — as the slot's default content the skeleton
 * still counted as a second table in WebKit's accessibility tree.
 *
 * Not exported: `loadOpengrid` installs it when the module fails to load, and a
 * page that reaches for it directly is asking for the broken state on purpose
 * (an export is a promise).
 *
 * @param {string} [name] the element to define.
 */
function installFallback(name = "opengrid-table") {
  if (customElements.get(name)) {
    return;
  }

  class OpengridTableFallback extends HTMLElement {
    static get observedAttributes() {
      return ["label"];
    }

    connectedCallback() {
      if (this.shadowRoot) {
        return;
      }
      const root = this.attachShadow({ mode: "open" });
      if (this.firstElementChild) {
        root.append(document.createElement("slot"));
        return;
      }
      const table = document.createElement("table");
      const caption = document.createElement("caption");
      table.append(caption);
      root.append(table);
      this.#applyLabel();
    }

    attributeChangedCallback() {
      if (this.shadowRoot) {
        this.#applyLabel();
      }
    }

    #applyLabel() {
      const table = this.shadowRoot.querySelector("table");
      if (!table) {
        return;
      }
      const caption = this.shadowRoot.querySelector("caption");
      const label = this.getAttribute("label");
      if (label) {
        table.setAttribute("aria-label", label);
      } else {
        table.removeAttribute("aria-label");
      }
      caption.textContent = label ?? "";
    }
  }

  customElements.define(name, OpengridTableFallback);
}

/**
 * A provider that runs the engine inside a module worker.
 *
 * The provider is attached to a host with `set_provider`; the worker itself is
 * started lazily and only once. The returned object matches the local provider
 * shape, so the page code is the same for both paths.
 *
 * @param {object} [options]
 * @param {string} [options.moduleUrl] wasm-bindgen glue of the engine module;
 *   defaults to the packaged one under `engine/`, which `worker.js` imports
 *   itself. A string, not a `URL`: it travels to the worker by `postMessage`.
 * @param {string} [options.wasmUrl] explicit `.wasm` URL, if it is not next to the glue.
 * @param {URL|string} [options.workerUrl] the worker entry; defaults to `worker.js` next to this file.
 * @returns {{load: Function, execute: Function, terminate: Function, worker?: Worker}}
 */
export function createWorkerProvider({
  moduleUrl,
  wasmUrl,
  workerUrl,
} = {}) {
  let worker;
  let ready;
  let nextId = 1;
  /** Pending replies by request id: id -> {resolve, reject}. */
  const pending = new Map();

  function onMessage(message) {
    const waiter = pending.get(message.id);
    if (!waiter) {
      return;
    }
    pending.delete(message.id);
    if (message.type === "error") {
      waiter.reject(new Error(message.message));
    } else if (message.type === "loaded") {
      waiter.resolve();
    } else {
      waiter.resolve(message.result);
    }
  }

  function request(message, transfer = []) {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      worker.postMessage({ ...message, id }, transfer);
    });
  }

  const watchers = createWatchers();

  // Rows as packs of columns (issue #445): to the worker, which owns the engine.
  let started = Promise.resolve();
  const loadRowsAsColumns = columnLoader({
    all: async (name, schema, rows, pack) => {
      await started;
      return request({ type: "columns", name, schema, rows, pack }, [pack]);
    },
    begin: async (name, schema, rows) => {
      await started;
      return request({ type: "columns_begin", name, schema, rows });
    },
    push: (pack) => request({ type: "columns_chunk", pack }, [pack]),
    end: () => request({ type: "columns_end" }),
    abort: async () => {
      await started;
      return request({ type: "columns_abort" });
    },
  });

  function start() {
    if (!worker) {
      // Written out in full: the `new Worker(new URL(…, import.meta.url), …)` form
      // is what bundlers recognise and bundle.
      worker = workerUrl
        ? new Worker(workerUrl, { type: "module" })
        : new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
      worker.onmessage = ({ data }) => onMessage(data);
      ready = request({ type: "init", moduleUrl, wasmUrl });
    }
    return ready;
  }

  return {
    /** Where the queries run; the grid names it in its footer (issue #33). */
    kind: "worker",
    /** Loads CSV bytes against a schema, starting the worker on first use. */
    async load(name, bytes, schema) {
      await start();
      const buffer = toTransferable(bytes);
      await request({ type: "load", name, bytes: buffer, schema }, [buffer]);
      watchers.loaded(name);
    },

    /**
     * Loads a CSV file piece by piece (issue #427): a `File`/`Blob`, a
     * `ReadableStream` of bytes or an async iterable of them. Only a piece and
     * the columns are in the engine's memory, not the whole file as well. A
     * name already loaded is replaced, all or nothing; a file that cannot be
     * read leaves the old source and rejects with the line.
     */
    async loadStream(name, source, schema, options = {}) {
      await start();
      await request({ type: "load_begin", name, schema, size: sizeOf(source) });
      try {
        for await (const buffer of piecesOf(source, options.chunkSize)) {
          await request({ type: "load_chunk", bytes: buffer }, [buffer]);
        }
        await request({ type: "load_end" });
      } catch (error) {
        await request({ type: "load_abort" });
        throw error;
      }
      watchers.loaded(name);
    },

    /**
     * Loads rows the page already holds — an array of objects keyed by field
     * name (or that array as JSON text) — against a schema (issue #182). A
     * name already loaded is replaced, all or nothing.
     */
    async loadRows(name, rows, schema) {
      // The worker starts while the rows are made into columns, which is most of the wait.
      started = start();
      if (!(await loadRowsAsColumns(name, rows, schema))) {
        await started;
        await request({ type: "rows", name, rows: rowsJson(rows), schema });
      }
      watchers.loaded(name);
    },

    /** Called by `set_provider`: the grids to tell when a source is replaced. */
    watch(host) {
      watchers.watch(host);
    },

    /** Runs a query JSON and resolves with the result in the binary form. */
    async execute(queryJson) {
      await start();
      return request({ type: "query", query: queryJson });
    },

    /**
     * Frees the source `name` (issue #459); resolves with whether there was
     * one. The worker's memory does not shrink, but the next load takes the
     * room the table had. A grid on the name answers its next query with an
     * unknown source.
     */
    async unload(name) {
      await start();
      return request({ type: "unload", name });
    },

    /**
     * What the source `name` is (issue #528): `{ name, schema }`, the schema as the
     * source has it, with the `from` of a derived field — a result never says. A
     * grid asks it once per `datasource`. Rejects for a source that is not loaded.
     */
    async describe(name) {
      await start();
      return { name, schema: JSON.parse(await request({ type: "describe", name })) };
    },

    /**
     * What the engine in the worker holds (issue #70): its WASM memory and,
     * per source, rows, columns and bytes.
     */
    async stats() {
      await start();
      return { kind: "worker", ...JSON.parse(await request({ type: "stats" })) };
    },

    /** Runs a pivot JSON and resolves with the pivot in the binary form (issue #28). */
    async pivot(pivotJson) {
      await start();
      return request({ type: "pivot", pivot: pivotJson });
    },

    /** Stops the worker and rejects everything still in flight. */
    terminate() {
      worker?.terminate();
      worker = undefined;
      ready = undefined;
      for (const waiter of pending.values()) {
        waiter.reject(new Error("worker terminated"));
      }
      pending.clear();
    },

    get worker() {
      return worker;
    },
  };
}

/**
 * The grids on an engine provider's sources (issue #182).
 *
 * `watch(host)` is called by `set_provider`; the host is held weakly, so a
 * removed grid is still collected. Loading a source — under a new
 * name or over a loaded one, which replaces it — tells every watching grid on
 * that name with an `opengrid-source-replaced` event — it re-runs its query and
 * keeps its view.
 */
function createWatchers() {
  const hosts = new Set();
  return {
    watch(host) {
      for (const ref of hosts) {
        if (ref.deref() === host) {
          return;
        }
      }
      hosts.add(new WeakRef(host));
    },
    /**
     * Tells the grids on `name` that it was loaded. A first load counts too: the
     * engine may hold the source from before this provider (a grid that asked
     * early shows its error until the rows arrive).
     */
    loaded(name) {
      for (const ref of [...hosts]) {
        const host = ref.deref();
        if (!host) {
          hosts.delete(ref);
        } else if (host.getAttribute("datasource") === name) {
          host.dispatchEvent(new CustomEvent("opengrid-source-replaced", { detail: { name } }));
        }
      }
    },
  };
}

/**
 * The grids a REST provider's source is shown in, and the server's stream that
 * says it changed (issue #328): `GET /watch/{source}` with the token — fetch,
 * not `EventSource`, which cannot send a header. The grid is told with a
 * cancelable `opengrid-source-changed` on its element and decides what the
 * reader sees; nothing is queried here. A cut stream is taken up again after
 * the server's `retry` (at most 30 s, longer after each failure); an answer of
 * 4xx ends it, since asking again would not change it.
 */
function createChangeStream(endpoint, token) {
  const hosts = new Set();
  let controller = null;
  let closed = false;

  const alive = () => {
    for (const ref of [...hosts]) {
      if (!ref.deref()) {
        hosts.delete(ref);
      }
    }
    return [...hosts].map((ref) => ref.deref());
  };

  const tell = (name) => {
    for (const host of alive()) {
      host.dispatchEvent(
        new CustomEvent("opengrid-source-changed", { detail: { name }, cancelable: true }),
      );
    }
  };

  async function run() {
    let wait = 5000;
    let failures = 0;
    while (!closed && alive().length > 0) {
      controller = new AbortController();
      try {
        const response = await fetch(endpoint, {
          headers: { Accept: "text/event-stream", ...(token ? { Authorization: `Bearer ${token}` } : {}) },
          signal: controller.signal,
          redirect: "error",
        });
        if (response.status >= 400 && response.status < 500) {
          return;
        }
        if (!response.ok || !response.body) {
          throw new Error(`status ${response.status}`);
        }
        failures = 0;
        const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
        let buffer = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) {
            break;
          }
          buffer += value;
          let end;
          while ((end = buffer.indexOf("\n\n")) >= 0) {
            const block = buffer.slice(0, end);
            buffer = buffer.slice(end + 2);
            const lines = block.split("\n");
            const retry = lines.find((line) => line.startsWith("retry:"));
            if (retry) {
              wait = Number(retry.slice(6)) || wait;
            }
            if (lines.includes("event: changed")) {
              const data = lines.find((line) => line.startsWith("data:"));
              let name;
              try {
                name = JSON.parse(data.slice(5)).source;
              } catch {
                name = undefined;
              }
              tell(name);
            }
          }
        }
      } catch {
        if (closed) {
          return;
        }
        failures += 1;
      }
      await new Promise((resolve) => setTimeout(resolve, Math.min(30000, wait * 2 ** Math.min(failures, 3))));
    }
    controller = null;
  }

  return {
    watch(host) {
      for (const ref of hosts) {
        if (ref.deref() === host) {
          return;
        }
      }
      hosts.add(new WeakRef(host));
      if (!controller && !closed) {
        controller = new AbortController();
        run();
      }
    },
    close() {
      closed = true;
      controller?.abort();
    },
  };
}

/** Rows as the engine reads them: a JSON array of objects, as text. */
function rowsJson(rows) {
  return typeof rows === "string" ? rows : JSON.stringify(rows);
}

// ---------------------------------------------------------------------------------------------
// Rows as columns (issue #445)
//
// An array of objects is handed to the engine as packs of typed columns, not as one JSON text:
// no text of the whole table on the main thread, in the worker and in the module, and no second
// reading of it. The format of a pack is in `crates/opengrid-engine/src/ingest/columns.rs`.
//
// The table has to be the table the JSON route makes of the same rows, so only what is plain is
// sent this way. A value that is not, an unknown key, a column the engine does not read from
// columns, or any error of the engine ends it, the engine drops what it had, and `loadRows` runs
// the JSON route, which owns every message.
// ---------------------------------------------------------------------------------------------

/** Rows per pack: a bound on what is in flight at once, as the pieces of `loadStream` are. */
const PACK_ROWS = 65536;
const textEncoder = new TextEncoder();

/** What the columns of `schema` are, or `null` when this way is not for these rows. */
function columnPlan(rows, schema) {
  // A text that is not well formed is not what `JSON.stringify` and the JSON reader would make of
  // it; a browser that cannot say takes the JSON route.
  if (!Array.isArray(rows) || rows.length === 0 || typeof "".isWellFormed !== "function") {
    return null;
  }
  let fields;
  try {
    fields = JSON.parse(schema)?.fields;
  } catch {
    return null;
  }
  if (!Array.isArray(fields)) {
    return null;
  }
  const kinds = { bool: "bool", int64: "int", float64: "float", utf8: "utf8", date: "date", timestamp: "timestamp" };
  const stored = [];
  for (const field of fields) {
    if (field?.from !== undefined && field?.from !== null) {
      continue; // derived: computed by the engine, never sent
    }
    const kind = field?.type?.decimal ? "decimal" : kinds[field?.type];
    if (typeof field?.name !== "string" || kind === undefined) {
      return null; // a list column, or a schema the engine will explain
    }
    stored.push({ name: field.name, kind });
  }
  return stored.length > 0 ? stored : null;
}

/**
 * One pack: rows `from` to `to` as typed columns, or `null` when a value is not plain.
 * Plain is what the JSON route reads the same way: a boolean in a bool column, a safe integer
 * in an int64, a finite number in a float64, a string in a text, date or timestamp column, a
 * string or an integer in a decimal; `null` and a missing key are NULL.
 */
function packOf(rows, from, to, stored) {
  const n = to - from;
  const pad = (size) => (size + 7) & ~7;
  const bitsLength = Math.ceil(n / 8);
  const sections = [];
  // The keys of a row against the keys the schema has: a row with a key it does not know is the
  // JSON route's, which names the key. `present` counts the keys of the schema a row has.
  const present = new Uint16Array(n);
  const plain = Object.prototype;
  for (let i = 0; i < n; i += 1) {
    const row = rows[from + i];
    if (row === null || typeof row !== "object" || Object.getPrototypeOf(row) !== plain) {
      return null;
    }
  }
  for (const { name, kind } of stored) {
    let bits = null; // the validity bitmap, made at the first NULL
    const missing = (i) => {
      bits ??= new Uint8Array(bitsLength).fill(255);
      bits[i >> 3] &= ~(1 << (i & 7));
    };
    if (kind === "bool") {
      const values = new Uint8Array(n);
      for (let i = 0; i < n; i += 1) {
        const value = rows[from + i][name];
        if (value === undefined) {
          missing(i);
          continue;
        }
        present[i] += 1;
        if (value === null) {
          missing(i);
        } else if (typeof value === "boolean") {
          values[i] = value ? 1 : 0;
        } else {
          return null;
        }
      }
      sections.push({ bits, payload: values, text: null });
    } else if (kind === "int" || kind === "float") {
      const values = new Float64Array(n);
      for (let i = 0; i < n; i += 1) {
        const value = rows[from + i][name];
        if (value === undefined) {
          missing(i);
          continue;
        }
        present[i] += 1;
        if (value === null) {
          missing(i);
        } else if (
          typeof value === "number" &&
          (kind === "int" ? Number.isSafeInteger(value) : Number.isFinite(value))
        ) {
          // JSON writes -0 as 0.
          values[i] = value === 0 ? 0 : value;
        } else {
          return null;
        }
      }
      sections.push({ bits, payload: new Uint8Array(values.buffer), text: null });
    } else {
      // Text, and what the engine reads from text: a decimal, a date, a timestamp.
      const offsets = new Uint32Array(n + 1);
      let bytes = new Uint8Array(Math.max(256, n * 12));
      let at = 0;
      for (let i = 0; i < n; i += 1) {
        let value = rows[from + i][name];
        if (value === undefined) {
          missing(i);
          offsets[i + 1] = at;
          continue;
        }
        present[i] += 1;
        if (value === null) {
          missing(i);
          offsets[i + 1] = at;
          continue;
        }
        if (kind === "decimal" && typeof value === "number" && Number.isSafeInteger(value)) {
          value = String(value);
        }
        if (typeof value !== "string") {
          return null;
        }
        const length = value.length;
        if (at + length * 3 > bytes.length) {
          const grown = new Uint8Array(Math.max(bytes.length * 2, at + length * 3));
          grown.set(bytes.subarray(0, at));
          bytes = grown;
        }
        // ASCII, which most text is, is copied as it is; the rest is encoded, if it is well formed.
        let ascii = true;
        for (let j = 0; j < length; j += 1) {
          const code = value.charCodeAt(j);
          if (code > 127) {
            ascii = false;
            break;
          }
          bytes[at + j] = code;
        }
        if (ascii) {
          at += length;
        } else if (value.isWellFormed()) {
          at += textEncoder.encodeInto(value, bytes.subarray(at)).written;
        } else {
          return null;
        }
        offsets[i + 1] = at;
      }
      sections.push({ bits, payload: new Uint8Array(offsets.buffer), text: bytes.subarray(0, at) });
    }
  }
  for (let i = 0; i < n; i += 1) {
    let keys = 0;
    for (const _key in rows[from + i]) {
      keys += 1;
    }
    if (keys !== present[i]) {
      return null;
    }
  }
  // The pack: the header, then each column's section from a multiple of 8.
  let total = pad(4 + 8 * stored.length);
  const starts = sections.map((section) => {
    const start = total;
    total += pad((section.bits ? pad(bitsLength) : 0) + section.payload.length + (section.text?.length ?? 0));
    return start;
  });
  const pack = new ArrayBuffer(total);
  const view = new DataView(pack);
  const all = new Uint8Array(pack);
  view.setUint32(0, n, true);
  sections.forEach((section, index) => {
    view.setUint32(4 + 8 * index, section.bits ? 1 : 0, true);
    view.setUint32(8 + 8 * index, section.text?.length ?? 0, true);
    let at = starts[index];
    if (section.bits) {
      all.set(section.bits, at);
      at += pad(bitsLength);
    }
    all.set(section.payload, at);
    if (section.text) {
      all.set(section.text, at + section.payload.length);
    }
  });
  return pack;
}

/**
 * Loads `rows` as packs of columns through `io`, or says that it did not (`false`): the rows are
 * not plain, or the engine refused them or does not know the calls. The engine holds nothing of
 * them then, and the caller takes the JSON route.
 */
async function loadColumns(name, rows, schema, io) {
  const stored = columnPlan(rows, schema);
  if (!stored) {
    return false;
  }
  try {
    if (rows.length <= PACK_ROWS) {
      const pack = packOf(rows, 0, rows.length, stored);
      if (!pack) {
        return false;
      }
      await io.all(name, schema, rows.length, pack);
      return true;
    }
    // The next pack is made while the engine takes the one before: a pack waits for the one two
    // before it, not for the one before (which would leave the engine idle while a pack is made),
    // and the page gets the thread back between packs. The first is made before `begin` is asked,
    // so that a worker still starting does its start while that is done.
    let pack = packOf(rows, 0, PACK_ROWS, stored);
    if (!pack) {
      return false;
    }
    await io.begin(name, schema, rows.length);
    const sent = [];
    let refused = null;
    for (let from = 0; from < rows.length && !refused; from += PACK_ROWS) {
      pack ??= packOf(rows, from, Math.min(rows.length, from + PACK_ROWS), stored);
      if (!pack) {
        refused = new Error("rows that are not plain");
        break;
      }
      sent.push(io.push(pack).catch((error) => (refused ??= error)));
      pack = null;
      if (sent.length >= 2) {
        await sent[sent.length - 2];
      }
    }
    await Promise.all(sent);
    if (refused) {
      throw refused;
    }
    await io.end();
    return true;
  } catch {
    try {
      await io.abort();
    } catch {
      // Nothing to drop, or no engine to ask: the JSON route is next either way.
    }
    return false;
  }
}

/**
 * [`loadColumns`] one at a time: the engine keeps one set of rows in the making, so two loads
 * must not interleave their packs.
 */
function columnLoader(io) {
  let queue = Promise.resolve();
  return (name, rows, schema) => {
    const run = queue.then(() => loadColumns(name, rows, schema, io));
    queue = run.catch(() => {});
    return run;
  };
}

/**
 * A provider over an engine that is already initialised on the main thread.
 *
 * The fallback path of the elements: no worker, the same shape as
 * `createWorkerProvider`, so a page can swap the two without changing the grid.
 *
 * @param {object} engine the loaded `Engine` from the engine module.
 * @returns {{load: Function, execute: Function, terminate: Function}}
 */
export function createLocalProvider(engine) {
  const watchers = createWatchers();
  // Rows as packs of columns (issue #445); an engine module without the calls takes the JSON route.
  const loadRowsAsColumns = columnLoader({
    all: async (name, schema, rows, pack) => {
      engine.begin_columns(name, schema, rows);
      engine.push_columns(new Uint8Array(pack));
      engine.end_columns();
    },
    begin: async (name, schema, rows) => engine.begin_columns(name, schema, rows),
    push: async (pack) => engine.push_columns(new Uint8Array(pack)),
    end: async () => engine.end_columns(),
    abort: async () => engine.abort_columns(),
  });
  return {
    /** Where the queries run; the grid names it in its footer (issue #33). */
    kind: "local",
    async load(name, bytes, schema) {
      engine.load_csv(
        name,
        bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes),
        schema,
      );
      watchers.loaded(name);
    },
    async loadStream(name, source, schema, options = {}) {
      engine.begin_csv(name, schema, sizeOf(source));
      try {
        for await (const buffer of piecesOf(source, options.chunkSize)) {
          engine.push_csv(new Uint8Array(buffer));
        }
        engine.end_csv();
      } catch (error) {
        engine.abort_csv();
        throw error;
      }
      watchers.loaded(name);
    },
    async loadRows(name, rows, schema) {
      if (!(await loadRowsAsColumns(name, rows, schema))) {
        engine.load_rows(name, rowsJson(rows), schema);
      }
      watchers.loaded(name);
    },
    watch(host) {
      watchers.watch(host);
    },
    /** The answer itself, or a promise of it while the collator loads (S4a). */
    execute(queryJson) {
      return withCollator(engine, queryJson, () => engine.execute_columns(queryJson));
    },
    /** Frees the source `name` (issue #459), as the worker provider does. */
    async unload(name) {
      return engine.unload(name);
    },
    /** What the source `name` is (issue #528), as the worker provider says it. */
    async describe(name) {
      return { name, schema: JSON.parse(engine.describe(name)) };
    },
    /** What the engine holds (issue #70), as the worker provider says it. */
    async stats() {
      return { kind: "local", ...JSON.parse(engine.stats()) };
    },
    /** A pivot over the engine's sources, in the binary form (issue #28). */
    pivot(pivotJson) {
      return engine.pivot_columns(pivotJson);
    },
    terminate() {},
  };
}

/**
 * A provider that talks to an `opengrid-server` over HTTP.
 *
 * The same shape as `createLocalProvider` and `createWorkerProvider`, so a page
 * swaps local for remote without touching the grid: `set_provider` sees one
 * `execute(queryJson)` either way. The difference is only where the work
 * happens — and that the answer can fail for reasons a local engine never has.
 *
 * The server's error form (docs/protocol.md, Errors) is unwrapped here: a failed request
 * rejects with the server's own sentence, so the grid's status line shows
 * "unknown source …" rather than "HTTP 404". The `Error` also carries the
 * HTTP `status` and the form's `code` and `path` (issue #16), so a page can
 * tell a `busy` server from a request that is too big without reading the
 * sentence.
 *
 * No request follows a redirect (`redirect: "error"`): the bearer token goes
 * to the URL the page configured, and nowhere a response points to.
 *
 * @param {object} options
 * @param {string} options.url base URL of the server, e.g. `http://127.0.0.1:8081`.
 * @param {string} options.source the configured data source name.
 * @param {string} [options.token] bearer token; the server refuses without one.
 * @returns {{execute: Function, describe: Function, export: Function, exportPivot: Function}}
 */
export function createRestProvider({ url, source, token, watch = false } = {}) {
  const base = String(url).replace(/\/$/, "");
  const endpoint = `${base}/query/${encodeURIComponent(source)}`;
  const headers = { "Content-Type": "application/json", Accept: ACCEPT_ANSWER };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  // The export answers a file, CSV or JSON, whatever a query would.
  const { Accept: _answer, ...exportHeaders } = headers;
  const changes = watch
    ? createChangeStream(`${base}/watch/${encodeURIComponent(source)}`, token)
    : null;

  return {
    /**
     * With `watch: true`, `set_provider` calls this: the grid hears
     * `opengrid-source-changed` when the server says the source changed
     * (issue #328). A page does not call it.
     */
    watch(host) {
      changes?.watch(host);
    },

    /** Closes the change stream, if there is one. */
    terminate() {
      changes?.close();
    },

    /** Where the queries run; the grid names it in its footer (issue #33). */
    kind: "remote",
    /**
     * What the server says this source is: `{ name, schema, capabilities }`. A
     * grid asks it for the derived fields of its source (issue #528); the
     * provider is for one source, so an argument is ignored.
     *
     * A planner needs both halves before it can split anything — the
     * schema to validate against, the capabilities to know what may be pushed.
     * The schema is the one this token is allowed to see.
     */
    async describe() {
      const response = await fetch(`${base}/source/${encodeURIComponent(source)}`, {
        headers: token ? { Authorization: headers.Authorization } : {},
        redirect: "error",
      });
      const text = await response.text();
      if (!response.ok) {
        throw refusal(text, response.status);
      }
      return JSON.parse(text);
    },

    async execute(queryJson, _mode, { signal } = {}) {
      const response = await fetch(endpoint, {
        method: "POST",
        headers,
        body: queryJson,
        signal,
        redirect: "error",
      });
      return answerOf(response);
    },

    /**
     * `POST /export/{source}`: every row of `query` in **one** streamed request,
     * as a `Blob` (issue #2). The server reads its source through a cursor
     * and writes the rows in the notation `exportRows` writes in the tab, so
     * the file is the same byte for byte — one request instead of pieces, and
     * against PostgreSQL no `OFFSET` that gets dearer with every piece.
     *
     * The server's rules are `/query`'s: the token, `allowed_fields`, the
     * mandatory row filter. Its own bound is `max_export_rows`; more is an
     * error with the server's sentence before the first byte. The row count
     * arrives before the rows (`X-Total-Count`), so `maxRows` refuses without
     * reading the body, and `onProgress` hears `{ rows, total }` once, at the
     * end. `signal` aborts the request, the download included; the server
     * notices at its next piece and ends the query.
     *
     * @param {object} query the query, as an object — as `exportRows` takes it.
     * @param {object} [options] `format`, `signal`, `maxRows`, `onProgress`
     *   and the CSV options (`delimiter`, `bom`, `protectFormulas`, `null`).
     * @returns {Promise<Blob>} `text/csv;charset=utf-8` or `application/json`.
     */
    async export(query, { format = "csv", signal, maxRows, onProgress, headers, ...csv } = {}) {
      for (const key of Object.keys(csv)) {
        if (!CSV_OPTIONS.includes(key)) {
          throw new TypeError(`export: unknown option "${key}"`);
        }
      }
      // The server checks the options (a wrong one is its 400, with a
      // sentence); here they only become parameters, under their own names.
      const parameters = new URLSearchParams({ format });
      for (const key of CSV_OPTIONS) {
        if (csv[key] !== undefined) {
          parameters.set(key, String(csv[key]));
        }
      }
      // The header texts of CSV and XLSX, one parameter each (issue #353).
      for (const [column, text] of Object.entries(headers ?? {})) {
        parameters.set(`header.${column}`, text);
      }
      const response = await fetch(`${base}/export/${encodeURIComponent(source)}?${parameters}`, {
        method: "POST",
        headers: exportHeaders,
        body: JSON.stringify(query),
        signal,
        redirect: "error",
      });
      if (!response.ok) {
        throw refusal(await response.text(), response.status);
      }
      // The count is the server's promise about the rows that follow; without
      // it there is nothing to hold `maxRows` or the progress against, and a
      // missing one read as 0 would pass any bound.
      const counted = response.headers.get("X-Total-Count");
      if (counted === null || !/^\d+$/.test(counted.trim())) {
        await response.body?.cancel();
        throw new Error("export: the server did not say how many rows follow (X-Total-Count)");
      }
      const total = Number(counted);
      if (maxRows !== undefined && total > maxRows) {
        // Not one row more than needed: the body is dropped unread.
        await response.body?.cancel();
        throw coded(
          "too_many_rows",
          `export: ${total} rows match, more than the ${maxRows} an export may have (maxRows)`,
        );
      }
      const blob = await response.blob();
      onProgress?.({ rows: total, total });
      // The type `exportRows` gives its Blob, whatever the header's spelling.
      return new Blob([blob], { type: EXPORT_TYPES[format] ?? EXPORT_TYPES.csv });
    },

    /** `POST /pivot/{source}?format=xlsx`: see {@link pivotWorkbook}. */
    exportPivot(pivotQuery, options) {
      return pivotWorkbook(`${base}/pivot/${encodeURIComponent(source)}`, exportHeaders, pivotQuery, options);
    },
  };
}

/**
 * The texts a pivot's workbook is written in: the keys `set_texts` gives
 * them, and the parameters `POST /pivot/{source}?format=xlsx` reads.
 */
const PIVOT_TEXTS = ["noValue", "emptyValue", "total", "subtotal"];

/**
 * What a calculated measure is shown as (issue #329), after `PIVOT_TEXTS` in
 * the order `pivot_words` gives them. Sent only with a query that has one: a
 * server from before calculated measures refuses a parameter it does not know.
 */
const PIVOT_CALC_TEXTS = ["shareOfRow", "shareOfColumn", "shareOfTotal", "runningTotal"];

/**
 * `POST /pivot/{source}?format=xlsx`: a pivot as an XLSX file, a `Blob`
 * (issue #331) — what `exportPivot` of the REST and the pivot provider does.
 *
 * The server answers the pivot of `pivotQuery` as one flat sheet, the table a
 * pivot's CSV is: one header row, every row with its subtotals and the grand
 * total, numbers and dates as cells of their type. Its rules are `/pivot`'s —
 * the token, `allowed_fields`, the mandatory row filter, the pivot's limits —
 * and an export's: `max_xlsx_rows`, and a place among the running exports.
 *
 * `texts` are the words for a total and for a NULL or empty group, under the
 * keys of {@link PIVOT_TEXTS}, and — for a query with a calculated measure —
 * the words of {@link PIVOT_CALC_TEXTS}; any other key is ignored. `exportPivot` reads
 * them from the element; a page that calls this method with a query of its
 * own hands them over, or gets the server's English defaults.
 *
 * @param {string} endpoint `/pivot/{source}` of the server.
 * @param {object} headers the provider's, without `Accept`.
 * @param {object} pivotQuery the pivot query, as an object.
 * @param {object} [options] `format` (`"xlsx"`), `texts`, `signal`.
 * @returns {Promise<Blob>} the workbook.
 */
async function pivotWorkbook(endpoint, headers, pivotQuery, options = {}) {
  for (const key of Object.keys(options)) {
    if (!PIVOT_EXPORT_OPTIONS.includes(key)) {
      throw new TypeError(`exportPivot: unknown option "${key}"`);
    }
  }
  const { format, texts, signal } = options;
  if (format !== "xlsx") {
    throw new TypeError(
      `exportPivot: format is "xlsx", not ${JSON.stringify(format)} — a pivot's CSV is get_pivot(host)`,
    );
  }
  const parameters = new URLSearchParams({ format });
  const calculated = Array.isArray(pivotQuery?.values) && pivotQuery.values.some((value) => value?.calc);
  for (const key of calculated ? [...PIVOT_TEXTS, ...PIVOT_CALC_TEXTS] : PIVOT_TEXTS) {
    if (typeof texts?.[key] === "string") {
      parameters.set(key, texts[key]);
    }
  }
  const response = await fetch(`${endpoint}?${parameters}`, {
    method: "POST",
    headers,
    body: JSON.stringify(pivotQuery),
    signal,
    redirect: "error",
  });
  if (!response.ok) {
    throw refusal(await response.text(), response.status);
  }
  // A server from before this export ignores the parameter and answers the
  // pivot itself: that is not a file to hand out under a workbook's name.
  const type = (response.headers.get("Content-Type") ?? "").split(";")[0].trim().toLowerCase();
  if (type !== EXPORT_TYPES.xlsx) {
    await response.body?.cancel();
    throw new Error("exportPivot: the server answered the pivot, not a workbook — it does not write a pivot as XLSX");
  }
  return new Blob([await response.blob()], { type: EXPORT_TYPES.xlsx });
}

/**
 * What a provider asks the server for: the binary result form (E35), JSON as
 * the fallback — a server that does not know the form answers JSON, and
 * `answerOf` takes that too.
 */
const COLUMNS_TYPE = "application/vnd.opengrid.columns";
const ACCEPT_ANSWER = `${COLUMNS_TYPE}, application/json;q=0.9`;

/**
 * A response as a provider's answer: the bytes of the binary form, the result
 * JSON otherwise, the server's refusal as an `Error`.
 */
async function answerOf(response) {
  if (!response.ok) {
    throw refusal(await response.text(), response.status);
  }
  const type = response.headers.get("Content-Type") ?? "";
  if (type.split(";")[0].trim().toLowerCase() === COLUMNS_TYPE) {
    return new Uint8Array(await response.arrayBuffer());
  }
  return response.text();
}

/**
 * A failed request as the `Error` it rejects with (issue #16). The message is
 * the server's own sentence, or the status when the body is not the error
 * form of docs/protocol.md (Errors). The fields are what a page branches on without reading
 * the sentence: `status` always, `code` and `path` when the error form has
 * them.
 */
function refusal(body, status) {
  let form;
  try {
    form = JSON.parse(body)?.error;
  } catch {
    // A body that is not the error form: the status is all there is.
  }
  let message = `HTTP ${status}`;
  if (form?.message) {
    message = form.path ? `${form.message} (${form.path})` : form.message;
  }
  const error = new Error(message);
  error.status = status;
  if (typeof form?.code === "string") {
    error.code = form.code;
  }
  if (typeof form?.path === "string") {
    error.path = form.path;
  }
  return error;
}

/**
 * An `Error` of the loader's own with a `code` a page can branch on (issue
 * #16), in the style of the server's: the sentence is for the developer, the
 * code for the page. The codes are frozen in
 * crates/opengrid-web-components/src/api.rs, which reads them from the calls
 * of this function.
 */
function coded(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

// The collator of the engine (rule S4a): `collation.js` and the module behind it are fetched
// by the first query that names a locale, and never otherwise — so a page that serves the
// package's files itself and leaves them out loses locale sorts, not the grid. The same test
// is in `worker.js`; the two must agree.
const NAMES_A_LOCALE = /"collation"\s*:\s*"(?!binary")/;
/** The engines and planners that were given their collator. */
const withCollation = new WeakSet();

async function giveCollation(engine) {
  let collation;
  try {
    collation = await import("./collation.js");
  } catch (error) {
    throw coded(
      "module_not_loaded",
      `a sort by a locale needs collation.js, and it did not load (${error?.message ?? error})`,
    );
  }
  await collation.giveCollator(engine);
  withCollation.add(engine);
}

/**
 * Runs `run`, the answer itself — or, for a query that names a locale and an engine that has
 * no collator yet, a promise of it once the collation module is loaded.
 */
function withCollator(engine, json, run) {
  if (withCollation.has(engine) || !NAMES_A_LOCALE.test(json)) {
    return run();
  }
  return giveCollation(engine).then(run);
}

/**
 * A provider that asks an `opengrid-server` for a whole **pivot**.
 *
 * The same shape as every other provider — `execute(json)` in, JSON out — so
 * `set_provider` is unchanged; only the endpoint and the body differ. What comes
 * back is the pivot wire form: the ordinary result under `result`, plus the
 * level of each row and what each generated column stands for.
 *
 * @param {object} options
 * @param {string} options.url base URL of the server.
 * @param {string} options.source the configured data source name.
 * @param {string} [options.token] bearer token.
 * @returns {{execute: Function, pivot: Function, exportPivot: Function}}
 */
export function createPivotProvider({ url, source, token } = {}) {
  const endpoint = `${String(url).replace(/\/$/, "")}/pivot/${encodeURIComponent(source)}`;
  const headers = { "Content-Type": "application/json", Accept: ACCEPT_ANSWER };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }

  async function pivot(pivotJson, _mode, { signal } = {}) {
    const response = await fetch(endpoint, {
      method: "POST",
      headers,
      body: pivotJson,
      signal,
      redirect: "error",
    });
    return answerOf(response);
  }

  // The export answers a file, whatever a pivot would.
  const { Accept: _answer, ...exportHeaders } = headers;

  return {
    /** Where the queries run; the grid names it in its footer (issue #33). */
    kind: "remote",
    // `pivot` is what `<opengrid-pivot>` calls (issue #28); `execute` stays for
    // a page that called it before.
    execute: pivot,
    pivot,
    /** `POST /pivot/{source}?format=xlsx`: see {@link pivotWorkbook}. */
    exportPivot(pivotQuery, options) {
      return pivotWorkbook(endpoint, exportHeaders, pivotQuery, options);
    },
  };
}

/**
 * A provider that splits each query between a remote source and the engine in
 * this tab.
 *
 * Three steps, all of them JSON:
 *
 * 1. the `Planner` splits the query into the half the source can answer and the
 *    half that is left,
 * 2. the remote provider answers its half,
 * 3. the engine finishes the rest over that answer.
 *
 * When the source can answer everything, step 3 does not happen and the remote
 * answer is passed through untouched — so a capable server costs nothing.
 *
 * The plan is handed to `onPlan` before anything is sent: that is how it becomes
 * "offengelegt für Debugging, Performance-Analyse und Developer-Tools", and it
 * is a plain object (`{ mode, describe, steps, source, client }`).
 *
 * @param {object} options
 * @param {{execute: Function}} options.remote the source-side provider, usually
 *   `createRestProvider(...)`.
 * @param {{plan: Function, finish: Function}} options.planner a `Planner` from
 *   the WASM module, built with the source's schema and capabilities.
 * @param {string} [options.mode] `local`, `remote`, `hybrid` or `auto`; the
 *   element's `mode` attribute overrides it per query.
 * @param {Function} [options.onPlan] called with each plan.
 * @returns {{execute: Function}}
 */
export function createHybridProvider({ remote, planner, mode = "auto", onPlan } = {}) {
  return {
    /** Where the queries run; the grid names it in its footer (issue #33). */
    kind: "hybrid",
    // What the source is, where the remote half can say (issue #528).
    ...(typeof remote?.describe === "function" ? { describe: (source) => remote.describe(source) } : {}),
    async execute(queryJson, elementMode, { signal } = {}) {
      // The element's attribute wins when it has one: the page sets the default,
      // the markup can override it per grid.
      const plan = JSON.parse(planner.plan(queryJson, elementMode || mode));
      if (onPlan) {
        onPlan(plan);
      }
      // The remote half is the one that can be stopped; the engine's half runs
      // after it and is synchronous. The source half has no element mode of
      // its own: the plan already decided where it runs.
      const partial = await remote.execute(JSON.stringify(plan.source), "", { signal });
      if (!plan.client) {
        return partial;
      }
      // The engine finishes in the form the source answered in — with its
      // collator, when the client's half sorts by a locale (S4a).
      const client = JSON.stringify(plan.client);
      return withCollator(planner, client, () =>
        typeof partial === "string"
          ? planner.finish(client, partial)
          : planner.finish_columns(client, partial),
      );
    },
  };
}

/** The media type of each export's `Blob`. */
const EXPORT_TYPES = {
  csv: "text/csv;charset=utf-8",
  json: "application/json",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
};

/** The options of `exportPivot`, the function and the providers' method. */
const PIVOT_EXPORT_OPTIONS = ["format", "texts", "signal"];
/** The options of `exportRows` that are its own. */
const EXPORT_OPTIONS = ["format", "chunkSize", "maxRows", "onProgress", "signal", "headers"];
/**
 * The options handed to `export_csv` — only these, whatever else is passed.
 * The same list as `CSV_OPTION_KEYS` in
 * crates/opengrid-web-components/src/export.rs, which reads them; the two
 * must stay in step (a test in that crate's api.rs compares them).
 */
const CSV_OPTIONS = ["delimiter", "bom", "protectFormulas", "null"];
/** A result without rows or columns: enough for `export_csv` to read its options. */
const NO_ROWS = '{"total_count":0,"columns":[]}';

/**
 * Every match of `query`, fetched through `provider` in pieces, as a `Blob` of
 * CSV or JSON.
 *
 * The browser half of exporting the view: `get_query(host)` says what the
 * reader sees, this fetches all of it and writes it in the notation the server
 * uses too (`opengrid-export`, through the element module's `export_csv` and
 * `export_json`). The grid has no export button; the button, the file name and
 * what to say afterwards are the page's.
 *
 * **Stable pieces.** The pieces are `offset`/`limit` windows, and a window is
 * only stable under a total order: the grid's sort has at least one key but may
 * tie, and against PostgreSQL a row can then repeat or go missing across two
 * `OFFSET`s. So every selected column not yet in the sort is appended to it,
 * ascending. Rows equal in every selected column look the same whichever copy
 * comes — and within a tie the export follows the columns, not the order the
 * grid happened to show.
 *
 * **A source that changes is refused, not exported.** The tie-breaker fixes
 * ties; it cannot fix rows that come or go between two pieces — they would
 * shift the windows, and a row would repeat or go missing. So every piece has
 * to report the first piece's `total_count`, and a piece that ends before
 * `total` is an error too. A change that keeps the count and moves a row is not
 * visible from here.
 *
 * **Bounded.** `total` is the first piece's `total_count`; a total above
 * `maxRows` is an error before anything else is fetched, never a truncated
 * file.
 *
 * **Errors a page can tell apart** (issue #16): its own carry a `code` —
 * `too_many_rows`, `source_changed`, `module_not_loaded` — and a provider's
 * rejection passes through as it came, the server's `status` and `code`
 * included. A wrong option is a `TypeError`, an abort an `AbortError`.
 *
 * **Cancellable.** `signal` reaches the provider as `execute(json, mode,
 * { signal })`; an abort rejects with an `AbortError` at once, leaves no request
 * running where the provider can stop one, and produces no `Blob`.
 *
 * **One request where the provider can.** A provider with an `export(query,
 * options)` method — `createRestProvider` — gets the whole export in one
 * streamed request (issue #2): the same query, the same options checked here
 * first, `signal`, `maxRows` and `onProgress` handed on. `chunkSize` does not
 * apply, and `onProgress` hears the end.
 *
 * @param {{execute: Function}} provider any provider — the one the grid uses.
 * @param {object} query a query without a window, as `get_query(host)` gives it.
 * @param {object} [options] see `ExportOptions` in `loader.d.ts`.
 * @returns {Promise<Blob>} `text/csv;charset=utf-8` or `application/json`.
 */
export async function exportRows(provider, query, options = {}) {
  for (const key of Object.keys(options)) {
    if (!EXPORT_OPTIONS.includes(key) && !CSV_OPTIONS.includes(key)) {
      throw new TypeError(`exportRows: unknown option "${key}"`);
    }
  }
  const {
    format = "csv",
    chunkSize = 10_000,
    maxRows: askedMaxRows,
    onProgress,
    signal,
    headers,
    delimiter,
    bom,
    protectFormulas,
    null: nullText,
  } = options;
  if (format !== "csv" && format !== "json" && format !== "xlsx") {
    throw new TypeError(
      `exportRows: format is "csv", "json" or "xlsx", not ${JSON.stringify(format)}`,
    );
  }
  // A workbook built in the tab is held whole in memory — about a kilobyte a row
  // of five columns, a gigabyte at a million — so its default is lower than the
  // server's, which streams to a file; asked for, it is the page's to raise.
  const maxRows =
    askedMaxRows ?? (format === "xlsx" && typeof provider?.export !== "function" ? XLSX_TAB_MAX_ROWS : 1_000_000);
  if (format !== "csv") {
    // A CSV option on another export would do nothing, and do it silently.
    const stray = CSV_OPTIONS.find((key) => options[key] !== undefined);
    if (stray) {
      throw new TypeError(
        `exportRows: "${stray}" is a CSV option, and this export is ${format.toUpperCase()}`,
      );
    }
  }
  if (headers !== undefined) {
    if (format === "json") {
      throw new TypeError("exportRows: headers name the header row, and a JSON export has none");
    }
    if (
      headers === null ||
      typeof headers !== "object" ||
      Object.values(headers).some((text) => typeof text !== "string")
    ) {
      throw new TypeError("exportRows: headers is an object of texts, by column");
    }
  }
  if (!Number.isSafeInteger(chunkSize) || chunkSize < 1) {
    throw new TypeError("exportRows: chunkSize is a whole number of rows, at least 1");
  }
  if (!Number.isSafeInteger(maxRows) || maxRows < 0) {
    throw new TypeError("exportRows: maxRows is a whole number of rows");
  }
  if (!query || typeof query !== "object") {
    throw new TypeError("exportRows: no query — get_query answers null when there is nothing to export");
  }
  if (!Array.isArray(query.select) || query.select.length === 0) {
    // The tie-breaker sorts by the selected columns, so it has to know them.
    throw new TypeError("exportRows: the query names no columns in `select`");
  }
  if ("offset" in query || "limit" in query) {
    throw new TypeError("exportRows: the query has a window; exportRows fetches every match itself");
  }
  if ("group" in query || "aggregate" in query) {
    // Its output columns are aliases `select` does not name, so the
    // tie-breaker could not make its order total. A view's query has neither.
    throw new TypeError("exportRows: the query groups or aggregates; an export is of a view's rows");
  }
  if (signal?.aborted) {
    throw aborted();
  }

  // The export notation of CSV and JSON is in the element module; a workbook
  // is written by a module of its own, loaded below when the pieces come.
  let module;
  if (format !== "xlsx") {
    const loaded = await loadOpengrid();
    if (loaded.fallback) {
      throw coded(
        "module_not_loaded",
        "exportRows: the WebAssembly module did not load, and the export notation is in it",
      );
    }
    module = loaded.module;
  }
  const csvOptions = { delimiter, bom, protectFormulas, null: nullText };
  if (format === "csv") {
    // Read the options now: a wrong one should cost no request.
    module.export_csv(NO_ROWS, csvOptions, false, headers);
  }

  const sort = [...(query.sort ?? [])];
  for (const field of query.select) {
    if (!sort.some((key) => key.field === field)) {
      sort.push({ field, direction: "asc" });
    }
  }

  if (typeof provider.export === "function") {
    // A provider that exports by itself — `createRestProvider` — does it in
    // one request. The query is the same, tie-breaker included, so the file
    // is the one the pieces would have made; one cursor needs no tie-breaker,
    // but the order within a tie is then the same whichever way was taken.
    const blob = await unlessAborted(
      provider.export(
        { ...query, sort },
        { format, signal, maxRows, onProgress, headers, ...(format === "csv" ? csvOptions : {}) },
      ),
      signal,
    );
    // An abort during `onProgress` still means no file.
    if (signal?.aborted) {
      throw aborted();
    }
    return blob;
  }

  // A workbook is built in the tab's memory by the XLSX module, piece by piece,
  // and is the file at the end: there is no temporary file to stream to (#332).
  let book;
  try {
    return await fetchPieces();
  } finally {
    book?.free();
  }

  async function fetchPieces() {
    // Each piece becomes a Blob of its own as soon as it is written, so its text
    // can leave the JS heap; the browser may keep a Blob's bytes elsewhere.
    const parts = format === "json" ? [new Blob(["["])] : [];
    let rows = 0;
    let total;
    for (;;) {
      if (signal?.aborted) {
        throw aborted();
      }
      // After the first piece nothing past `total` is asked for.
      const limit = total === undefined ? chunkSize : Math.min(chunkSize, total - rows);
      const piece = JSON.stringify({ ...query, sort, offset: rows, limit });
      const answer = await unlessAborted(ask(provider, piece, signal), signal);
      const result = countsOf(answer);
      const count = result.rows;
      const first = total === undefined;
      if (first) {
        total = result.total_count;
        if (total > maxRows) {
          throw coded(
            "too_many_rows",
            `exportRows: ${total} rows match, more than the ${maxRows} an export may have (maxRows)`,
          );
        }
        if (format === "xlsx" && total > XLSX_MAX_ROWS) {
          // Excel's limit is an error, never a shortened file — as on the server.
          throw coded(
            "too_many_rows",
            `exportRows: ${total} rows match, more than the ${XLSX_MAX_ROWS} an Excel sheet holds`,
          );
        }
      } else if (result.total_count !== total) {
        throw changed(`${total} rows matched at first, ${result.total_count} at row ${rows + 1}`);
      }
      if (count < limit && rows + count < total) {
        throw changed(`the rows ended at ${rows + count} of ${total}`);
      }
      if (format === "xlsx") {
        // After the checks above: a refused export does not load the module.
        if (!book) {
          book = new (await loadXlsx()).XlsxBook(sheetNameOf(query));
          if (headers) {
            book.headers(headers);
          }
        }
        book.add(answer);
      } else {
        parts.push(
          new Blob([
            format === "csv"
              ? module.export_csv(answer, csvOptions, first, headers)
              : // `first` here is "no row written yet", not "first piece": after an
                // empty first piece the next one still leads without a comma.
                module.export_json(answer, rows === 0),
          ]),
        );
      }
      rows += count;
      onProgress?.({ rows, total });
      if (rows >= total) {
        break;
      }
    }
    // An abort during the last `onProgress` still means no file.
    if (signal?.aborted) {
      throw aborted();
    }
    if (format === "xlsx") {
      return new Blob([book.finish()], { type: EXPORT_TYPES.xlsx });
    }
    if (format === "json") {
      parts.push(new Blob(["]"]));
    }
    return new Blob(parts, { type: EXPORT_TYPES[format] });
  }
}

/** The most rows an Excel sheet holds, as `XLSX_MAX_ROWS` in `opengrid-export`. */
const XLSX_MAX_ROWS = 1_048_575;

/** `maxRows` of a workbook written in the tab, unless the page says otherwise. */
const XLSX_TAB_MAX_ROWS = 250_000;

/** The name of the workbook's sheet: the source, which the writer may refuse. */
function sheetNameOf(query) {
  return typeof query.source === "string" && query.source !== "" ? query.source : "export";
}

/**
 * The XLSX writer of the tab, loaded the first time a workbook is asked for and
 * kept: a module of its own (`xlsx/`, built by `just wasm-build-xlsx`), so a
 * page that never exports to Excel does not fetch it. A failed load is not
 * kept, so a later call tries again.
 */
let xlsxModule;
async function loadXlsx() {
  xlsxModule ??= import("./xlsx/opengrid_xlsx.js").then(async (glue) => {
    await glue.default();
    return glue;
  });
  try {
    return await xlsxModule;
  } catch (error) {
    xlsxModule = undefined;
    throw coded(
      "module_not_loaded",
      `exportRows: the XLSX module did not load (${error?.message ?? error})`,
    );
  }
}

/**
 * `total_count` and the rows of an answer, whichever form it came in: from the
 * JSON, or from the header of the binary form (E35) — `total_count` at byte 8,
 * the row count at byte 16, both little-endian `u64`.
 */
function countsOf(answer) {
  if (typeof answer === "string") {
    const result = JSON.parse(answer);
    return { total_count: result.total_count, rows: result.columns[0]?.values.length ?? 0 };
  }
  const bytes = answer instanceof ArrayBuffer ? new Uint8Array(answer) : answer;
  if (bytes.byteLength < 24) {
    throw new TypeError("exportRows: the provider answered neither JSON nor the binary form");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const columns = view.getUint32(24, true);
  return {
    total_count: Number(view.getBigUint64(8, true)),
    rows: columns === 0 ? 0 : Number(view.getBigUint64(16, true)),
  };
}

/** One piece from the provider; a synchronous throw becomes a rejection. */
async function ask(provider, queryJson, signal) {
  return provider.execute(queryJson, "", { signal });
}

/**
 * `promise`, unless `signal` aborts first — then an `AbortError` at once. The
 * tab and the worker cannot stop a query that has started; their answer is not
 * waited for.
 */
function unlessAborted(promise, signal) {
  if (!signal) {
    return promise;
  }
  return new Promise((resolve, reject) => {
    const onAbort = () => reject(aborted());
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(resolve, reject).finally(() => signal.removeEventListener("abort", onAbort));
  });
}

/**
 * What an export rejects with when the source changed under it: the pieces no
 * longer add up to one result, and a file of them would be wrong without
 * saying so.
 */
function changed(detail) {
  return coded(
    "source_changed",
    `exportRows: the source changed during the export (${detail}); export again`,
  );
}

/**
 * The pivot `host` shows, as an XLSX file from the server (issue #331).
 *
 * `get_pivot` writes a pivot's CSV in the element, from the answer it holds.
 * A workbook is written by `opengrid-server`, so this asks the server for the
 * pivot again — the element's view as a pivot query, through the provider's
 * `exportPivot` — and gets the same table as one flat sheet: one header row,
 * every row with its subtotals and the grand total in the table's order,
 * numbers and dates as cells of their type. Folded groups are exported, as
 * in the CSV. If the data moved since the table was drawn, the file is the
 * newer answer.
 *
 * **Only a provider that exports on the server**: `createPivotProvider` and
 * `createRestProvider` have `exportPivot`. With any other provider — the
 * engine in the tab or in a worker — it is a `TypeError`, as an XLSX of
 * `exportRows` is there.
 *
 * **The words are the element's.** The totals and the NULL and empty groups
 * read as in the table: the element's texts as they are now — its defaults,
 * or what `set_texts` made of them — are read from it and sent along. The
 * page hands over nothing.
 *
 * @param {{exportPivot: Function}} provider the server-backed provider.
 * @param {HTMLElement} host an `<opengrid-pivot>`.
 * @param {object} options `format` (`"xlsx"`), `signal`.
 * @returns {Promise<Blob>} the workbook.
 */
export async function exportPivot(provider, host, options = {}) {
  for (const key of Object.keys(options)) {
    // `texts` is the provider method's: here they are the element's.
    if (key === "texts" || !PIVOT_EXPORT_OPTIONS.includes(key)) {
      throw new TypeError(`exportPivot: unknown option "${key}"`);
    }
  }
  if (options.format !== "xlsx") {
    throw new TypeError(
      `exportPivot: format is "xlsx", not ${JSON.stringify(options.format)} — a pivot's CSV is get_pivot(host)`,
    );
  }
  if (typeof provider?.exportPivot !== "function") {
    // An XLSX file is written on the server; the pivot the element holds
    // cannot become one here.
    throw new TypeError(
      "exportPivot: an XLSX export needs a provider that exports on the server (createPivotProvider, createRestProvider)",
    );
  }
  if (options.signal?.aborted) {
    throw aborted();
  }
  const loaded = await loadOpengrid();
  if (loaded.fallback) {
    throw coded(
      "module_not_loaded",
      "exportPivot: the WebAssembly module did not load, and the pivot's view is read through it",
    );
  }
  const source = host?.getAttribute?.("datasource");
  const view = host?.localName === "opengrid-pivot" ? loaded.module.get_view(host) : null;
  if (!view || !source) {
    throw new TypeError("exportPivot: no pivot to export — an <opengrid-pivot> in the document, with a datasource");
  }
  // The request the element sends: `sort` and `filter` only when there is one.
  const query = { source, rows: view.rows, columns: view.columns, values: view.values };
  if (view.sort.length > 0) {
    query.sort = view.sort;
  }
  if (view.filter) {
    query.filter = view.filter;
  }
  // The element's words, in the order of `PIVOT_TEXTS` and `PIVOT_CALC_TEXTS`.
  const words = loaded.module.pivot_words(host);
  const texts = Object.fromEntries([...PIVOT_TEXTS, ...PIVOT_CALC_TEXTS].map((key, index) => [key, words[index]]));
  return provider.exportPivot(query, { ...options, texts });
}

/** What an aborted export rejects with, whatever reason `abort()` was given. */
function aborted() {
  return new DOMException("The export was aborted.", "AbortError");
}

/**
 * Supplies an element from one options object, and keeps it supplied.
 *
 * Every page — and every framework adapter — would otherwise have to know the
 * rules of the module functions itself: wait for the module; set the texts
 * before the provider, or the first paint is in English and the grid rebuilds;
 * set the view before the provider, or the grid asks twice and announces
 * twice; and write a controlled view back without looping. They live here,
 * once.
 *
 * `update` applies only what changed. A key left out of an update keeps its
 * value; a key given as `undefined` resets it (texts to English, no formats, no
 * presentation, no choices, no actions, no validators) — except `provider`,
 * which has no "none", and `view`, where `undefined` (or `null`) means the grid leads. Attributes
 * (`label`, `columns`, `group-by`, …) are not options: the page or the
 * framework sets them on the element as usual.
 *
 * **The view is controlled.** It is written whenever it differs from what the
 * grid shows: after the reader sorted, passing the same saved view again puts
 * it back. Writing back what the grid just reported costs nothing. `defaultView`
 * is the uncontrolled form: applied once, then the grid leads.
 *
 * The view needs the element in the document (before that it has no view to
 * set). On an element that is not, the view **and the provider** wait for the
 * next `update` — together, so the first query still asks for the view.
 *
 * **Editing** (issue #206): `onRowAdd` answers with the new row's state — or a
 * promise of it — which is `set_row_state`; `setCellState` and `setValues` on
 * the connection are `set_cell_state` and `set_values`, and wait for the
 * module like every option does.
 *
 * `connect` loads the module with `loadOpengrid()`, whose first call decides
 * the URLs: a page that needs its own calls `loadOpengrid(options)` first.
 *
 * @param {HTMLElement} host an `<opengrid-grid>`, `-table` or `-pivot`.
 * @param {object} options see `ConnectOptions` in `loader.d.ts`.
 * @returns {{ready: Promise<{fallback: boolean, module?: object}>, update: Function, setCellState: Function, setValues: Function, disconnect: Function}}
 */
export function connect(host, options = {}) {
  let current = { ...options };
  let module;
  let connected = true;
  /** What was last written to the element, per option, as a comparable key. */
  const applied = new Map();
  /** The whole view the element last reported, as a key. */
  let reported;
  /** The view this connection last wrote, and what the grid reported for it. */
  let written;
  let writtenResult;
  /** True while this connection's own `set_view` runs. */
  let writing = false;
  let defaultApplied = false;
  let deferred = false;

  const listeners = [
    [
      "opengrid-view-change",
      (event) => {
        reported = stableKey(event.detail.view);
        if (writing) {
          // The report of our own write: the grid took the view.
          writtenResult = reported;
          return;
        }
        current.onViewChange?.(event.detail.view);
      },
    ],
    ["opengrid-selection-change", (event) => current.onSelectionChange?.(event.detail)],
    ["opengrid-cell-change", (event) => current.onCellChange?.(event.detail)],
    ["opengrid-row-add", (event) => answerRowAdd(event.detail)],
    ["opengrid-paste", (event) => current.onPaste?.(event.detail)],
    ["opengrid-bulk-change", (event) => current.onBulkChange?.(event.detail)],
    ["opengrid-row-action", (event) => current.onRowAction?.(event.detail)],
    ["opengrid-cell-action", (event) => current.onCellAction?.(event.detail)],
    ["opengrid-drill-down", (event) => current.onDrillDown?.(event.detail)],
    ["opengrid-source-replaced", (event) => current.onSourceReplaced?.(event.detail)],
  ];
  for (const [type, listener] of listeners) {
    host.addEventListener(type, listener);
  }

  /**
   * `onRowAdd`'s answer becomes the row's state: a promise shows `saving` until
   * it settles — resolved with nothing, the row is saved — and a rejection is
   * an `error` with its message. An undone add has no row left to answer for.
   */
  function answerRowAdd(detail) {
    const answer = current.onRowAdd?.(detail);
    if (answer == null || detail.undo) {
      return;
    }
    const key = detail.provisionalKey;
    if (typeof answer.then !== "function") {
      setRowState(key, answer);
      return;
    }
    setRowState(key, { state: "saving" });
    answer.then(
      (state) => setRowState(key, state ?? { state: "saved" }),
      (error) => setRowState(key, { state: "error", message: String(error?.message ?? error) }),
    );
  }

  function setRowState(key, { state, key: saved, message }) {
    if (module && connected) {
      module.set_row_state(host, key, state, state === "saved" ? saved : message);
    }
  }

  /** Runs `call` with the module — now, or once it has loaded. */
  function withModule(call) {
    if (module) {
      call(module);
    } else {
      ready.then(() => module && connected && call(module));
    }
  }

  /** Writes one option if it differs from what the element has. */
  function write(name, value, key, apply) {
    if (applied.has(name) && applied.get(name) === key) {
      return;
    }
    applied.set(name, key);
    apply(value);
  }

  /**
   * Writes `view` unless the grid shows it already: it just reported exactly
   * this, or this connection wrote it and the grid has not moved since. Only a
   * view the grid took counts as written — one it refused (a column it does
   * not have yet) is tried again on the next update.
   */
  function writeView(view) {
    const key = stableKey(view);
    if (key === reported || (key === written && writtenResult === reported)) {
      return;
    }
    writing = true;
    writtenResult = undefined;
    try {
      module.set_view(host, view);
    } finally {
      writing = false;
    }
    if (writtenResult !== undefined) {
      written = key;
    } else {
      written = undefined;
    }
    // The view the element now has, whether or not it changed.
    reported = stableKey(module.get_view(host));
    if (written !== undefined) {
      writtenResult = reported;
    }
  }

  function apply(changed) {
    if (!module || !connected) {
      return;
    }
    // The order is the point: texts first (they are in the skeleton), then
    // what the rows are drawn with, then the view, then the provider — so the
    // first query is the only one and asks for the right thing.
    if ("texts" in changed) {
      const texts = changed.texts ?? {};
      write("texts", texts, stableKey(texts), (value) => {
        // `set_texts` adds to the texts the element has; this option is all of
        // them, so what an earlier value set and this one leaves out goes first.
        module.set_texts(host, null);
        if (Object.keys(value).length > 0) {
          module.set_texts(host, value);
        }
      });
    }
    if ("formats" in changed) {
      const formats = changed.formats ?? {};
      write("formats", formats, identityKey(formats), (value) => module.set_formats(host, value));
    }
    if ("presentation" in changed) {
      const presentation = changed.presentation ?? {};
      write("presentation", presentation, stableKey(presentation), (value) =>
        module.set_columns(host, value),
      );
    }
    if ("choices" in changed) {
      const choices = changed.choices ?? {};
      write("choices", choices, stableKey(choices), (value) => module.set_choices(host, value));
    }
    if ("actions" in changed) {
      const actions = changed.actions ?? [];
      write("actions", actions, stableKey(actions), (value) => module.set_actions(host, value));
    }
    if ("cellActions" in changed) {
      const cellActions = changed.cellActions ?? [];
      write("cellActions", cellActions, stableKey(cellActions), (value) =>
        module.set_cell_actions(host, value),
      );
    }
    if ("cellTemplates" in changed) {
      const cellTemplates = changed.cellTemplates ?? {};
      write("cellTemplates", cellTemplates, identityKey(cellTemplates), (value) =>
        module.set_cell_templates(host, value),
      );
    }
    if ("detailRenderer" in changed) {
      const detailRenderer = changed.detailRenderer ?? null;
      write("detailRenderer", detailRenderer, identityKey({ detailRenderer }), (value) =>
        module.set_detail_renderer(host, value),
      );
    }
    if ("validators" in changed) {
      const validators = changed.validators ?? {};
      write("validators", validators, identityKey(validators), (value) =>
        module.set_validators(host, value),
      );
    }

    const wantsView =
      current.view != null ? "view" in changed || deferred : !defaultApplied && current.defaultView != null;
    if (wantsView && !host.isConnected) {
      // Nothing to set a view on yet; the provider waits with it, or the grid
      // would ask for its default order first.
      if (!deferred) {
        console.warn("[opengrid] connect: the element is not in the document; call update() once it is");
      }
      deferred = true;
      return;
    }
    if (wantsView) {
      deferred = false;
      if (reported === undefined) {
        reported = stableKey(module.get_view(host));
      }
      if (current.view != null) {
        writeView(current.view);
      } else {
        defaultApplied = true;
        writeView(current.defaultView);
      }
    }
    if (("provider" in changed || "provider" in current) && current.provider) {
      write("provider", current.provider, current.provider, (value) =>
        module.set_provider(host, value),
      );
    }
  }

  const ready = loadOpengrid().then((loaded) => {
    if (!loaded.fallback) {
      module = loaded.module;
      apply(current);
    }
    return loaded;
  });

  return {
    ready,
    update(next = {}) {
      if (!connected) {
        return;
      }
      current = { ...current, ...next };
      apply(next);
    },
    setCellState(key, column, state, message) {
      withModule((loaded) => loaded.set_cell_state(host, key, column, state, message));
    },
    setValues(values) {
      withModule((loaded) => loaded.set_values(host, values));
    },
    disconnect() {
      connected = false;
      for (const [type, listener] of listeners) {
        host.removeEventListener(type, listener);
      }
    },
  };
}

// ---------------------------------------------------------------------------------------------
// From plain rows to a table (issue #460)
//
// The engine never guesses a type; the package may, for a page that has `[{…}, {…}]` and no
// schema. What it guesses is an ordinary schema JSON, handed back to the caller, so the guess is
// visible, can be saved, and can be passed to `loadRows` unchanged.
// ---------------------------------------------------------------------------------------------

const DATE_TEXT = /^(\d{4})-(\d{2})-(\d{2})$/;

/** A `YYYY-MM-DD` that is a day of the calendar: `2026-02-30` is text, not a date. */
function isDateText(text) {
  const parts = DATE_TEXT.exec(text);
  if (!parts) {
    return false;
  }
  const [year, month, day] = [Number(parts[1]), Number(parts[2]) - 1, Number(parts[3])];
  const date = new Date(0);
  date.setUTCFullYear(year, month, day);
  return date.getUTCFullYear() === year && date.getUTCMonth() === month && date.getUTCDate() === day;
}

/** What a value is, in the words of a refusal. */
function shapeOf(value) {
  if (Array.isArray(value)) {
    return "a list";
  }
  return { bigint: "a bigint", object: "an object" }[typeof value] ?? `a ${typeof value}`;
}

function refuseField(row, name, why) {
  throw coded("cannot_infer", `inferSchema: row ${row}, field ${name}: ${why}; give a schema`);
}

/**
 * The schema JSON of `rows`, read from every row: a boolean is `bool`; a number `int64` when every
 * value of the field is an integer, else `float64`; a string `date` when every value is a
 * `YYYY-MM-DD` day, else `utf8`; a field that is `null` or missing in a row is nullable. The
 * fields are in the order they first appear.
 *
 * It refuses — an `Error` with `code: "cannot_infer"`, naming row and field — what it cannot type without
 * guessing: values of two kinds in one field, an integer beyond 2^53, a number that is not finite,
 * a bigint, an object or a list, a field with no value in any row. A field with fractions becomes
 * `float64` and is named in a warning: money wants a `decimal` in a schema of its own.
 *
 * @param {Record<string, unknown>[]} rows
 * @returns {string} the schema JSON `loadRows` takes.
 */
export function inferSchema(rows) {
  if (!Array.isArray(rows) || rows.length === 0) {
    throw coded("cannot_infer", "inferSchema: no rows to read the types from; give a schema");
  }
  /** Per field, in order: the kind of its values, where it was seen first, and what it held. */
  const fields = new Map();
  rows.forEach((row, index) => {
    if (row === null || typeof row !== "object" || Object.getPrototypeOf(row) !== Object.prototype) {
      throw coded("cannot_infer", `inferSchema: row ${index} is ${row === null ? "null" : shapeOf(row)}, not an object of fields`);
    }
    for (const [name, value] of Object.entries(row)) {
      let field = fields.get(name);
      if (!field) {
        field = { kind: null, row: index, values: 0, fraction: false, dates: true };
        fields.set(name, field);
      }
      if (value === null || value === undefined) {
        continue;
      }
      let kind;
      if (typeof value === "boolean") {
        kind = "bool";
      } else if (typeof value === "number") {
        if (!Number.isFinite(value)) {
          refuseField(index, name, `${value} is not a finite number`);
        }
        if (Number.isInteger(value) && !Number.isSafeInteger(value)) {
          refuseField(index, name, `${value} is beyond 2^53 and not exact in JavaScript`);
        }
        kind = "number";
        field.fraction ||= !Number.isInteger(value);
      } else if (typeof value === "string") {
        kind = "string";
        field.dates &&= isDateText(value);
      } else {
        refuseField(index, name, `${shapeOf(value)} cannot be typed`);
      }
      if (field.kind === null) {
        field.kind = kind;
        field.row = index;
      } else if (field.kind !== kind) {
        refuseField(index, name, `a ${kind} where row ${field.row} has a ${field.kind}`);
      }
      field.values += 1;
    }
  });
  const schema = [];
  for (const [name, field] of fields) {
    if (field.kind === null) {
      throw coded("cannot_infer", `inferSchema: field ${name} is null or missing in every row; give a schema`);
    }
    const type = {
      bool: "bool",
      number: field.fraction ? "float64" : "int64",
      string: field.dates ? "date" : "utf8",
    }[field.kind];
    if (field.fraction) {
      console.warn(
        `[opengrid] inferSchema: field ${name} has fractions and becomes float64. ` +
          "For money, give it a decimal in a schema: " +
          `{ "name": "${name}", "type": { "decimal": { "precision": 12, "scale": 2 } } }`,
      );
    }
    schema.push({ name, type, nullable: field.values < rows.length });
  }
  return JSON.stringify({ fields: schema });
}

/** Up to this many distinct values a column is offered as a list facet. */
const FACET_LIST_MAX = 12;

/**
 * What the schema suggests for how the columns look, as the two objects a page hands to
 * `set_columns` and `set_formats` — a plain function, so the page can log the result, change it and
 * pass it on; the grid guesses nothing of its own. The rules, field by field:
 *
 * - the `rowKey` field: read-only, monospaced, muted and narrow; nothing else;
 * - an integer or a decimal: aligned at the end and summed in groups; a float: aligned at the end;
 * - a date: a `date` format, in the locale of the page;
 * - a text or an integer with at most 12 distinct values — `distinct` says how many each field has,
 *   the function never counts — a `list` facet.
 *
 * A field no rule fits is left out. It throws a plain `Error` for what is wrong with the call: a
 * schema that is not the JSON text of `{ fields }`, a `rowKey` or a `distinct` name the schema does
 * not have, a count that is not a whole number.
 *
 * @param {string} schema the schema JSON, as `inferSchema` writes it and `loadRows` takes it.
 * @param {{ rowKey?: string, distinct?: Record<string, number> }} [options]
 * @returns {{ columns: Record<string, object>, formats: Record<string, object> }}
 */
export function suggestColumns(schema, options = {}) {
  let fields;
  try {
    fields = JSON.parse(schema)?.fields;
  } catch {
    fields = undefined;
  }
  if (!Array.isArray(fields)) {
    throw new Error("suggestColumns: the schema is not the JSON text of { fields: [...] }");
  }
  const { rowKey, distinct = {} } = options;
  const names = new Set(fields.map((field) => field?.name));
  if (rowKey !== undefined && !names.has(rowKey)) {
    throw new Error(`suggestColumns: rowKey ${rowKey} is not a field of the schema`);
  }
  for (const [name, count] of Object.entries(distinct)) {
    if (!names.has(name)) {
      throw new Error(`suggestColumns: distinct names ${name}, which is not a field of the schema`);
    }
    if (!Number.isInteger(count) || count < 0) {
      throw new Error(`suggestColumns: distinct ${name} is ${count}, not a count of values`);
    }
  }
  const columns = {};
  const formats = {};
  for (const { name, type } of fields) {
    const few = distinct[name] <= FACET_LIST_MAX; // false where no count was handed in
    if (name === rowKey) {
      columns[name] = { readonly: true, mono: true, muted: true, width: 96 };
    } else if (type === "int64" || type?.decimal) {
      columns[name] = { align: "end", aggregate: "sum", ...(few && type === "int64" ? { facet: "list" } : {}) };
    } else if (type === "float64") {
      columns[name] = { align: "end" };
    } else if (type === "date") {
      formats[name] = { kind: "date" };
    } else if (type === "utf8" && few) {
      columns[name] = { facet: "list" };
    }
  }
  return { columns, formats };
}

/**
 * Rows to a table in one call: the schema (`schema`, or `inferSchema(rows)`), a provider (the
 * worker, unless `provider` is given), the rows loaded into it, and the element connected.
 * `datasource` becomes `rows` and `columns` every field, where the element has none. For
 * `<opengrid-grid>` and `<opengrid-table>`; a pivot's `columns` are its own.
 *
 * @param {HTMLElement} host
 * @param {Record<string, unknown>[]} rows
 * @param {{schema?: string, provider?: object}} [options]
 * @returns {Promise<{schema: string, provider: object, connection: object}>}
 */
export async function fromRows(host, rows, { schema, provider } = {}) {
  const used = schema ?? inferSchema(rows);
  const target = provider ?? createWorkerProvider();
  if (!host.getAttribute("datasource")) {
    host.setAttribute("datasource", "rows");
  }
  if (!host.getAttribute("columns")) {
    const names = JSON.parse(used).fields.filter((field) => field.from == null).map((field) => field.name);
    host.setAttribute("columns", names.join(","));
  }
  // The module loads while the rows go in; the provider is set once they are there.
  await Promise.all([target.loadRows(host.getAttribute("datasource"), rows, used), loadOpengrid()]);
  const connection = connect(host, { provider: target });
  await connection.ready;
  return { schema: used, provider: target, connection };
}

/**
 * A JSON key that does not depend on the order an object's keys were written
 * in; `each` may stand in for a value first.
 */
function stableKey(value, each = (inner) => inner) {
  return JSON.stringify(value, (_, raw) => {
    const inner = each(raw);
    return inner && typeof inner === "object" && !Array.isArray(inner)
      ? Object.fromEntries(Object.entries(inner).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
      : inner;
  });
}

/**
 * Formats and validators compare by identity where they are functions and by
 * value elsewhere (`Intl` options, `block`). Function identities are numbered
 * per call site so two different functions never compare equal.
 */
const functionIds = new WeakMap();
let nextFunctionId = 1;
function identityKey(value) {
  return stableKey(value, (inner) => {
    if (typeof inner !== "function") {
      return inner;
    }
    if (!functionIds.has(inner)) {
      functionIds.set(inner, nextFunctionId++);
    }
    return { function: functionIds.get(inner) };
  });
}

/** The bytes of a stream source when it says how many there are, else 0 (a hint, not a promise). */
function sizeOf(source) {
  return typeof source?.size === "number" ? source.size : 0;
}

/**
 * The pieces of a CSV source: a `Blob` is read a slice at a time, a stream or an
 * async iterable as it delivers. Each piece is an `ArrayBuffer` the caller may transfer.
 */
async function* piecesOf(source, chunkSize = 4 * 1024 * 1024) {
  for await (const [piece, owned] of piecesAndOwners(source, chunkSize)) {
    // A piece the loader made, or a stream's chunk, is the loader's to hand on; the bytes an async
    // iterable yields may be reused by it, so they are copied.
    yield owned && piece.byteOffset === 0 && piece.byteLength === piece.buffer.byteLength
      ? piece.buffer
      : toTransferable(piece);
  }
}

async function* piecesAndOwners(source, chunkSize) {
  if (typeof Blob !== "undefined" && source instanceof Blob) {
    for (let at = 0; at < source.size; at += chunkSize) {
      yield [new Uint8Array(await source.slice(at, at + chunkSize).arrayBuffer()), true];
    }
    return;
  }
  if (typeof source?.getReader === "function") {
    const reader = source.getReader();
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) return;
        yield [value, true];
      }
    } finally {
      reader.releaseLock();
    }
  }
  if (typeof source?.[Symbol.asyncIterator] === "function") {
    for await (const piece of source) yield [piece, false];
    return;
  }
  throw new TypeError("a CSV source must be a Blob, a ReadableStream or an async iterable of bytes");
}

/**
 * The CSV bytes as a transferable `ArrayBuffer`.
 *
 * A view into a larger buffer is copied first, so transferring never detaches
 * data the caller still owns; a plain `ArrayBuffer` is transferred as is.
 */
function toTransferable(bytes) {
  if (bytes instanceof ArrayBuffer) {
    return bytes;
  }
  if (ArrayBuffer.isView(bytes)) {
    return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  }
  throw new TypeError("bytes must be an ArrayBuffer or a typed array");
}