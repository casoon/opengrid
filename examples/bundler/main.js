// The smallest page that uses the engine in a worker, built for production
// with nothing set (#238): no `moduleUrl`, no `workerUrl`, no served `pkg/`.
// `tests/e2e/bundler-production.spec.js` loads the build.
import { createWorkerProvider, loadOpengrid } from "@casoon/opengrid";

const SCHEMA = JSON.stringify({
  fields: [
    { name: "id", type: "int64", nullable: false },
    { name: "name", type: "utf8", nullable: false },
    { name: "seats", type: "int64", nullable: true },
  ],
});
const CSV = "id,name,seats\n1,A1,10\n2,B2,20\n3,C3,30\n";

const { fallback, module } = await loadOpengrid();
window.__fallback = fallback;

const provider = createWorkerProvider();
await provider.load("rooms", new TextEncoder().encode(CSV), SCHEMA);
module.set_provider(document.querySelector("opengrid-grid"), provider);
