// Scenario for `peakmark check`: load a CSV into the engine, query it and pivot it, then let go.
// The memory it needs is the number the engine's budget guards.
import { readFileSync } from "node:fs";

const data = new URL("../../crates/opengrid-conformance/data/", import.meta.url);
const schema = readFileSync(new URL("orders.schema.json", data), "utf8");
const ROWS = 100_000;

function csv() {
  const countries = ["DE", "FR", "PL", "NL", "ES", "IT", "AT", "CH"];
  const lines = ["id,customer,country,amount,qty,ratio,flag,ordered_on,created_at,note"];
  for (let i = 1; i <= ROWS; i++) {
    const day = String((i % 28) + 1).padStart(2, "0");
    lines.push(
      `${i},Customer ${i % 500},${countries[i % 8]},${(i % 997) + 0.25},${i % 40},` +
        `${(i % 100) / 10},${i % 2 === 0},2026-03-${day},2026-03-${day}T12:00:00Z,`,
    );
  }
  return new TextEncoder().encode(lines.join("\n"));
}

export default async function (engine) {
  const orders = new engine.Engine();
  orders.load_csv("orders", csv(), schema);
  orders.execute(
    JSON.stringify({
      source: "orders",
      select: ["country", "customer", "amount"],
      sort: [{ field: "amount", direction: "desc" }],
      limit: 100,
    }),
  );
  orders.pivot(
    JSON.stringify({
      source: "orders",
      rows: ["country", "customer"],
      columns: [],
      values: [{ field: "amount", fn: "sum", as: "total" }, { fn: "count", as: "n" }],
    }),
  );
  orders.free();
}
