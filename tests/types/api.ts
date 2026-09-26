// The public API, used once each the way docs/api.md shows it (plan point 75).
//
// A type test: `tsc` compiles it and nothing runs. The lines marked
// `@ts-expect-error` are the other half — forms the API does not take. If one
// of them compiles, the directive itself is the error, so a type that went
// loose fails here as surely as one that went wrong.

import {
  connect,
  createHybridProvider,
  createLocalProvider,
  createPivotProvider,
  createRestProvider,
  createWorkerProvider,
  loadOpengrid,
  type CellChangeDetail,
  type Engine,
  type PlannerLike,
  type Provider,
  type SelectionChangeDetail,
  type View,
} from "@casoon/opengrid";

declare const engine: Engine;
declare const planner: PlannerLike;

export async function page(): Promise<void> {
  const loader = await loadOpengrid();
  if (loader.fallback) {
    // @ts-expect-error — no module without WASM
    loader.module.set_view(document.body, {});
    return;
  }
  const { module } = loader;

  const grid = document.querySelector("opengrid-grid");
  const table = document.createElement("opengrid-table");
  const pivot = document.createElement("opengrid-pivot");
  if (!grid) return;

  // Providers: every one fits `set_provider`.
  const local = createLocalProvider(engine);
  await local.load("orders", new Uint8Array(), "{}");
  const worker = createWorkerProvider({ moduleUrl: new URL("./pkg/opengrid_wasm.js", location.href).href });
  worker.terminate();
  // @ts-expect-error — a URL object cannot cross postMessage; the href can
  createWorkerProvider({ moduleUrl: new URL("./pkg/opengrid_wasm.js", location.href) });
  // @ts-expect-error — the package ships no engine module, so there is no default
  createWorkerProvider({});
  // @ts-expect-error — the tab's provider reads a view other than bytes wrongly
  await local.load("orders", new DataView(new ArrayBuffer(0)), "{}");
  const rest = createRestProvider({ url: "https://example.test", source: "orders", token: "t" });
  const described = await rest.describe();
  described.name satisfies string;
  const hybrid = createHybridProvider({
    remote: rest,
    planner,
    mode: "auto",
    onPlan: (plan) => plan.describe satisfies string,
  });
  const custom: Provider = { execute: async (query) => query };
  for (const provider of [local, worker, rest, hybrid, custom, createPivotProvider({ url: "/", source: "o" })]) {
    module.set_provider(grid, provider);
  }
  module.set_provider(table, local);
  module.set_provider(pivot, local);
  // @ts-expect-error — a provider has an `execute`
  module.set_provider(grid, { run: () => "" });
  // @ts-expect-error — `mode` is one of four
  createHybridProvider({ remote: rest, planner, mode: "server" });

  // Texts: any subset, `lang`, and the operators by wire token.
  module.set_texts(grid, { lang: "de", loading: "Wird geladen …", operators: { gte: "größer gleich" } });
  // @ts-expect-error — not a text key
  module.set_texts(grid, { loadnig: "typo" });
  // @ts-expect-error — not an operator
  module.set_texts(grid, { operators: { between: "zwischen" } });

  // Formats: a function or `Intl` options with a kind.
  module.set_formats(grid, {
    amount: { kind: "number", locale: "de-DE", style: "currency", currency: "EUR" },
    ordered_on: { kind: "date", locale: "de-DE", dateStyle: "medium" },
    qty: (text, value) => `${value ?? text} pcs`,
  });
  // @ts-expect-error — a format answers text
  module.set_formats(grid, { qty: (text: string) => text.length });

  module.set_choices(grid, { customer: ["Alpha", "Beta"] });

  // Presentation.
  module.set_columns(grid, {
    id: { width: 96, mono: true, muted: true },
    amount: { align: "end", aggregate: "sum", facet: "range" },
    ordered_on: { aggregate: "range", facet: "period" },
  });
  // @ts-expect-error — not an alignment
  module.set_columns(grid, { amount: { align: "right" } });
  // @ts-expect-error — not an aggregate
  module.set_columns(grid, { amount: { aggregate: "median" } });

  // The view: read whole, written in parts.
  // @ts-expect-error — a grid that was never connected has no view
  void module.get_view(grid).sort;
  const view = module.get_view(grid);
  if (view) {
    view.sort[0]?.direction satisfies "asc" | "desc" | undefined;
    view.columns.widths["amount"] satisfies number | undefined;
    view.filterRow satisfies boolean;
    module.set_view(grid, view);
  }
  module.set_view(grid, {
    sort: [{ field: "amount", direction: "desc" }],
    filters: [{ column: "country", op: "eq", value: "DE" }],
    columns: { hidden: ["qty"] },
    density: "compact",
    group: ["country"],
    expanded: [["DE"], [null]],
    aggregates: { amount: "sum" },
    facets: { customer: { values: ["Alpha", null] }, amount: { min: "5", max: "" } },
  });
  // @ts-expect-error — a direction is `asc` or `desc`
  module.set_view(grid, { sort: [{ field: "amount", direction: "down" }] });
  // @ts-expect-error — a filter names its column, not its field
  module.set_view(grid, { filters: [{ field: "country", op: "eq", value: "DE" }] });
  // @ts-expect-error — not a density
  module.set_view(grid, { density: "tight" });
  const wrongShape: Partial<View> = {
    // @ts-expect-error — the widths are a map, not a list
    columns: { order: [], hidden: [], widths: [120] },
  };
  void wrongShape;

  // Events: the detail is typed on the element and on the document.
  grid.addEventListener("opengrid-selection-change", (event) => {
    event.detail satisfies SelectionChangeDetail;
  });
  grid.addEventListener("opengrid-cell-change", (event) => {
    event.detail satisfies CellChangeDetail;
    event.detail.previous satisfies string;
  });
  document.addEventListener("opengrid-view-change", (event) => {
    event.detail.view.density satisfies View["density"];
  });
  grid.addEventListener("opengrid-selection-change", (event) => {
    // @ts-expect-error — a selection has rows, not cells
    void event.detail.cells;
  });

  module.register();

  // The query of the view, for an export.
  const query = module.get_query(grid);
  if (query) {
    query.select satisfies string[];
    query.sort?.[0]?.direction satisfies "asc" | "desc" | undefined;
    // @ts-expect-error — the view's query has no window
    void query.limit;
  }

  // The pivot as it is shown, as CSV.
  const csv = module.get_pivot(pivot, { delimiter: ";", bom: false, null: "\\N" });
  csv satisfies string | null;
  module.get_pivot(pivot);
  // @ts-expect-error — a misspelt option is not an option
  module.get_pivot(pivot, { delimeter: ";" });
  // @ts-expect-error — `bom` is a boolean, not the text of one
  module.get_pivot(pivot, { bom: "false" });

  // `connect`: the same shapes, from one object.
  const connection = connect(grid, {
    provider: local,
    texts: { lang: "de" },
    presentation: { amount: { aggregate: "sum" } },
    view: { sort: [{ field: "amount", direction: "desc" }] },
    onViewChange: (next) => connection.update({ view: next }),
    onSelectionChange: (detail) => detail.rows satisfies number[],
    onCellChange: (detail) => detail.column satisfies string,
  });
  (await connection.ready).fallback satisfies boolean;
  connection.update({ texts: undefined, view: null });
  connection.update();
  connect(table, { provider: local, defaultView: { density: "compact" } }).disconnect();
  // @ts-expect-error — `presentation`, not `columns`: that is the attribute
  connection.update({ columns: { amount: { aggregate: "sum" } } });
  // @ts-expect-error — the callback gets the view, not the event
  connect(grid, { onViewChange: (event: CustomEvent) => event.detail });
  connection.disconnect();
}
