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
 *   installed, and the loaded WASM module so a page can call its exports (for
 *   example `set_provider` from point 14).
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
 * @param {string} [name] the element to define.
 */
export function installFallback(name = "opengrid-table") {
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
 * @param {URL|string} options.moduleUrl wasm-bindgen glue of the engine module.
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