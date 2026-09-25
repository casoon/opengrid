// What the Svelte components take and what they refuse (plan point 79) — a
// type test, compiled by `just types` and never bundled. Written against the
// components' props: checking `.svelte` markup would take svelte-check, which
// the project does not carry.

import type { ComponentProps } from "svelte";
import { OpengridGrid, OpengridPivot, OpengridTable } from "@casoon/opengrid-svelte";
import type { View } from "@casoon/opengrid";

type GridProps = ComponentProps<typeof OpengridGrid>;

export const grid: GridProps = {
  label: "Orders",
  datasource: "orders",
  columns: "id,customer",
  windowSize: 40,
  density: "compact",
  selection: true,
  view: { sort: [{ field: "amount", direction: "desc" }] },
  onviewchange: (view: View) => view.sort,
  onselectionchange: (detail) => detail.count satisfies number,
  oncellchange: (detail) => detail.column satisfies string,
  presentation: { amount: { aggregate: "sum" } },
  class: "orders",
  id: "orders",
};
export const table: ComponentProps<typeof OpengridTable> = {
  label: "Orders",
  defaultView: { density: "compact" },
};
export const pivot: ComponentProps<typeof OpengridPivot> = { rows: "country", values: "[]" };

export const refused: GridProps[] = [
  // @ts-expect-error — a density is one of three
  { density: "tight" },
  // @ts-expect-error — the size is a number
  { windowSize: "forty" },
  // @ts-expect-error — `presentation`, not an object on `columns`
  { columns: { amount: { aggregate: "sum" } } },
  // @ts-expect-error — the view callback gets the view
  { onviewchange: (view: string) => view },
];
