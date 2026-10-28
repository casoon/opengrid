// What the React components take and what they refuse (plan point 77) — a
// type test, compiled by `just types` and never bundled.

import { useRef, useState } from "react";
import { OpengridGrid, OpengridPivot, OpengridTable } from "@casoon/opengrid/react";
import type { OpengridGridElement, View } from "@casoon/opengrid";
import type { OpengridGridRef } from "@casoon/opengrid/react";

/** Editing through props and the ref (issue #206). */
function Rooms() {
  const rooms = useRef<OpengridGridRef>(null);
  return (
    <>
      <OpengridGrid
        ref={rooms}
        rowKey="id"
        addRow
        actions={[{ id: "remove", label: "Remove" }]}
        validators={{ seats: () => null }}
        onRowAdd={async (detail) => ({ state: "saved", key: detail.values.id ?? null })}
        onCellChange={(detail) => rooms.current?.setCellState(detail.key ?? null, detail.column, "saved")}
        onPaste={(detail) => detail.added.length satisfies number}
        onRowAction={(detail) => detail.id satisfies string}
      />
      {/* @ts-expect-error — `addRow` is a boolean */}
      <OpengridGrid addRow="yes" />
      {/* @ts-expect-error — a row's answer is a state */}
      <OpengridGrid onRowAdd={() => ({ state: "done" })} />
    </>
  );
}

export function Page() {
  const grid = useRef<OpengridGridElement>(null);
  const [view, setView] = useState<View | null>(null);
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
        view={view}
        onViewChange={setView}
        onSelectionChange={(detail) => detail.count}
        presentation={{ amount: { aggregate: "sum" } }}
        className="orders"
        id="orders"
        aria-describedby="help"
      />
      <OpengridTable label="Orders" datasource="orders" columns="id" defaultView={{ density: "compact" }} />
      <Rooms />
      <OpengridPivot datasource="orders" rows="country" columns="ordered_year" values="[]" />
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
    </>
  );
}
