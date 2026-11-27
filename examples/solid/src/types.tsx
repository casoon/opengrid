// What the Solid components take and what they refuse (#510) — a type test,
// compiled by `just types` and never bundled.

import { createSignal } from "solid-js";
import { OpengridGrid, OpengridPivot, OpengridTable } from "@casoon/opengrid/solid";
import type { OpengridGridElement, View } from "@casoon/opengrid";
import type { OpengridGridRef } from "@casoon/opengrid/solid";

/** Editing through props and the ref (issue #206). */
function Rooms() {
  let rooms!: OpengridGridRef;
  return (
    <>
      <OpengridGrid
        ref={rooms}
        rowKey="id"
        addRow
        enterMoves="down"
        actions={[{ id: "remove", label: "Remove" }]}
        validators={{ seats: () => null }}
        onRowAdd={async (detail) => ({ state: "saved", key: detail.values.id ?? null })}
        onCellChange={(detail) => rooms.setCellState(detail.key ?? null, detail.column, "saved")}
        onPaste={(detail) => detail.added.length satisfies number}
        onRowAction={(detail) => detail.id satisfies string}
      />
      {/* @ts-expect-error — `addRow` is a boolean */}
      <OpengridGrid addRow="yes" />
      {/* @ts-expect-error — `Enter` moves down or not at all */}
      <OpengridGrid enterMoves="up" />
      {/* @ts-expect-error — a row's answer is a state */}
      <OpengridGrid onRowAdd={() => ({ state: "done" })} />
    </>
  );
}

export function Page() {
  let grid!: OpengridGridElement;
  const [view, setView] = createSignal<View | null>(null);
  return (
    <>
      <OpengridGrid
        ref={grid}
        label="Orders"
        datasource="orders"
        columns="id,customer"
        windowSize={40}
        density="compact"
        selection
        view={view()}
        onViewChange={setView}
        onSelectionChange={(detail) => detail.count}
        presentation={{ amount: { aggregate: "sum" }, id: { pin: "start" } }}
        class="orders"
        id="orders"
        aria-describedby="help"
        data-testid="orders"
        hidden={false}
      />
      <OpengridGrid ref={(element) => element.setCellState(1, "name", "saved")} class={["orders", { wide: true }]} />
      <OpengridTable label="Orders" datasource="orders" columns="id" defaultView={{ density: "compact" }} />
      <Rooms />
      <OpengridPivot datasource="orders" rows="country" columns="ordered_year" values="[]" />
      <OpengridGrid filter='{"field":"country","op":"eq","value":"DE"}' />
      <OpengridGrid defaultView={{ columns: { pinned: { id: "none", amount: "end" } } }} />
      {/* @ts-expect-error — a pin names an end of the row: `start` or `end` */}
      <OpengridGrid presentation={{ id: { pin: "left" } }} />
      {/* @ts-expect-error — the filter is the wire's JSON as text, like the pivot's */}
      <OpengridGrid filter={{ field: "country", op: "eq", value: "DE" }} />
      <OpengridPivot drillDown onDrillDown={(detail) => detail.query.source satisfies string} />
      {/* @ts-expect-error — a density is one of three */}
      <OpengridGrid density="tight" />
      {/* @ts-expect-error — the size is a number */}
      <OpengridGrid windowSize="forty" />
      {/* @ts-expect-error — `presentation`, not an object on `columns` */}
      <OpengridGrid columns={{ amount: { aggregate: "sum" } }} />
      <OpengridPivot
        view={{ rows: ["country"], values: [{ fn: "count", as: "n" }] }}
        onViewChange={(view) => view.columns satisfies string[]}
      />
      {/* @ts-expect-error — a pivot's view is its rows, columns and values */}
      <OpengridPivot view={{ density: "compact" }} />
      {/* @ts-expect-error — a pivot has no `groupBy` */}
      <OpengridPivot groupBy="country" />
      {/* @ts-expect-error — the callback gets the view, not an event */}
      <OpengridGrid onViewChange={(event: CustomEvent) => event.detail} />
      {/* @ts-expect-error — Solid's class is `class`, not `className` */}
      <OpengridGrid className="orders" />
    </>
  );
}
