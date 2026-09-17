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