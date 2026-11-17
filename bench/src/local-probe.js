import { createLocalProvider, createWorkerProvider, loadOpengrid } from "@casoon/opengrid";
import { makeRows } from "./data.js";
const N = Number(new URLSearchParams(location.search).get("rows") ?? 1_000_000);
const SCHEMA = JSON.stringify({ fields: [
  { name: "id", type: "int64", nullable: false }, { name: "customer", type: "utf8", nullable: true },
  { name: "country", type: "utf8", nullable: true }, { name: "amount", type: "float64", nullable: true },
  { name: "qty", type: "int64", nullable: true }, { name: "ordered_on", type: "date", nullable: true }] });
const out = {};
const rows = makeRows(N);
let t = performance.now();
await loadOpengrid();
out.loadModule = performance.now() - t;
const lines = ["id,customer,country,amount,qty,ordered_on"];
for (const r of rows) lines.push(`${r.id},${r.customer},${r.country},${r.amount},${r.qty},${r.ordered_on}`);
const bytes = new TextEncoder().encode(lines.join("\n"));
for (const route of ["rows", "csv", "rows", "csv"]) {
  const provider = createWorkerProvider();
  t = performance.now();
  if (route === "rows") await provider.loadRows("orders", rows, SCHEMA);
  else await provider.load("orders", bytes, SCHEMA);
  out[route + "_" + (out[route] ? "second" : "first")] = performance.now() - t;
  out[route] = true;
  provider.terminate();
}
{
  let packs = 0;
  const fake = { begin_columns() {}, push_columns() { packs++; }, end_columns() {}, abort_columns() {} };
  const provider = createLocalProvider(fake);
  for (let i = 0; i < 2; i++) {
    t = performance.now();
    await provider.loadRows("orders", rows, SCHEMA);
    out["pack_only_" + i] = performance.now() - t;
  }
}
window.__probe = out;
