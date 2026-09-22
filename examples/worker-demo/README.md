# Worker demo (100k)

The engine in a Web Worker: the same interface as the grid demo, but the local
WASM engine runs in a module worker. The main thread keeps the DOM, the events,
the keyboard and the rendering; ingest, filtering, sorting and grouping happen in
the worker. The page uses `createWorkerProvider` from
`packages/opengrid/loader.js` — the same provider seam as the main-thread path
(`createLocalProvider`), so the elements themselves are unchanged.

## Running it

```console
just wasm-build            # engine module (once, or after engine changes)
just wasm-build-components # element module (after component changes)
just serve-demo            # the server root is the repository
```

Then open <http://127.0.0.1:8080/examples/worker-demo/>.

## Optional: 100,000 rows

Without the data set the demo falls back to the small conformance one. The 100k
file lives under a gitignored path (`target/`) and is not checked in:

```console
mkdir -p target/worker-demo
cargo run -p xtask -- gen-orders --rows 100000 --seed 1 --out target/worker-demo/orders-100k.csv
```

## Worker or main thread

Both paths look the same from the page:

```js
// Worker
const provider = createWorkerProvider({
  moduleUrl: "/examples/engine-demo/pkg/opengrid_wasm.js",
  wasmUrl: "/examples/engine-demo/pkg/opengrid_wasm_bg.wasm",
});
await provider.load("orders", csv, schema);
loader.module.set_provider(host, provider);

// Main-thread fallback
const provider = createLocalProvider(engine);
await provider.load("orders", csv, schema);
loader.module.set_provider(host, provider);
```

Exactly **one** worker in V1 — no pool, no SharedArrayBuffer. The message
protocol is in `packages/opengrid/worker.js`, and the responsiveness measurement
is in `tests/e2e/worker.spec.js`.
