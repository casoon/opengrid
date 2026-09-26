/**
 * @casoon/opengrid loader (plan point 13, plan/spezifikation/08-rendering.md
 * §JavaScript-Minimum).
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
 * renders; point 14 replaces it once the table carries data.
 *
 * Point 19 adds the two providers. Both expose the same tiny shape the elements
 * consume (`load`, `execute`, `terminate`), so a page switches between the
 * Worker and the main thread without touching the grid:
 *
 *   createWorkerProvider({ moduleUrl, wasmUrl })  engine in a module worker
 *   createLocalProvider(engine)                   engine on the main thread
 *   createRestProvider({ url, source, token })    opengrid-server über HTTP (point 27)
 *
 * Every provider's `execute(queryJson, mode, { signal })` may take an
 * `AbortSignal` as its third argument (point 84). The providers that talk HTTP
 * hand it to `fetch`, so an aborted export leaves no request running; the tab
 * and the worker cannot stop a query that has started and ignore it — the
 * caller drops their answer.
 *
 * The Worker provider starts lazily on the first `load`/`execute` and stays the
 * single worker of V1 (no pool, no SharedArrayBuffer).
 */

/** Default location of the wasm-bindgen glue; point 40 pins the packaged path. */
const DEFAULT_MODULE_URL = new URL("./pkg/opengrid_web_components.js", import.meta.url);

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
 *   `set_provider` (point 14) and `set_texts` (point 48, call it first: it
 *   rebuilds the component).
 */
export function loadOpengrid(options = {}) {
  if (!loading) {
    loading = start(options);
  }
  return loading;
}

