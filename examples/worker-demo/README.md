# Worker-Demo (100k)

Manuelle Demo für die Engine im Web Worker (Plan-Punkt 19): dieselbe Oberfläche
wie die Grid-Demo, aber die lokale WASM-Engine läuft in einem Modul-Worker. Der
Main Thread behält DOM, Events, Tastatur und Rendering; Ingest, Filter,
Sortierung und Gruppierung laufen im Worker. Die Seite nutzt dafür
`createWorkerProvider` aus `packages/opengrid/loader.js` — derselbe Provider-Seam
wie der Main-Thread-Pfad (`createLocalProvider`), die Grid-Elemente bleiben
unverändert.

## Starten

```console
just wasm-build            # Engine-Modul (einmalig, oder nach Engine-Änderungen)
just wasm-build-components # Element-Modul (nach Änderungen an den Komponenten)
just serve-demo            # Server-Wurzel ist das Repo
```

Dann <http://127.0.0.1:8080/examples/worker-demo/> öffnen.

## Optional: 100 000 Zeilen

Ohne Datensatz lädt die Demo den kleinen Conformance-Datensatz. Der 100k-Datensatz
liegt unter einem gitignorierten Pfad (`target/`) und wird nicht eingecheckt:

```console
mkdir -p target/worker-demo
cargo run -p xtask -- gen-orders --rows 100000 --seed 1 --out target/worker-demo/orders-100k.csv
```

## Worker oder Main Thread

Beide Pfade sehen für die Seite gleich aus:

```js
// Worker
const provider = createWorkerProvider({
  moduleUrl: "/examples/engine-demo/pkg/opengrid_wasm.js",
  wasmUrl: "/examples/engine-demo/pkg/opengrid_wasm_bg.wasm",
});
await provider.load("orders", csv, schema);
loader.module.set_provider(host, provider);

// Main-Thread-Fallback
const provider = createLocalProvider(engine);
await provider.load("orders", csv, schema);
loader.module.set_provider(host, provider);
```

Genau **ein** Worker in V1 (kein Pool, kein SharedArrayBuffer). Das
Nachrichtenprotokoll steht in `packages/opengrid/worker.js`; die Messung zur
Responsivität liegt in `tests/e2e/worker.spec.js` und ist in
`plan/spezifikation/12-qualitaet.md` notiert.
