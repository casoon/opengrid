// What the Vue components take and what they refuse (plan point 78) — a type
// test, compiled by `just types` and never bundled. Written with `h()`:
// checking templates would take vue-tsc, which the project does not carry.

import { h } from "vue";
import { OpengridGrid, OpengridPivot, OpengridTable } from "@casoon/opengrid-vue";
import type { View } from "@casoon/opengrid";

let view: View | null = null;

export const nodes = [
  h(OpengridGrid, {
    label: "Orders",
    datasource: "orders",
    columns: "id,customer",
    windowSize: 40,
    density: "compact",
    selection: true,
    view,
    "onUpdate:view": (next: View) => {
      view = next;
    },
    onSelectionChange: (detail) => detail.count satisfies number,
    onCellChange: (detail) => detail.column satisfies string,
    presentation: { amount: { aggregate: "sum" } },
  }),
  h(OpengridTable, { label: "Orders", datasource: "orders", columns: "id", defaultView: { density: "compact" } }),
  h(OpengridPivot, { datasource: "orders", rows: "country", columns: "ordered_year", values: "[]" }),
  // @ts-expect-error — a density is one of three
  h(OpengridGrid, { density: "tight" }),
  // @ts-expect-error — the size is a number
  h(OpengridGrid, { windowSize: "forty" }),
  // @ts-expect-error — `presentation`, not an object on `columns`
  h(OpengridGrid, { columns: { amount: { aggregate: "sum" } } }),
  // @ts-expect-error — the view event carries the view
  h(OpengridGrid, { "onUpdate:view": (next: string) => next }),
];