async function start({ moduleUrl = DEFAULT_MODULE_URL, wasmUrl } = {}) {
  try {
    const module = await import(moduleUrl);
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
 * Not exported: `loadOpengrid` installs it when the module fails to load, and a
 * page that reaches for it directly is asking for the broken state on purpose
 * (plan point 39 — an export is a promise).
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
 * @param {object} options
 * @param {string} options.moduleUrl wasm-bindgen glue of the engine module. A
 *   string, not a `URL`: it travels to the worker by `postMessage`.
 * @param {string} [options.wasmUrl] explicit `.wasm` URL, if it is not next to the glue.
 * @param {URL|string} [options.workerUrl] the worker entry; defaults to `worker.js` next to this file.
 * @returns {{load: Function, execute: Function, terminate: Function, worker?: Worker}}
 */
export function createWorkerProvider({ moduleUrl, wasmUrl, workerUrl } = {}) {
  const workerScript = workerUrl ?? new URL("./worker.js", import.meta.url);
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

  function start() {
    if (!worker) {
      worker = new Worker(workerScript, { type: "module" });
      worker.onmessage = ({ data }) => onMessage(data);
      ready = request({ type: "init", moduleUrl, wasmUrl });
    }
    return ready;
  }

  return {
    /** Loads CSV bytes against a schema, starting the worker on first use. */
    async load(name, bytes, schema) {
      await start();
      const buffer = toTransferable(bytes);
      await request({ type: "load", name, bytes: buffer, schema }, [buffer]);
    },

    /** Runs a query JSON and resolves with the result JSON. */
    async execute(queryJson) {
      await start();
      return request({ type: "query", query: queryJson });
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
 * A provider over an engine that is already initialised on the main thread.
 *
 * The fallback path of the elements (point 14): no worker, the same shape as
 * `createWorkerProvider`, so a page can swap the two without changing the grid.
 *
 * @param {object} engine the loaded `Engine` from the engine module.
 * @returns {{load: Function, execute: Function, terminate: Function}}
 */
export function createLocalProvider(engine) {
  return {
    async load(name, bytes, schema) {
      engine.load_csv(
        name,
        bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes),
        schema,
      );
    },
    execute(queryJson) {
      return engine.execute(queryJson);
    },
    terminate() {},
  };
}

/**
 * A provider that talks to an `opengrid-server` over HTTP (plan point 27).
 *
 * The same shape as `createLocalProvider` and `createWorkerProvider`, so a page
 * swaps local for remote without touching the grid: `set_provider` sees one
 * `execute(queryJson)` either way. The difference is only where the work
 * happens — and that the answer can fail for reasons a local engine never has.
 *
 * The server's error form (point 23) is unwrapped here: a failed request
 * rejects with the server's own sentence, so the grid's status line shows
 * "unknown source …" rather than "HTTP 404".
 *
 * @param {object} options
 * @param {string} options.url base URL of the server, e.g. `http://127.0.0.1:8081`.
 * @param {string} options.source the configured data source name.
 * @param {string} [options.token] bearer token; the server refuses without one.
 * @returns {{execute: Function}}
 */
export function createRestProvider({ url, source, token } = {}) {
  const base = String(url).replace(/\/$/, "");
  const endpoint = `${base}/query/${encodeURIComponent(source)}`;
  const headers = { "Content-Type": "application/json" };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }

  return {
    /**
     * What the server says this source is: `{ name, schema, capabilities }`.
     *
     * A planner needs both halves before it can split anything (point 28) — the
     * schema to validate against, the capabilities to know what may be pushed.
     * The schema is the one this token is allowed to see.
     */
    async describe() {
      const response = await fetch(`${base}/source/${encodeURIComponent(source)}`, {
        headers: token ? { Authorization: headers.Authorization } : {},
      });
      const text = await response.text();
      if (!response.ok) {
        throw new Error(messageOf(text, response.status));
      }
      return JSON.parse(text);
    },

    async execute(queryJson, _mode, { signal } = {}) {
      const response = await fetch(endpoint, {
        method: "POST",
        headers,
        body: queryJson,
        signal,
      });
      const text = await response.text();
      if (response.ok) {
        return text;
      }
      throw new Error(messageOf(text, response.status));
    },
  };
}

/**
 * The server's own sentence for a failed request, or the status when the body
 * is not the error form of point 23.
 */
function messageOf(body, status) {
  try {
    const error = JSON.parse(body)?.error;
    if (error?.message) {
      return error.path ? `${error.message} (${error.path})` : error.message;
    }
  } catch {
    // A body that is not the error form: keep the status.
  }
  return `HTTP ${status}`;
}

/**
 * A provider that asks an `opengrid-server` for a whole **pivot** (point 53).
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
 * @returns {{execute: Function}}
 */
export function createPivotProvider({ url, source, token } = {}) {
  const endpoint = `${String(url).replace(/\/$/, "")}/pivot/${encodeURIComponent(source)}`;
  const headers = { "Content-Type": "application/json" };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }

  return {
    async execute(pivotJson, _mode, { signal } = {}) {
      const response = await fetch(endpoint, {
        method: "POST",
        headers,
        body: pivotJson,
        signal,
      });
      const text = await response.text();
      if (response.ok) {
        return text;
      }
      throw new Error(messageOf(text, response.status));
    },
  };
}

/**
 * A provider that splits each query between a remote source and the engine in
 * this tab (plan point 28, plan/spezifikation/05-planner.md).
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
      return planner.finish(JSON.stringify(plan.client), partial);
    },
  };
}

/** The options of `exportRows` that are its own. */
const EXPORT_OPTIONS = ["format", "chunkSize", "maxRows", "onProgress", "signal"];
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
 * CSV or JSON (plan point 84).
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
 * **Cancellable.** `signal` reaches the provider as `execute(json, mode,
 * { signal })`; an abort rejects with an `AbortError` at once, leaves no request
 * running where the provider can stop one, and produces no `Blob`.
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
    maxRows = 1_000_000,
    onProgress,
    signal,
    delimiter,
    bom,
    protectFormulas,
    null: nullText,
  } = options;
  if (format !== "csv" && format !== "json") {
    throw new TypeError(`exportRows: format is "csv" or "json", not ${JSON.stringify(format)}`);
  }
  if (format === "json") {
    // A CSV option on a JSON export would do nothing, and do it silently.
    const stray = CSV_OPTIONS.find((key) => options[key] !== undefined);
    if (stray) {
      throw new TypeError(`exportRows: "${stray}" is a CSV option, and this export is JSON`);
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

  const loaded = await loadOpengrid();
  if (loaded.fallback) {
    throw new Error("exportRows: the WebAssembly module did not load, and the export notation is in it");
  }
  const { module } = loaded;
  const csvOptions = { delimiter, bom, protectFormulas, null: nullText };
  if (format === "csv") {
    // Read the options now: a wrong one should cost no request.
    module.export_csv(NO_ROWS, csvOptions, false);
  }

  const sort = [...(query.sort ?? [])];
  for (const field of query.select) {
    if (!sort.some((key) => key.field === field)) {
      sort.push({ field, direction: "asc" });
    }
  }

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
    const result = JSON.parse(answer);
    const count = result.columns[0]?.values.length ?? 0;
    const first = total === undefined;
    if (first) {
      total = result.total_count;
      if (total > maxRows) {
        throw new Error(
          `exportRows: ${total} rows match, more than the ${maxRows} an export may have (maxRows)`,
        );
      }
    } else if (result.total_count !== total) {
      throw changed(`${total} rows matched at first, ${result.total_count} at row ${rows + 1}`);
    }
    if (count < limit && rows + count < total) {
      throw changed(`the rows ended at ${rows + count} of ${total}`);
    }
    parts.push(
      new Blob([
        format === "csv"
          ? module.export_csv(answer, csvOptions, first)
          : // `first` here is "no row written yet", not "first piece": after an
            // empty first piece the next one still leads without a comma.
            module.export_json(answer, rows === 0),
      ]),
    );
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
  if (format === "json") {
    parts.push(new Blob(["]"]));
  }
  return new Blob(parts, {
    type: format === "csv" ? "text/csv;charset=utf-8" : "application/json",
  });
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
  return new Error(`exportRows: the source changed during the export (${detail}); export again`);
}

/** What an aborted export rejects with, whatever reason `abort()` was given. */
function aborted() {
  return new DOMException("The export was aborted.", "AbortError");
}

/**
 * Supplies an element from one options object, and keeps it supplied (plan
 * point 76).
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
 * presentation, no choices) — except `provider`, which has no "none", and
 * `view`, where `undefined` (or `null`) means the grid leads. Attributes
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
 * `connect` loads the module with `loadOpengrid()`, whose first call decides
 * the URLs: a page that needs its own calls `loadOpengrid(options)` first.
 *
 * @param {HTMLElement} host an `<opengrid-grid>`, `-table` or `-pivot`.
 * @param {object} options see `ConnectOptions` in `loader.d.ts`.
 * @returns {{ready: Promise<{fallback: boolean, module?: object}>, update: Function, disconnect: Function}}
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
  ];
  for (const [type, listener] of listeners) {
    host.addEventListener(type, listener);
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
      write("texts", texts, stableKey(texts), (value) => module.set_texts(host, value));
    }
    if ("formats" in changed) {
      const formats = changed.formats ?? {};
      write("formats", formats, formatsKey(formats), (value) => module.set_formats(host, value));
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
    disconnect() {
      connected = false;
      for (const [type, listener] of listeners) {
        host.removeEventListener(type, listener);
      }
    },
  };
}

/** A JSON key that does not depend on the order an object's keys were written in. */
function stableKey(value) {
  return JSON.stringify(value, (_, inner) =>
    inner && typeof inner === "object" && !Array.isArray(inner)
      ? Object.fromEntries(Object.entries(inner).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)))
      : inner,
  );
}

/**
 * Formats compare by identity where they are functions and by value where
 * they are `Intl` options. Function identities are numbered per call site so
 * two different functions never compare equal.
 */
const functionIds = new WeakMap();
let nextFunctionId = 1;
function formatsKey(formats) {
  return stableKey(
    Object.fromEntries(
      Object.entries(formats).map(([column, format]) => {
        if (typeof format !== "function") {
          return [column, format];
        }
        if (!functionIds.has(format)) {
          functionIds.set(format, nextFunctionId++);
        }
        return [column, { function: functionIds.get(format) }];
      }),
    ),
  );
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