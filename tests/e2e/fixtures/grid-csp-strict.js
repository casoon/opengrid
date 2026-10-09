// The script of grid-csp-strict.html: a file of its own, because that page's
// policy refuses every inline script.
import { loadOpengrid, createLocalProvider } from "/packages/opengrid/loader.js";
import init, { Engine } from "/examples/engine-demo/pkg/opengrid_wasm.js";

window.__changes = [];
document.addEventListener("opengrid-cell-change", (event) => window.__changes.push(event.detail));

await init();
const engine = new Engine();
const schema = await (await fetch("/crates/opengrid-conformance/data/orders.schema.json")).text();
const csv = await (await fetch("/tests/e2e/fixtures/grid-virtual.csv")).arrayBuffer();
engine.load_csv("orders", new Uint8Array(csv), schema);

const loader = await loadOpengrid();
const host = document.querySelector("opengrid-grid");
// What a marks sheet needs: a key the page owns, a column with a fixed list,
// and fixed widths.
loader.module.set_columns(host, {
  id: { readonly: true, width: 140 },
  customer: { width: 200 },
});
loader.module.set_choices(host, { customer: ["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta"] });
loader.module.set_provider(host, createLocalProvider(engine));
window.__ready = true;
