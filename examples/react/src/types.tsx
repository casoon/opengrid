// What the React components take and what they refuse (plan point 77) — a
// type test, compiled by `just types` and never bundled.

import { useRef, useState } from "react";
import { OpengridGrid, OpengridPivot, OpengridTable } from "@casoon/opengrid-react";
import type { OpengridGridElement, View } from "@casoon/opengrid";

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
      <OpengridPivot datasource="orders" rows="country" columns="ordered_year" values="[]" />
      {/* @ts-expect-error — a density is one of three */}
      <OpengridGrid density="tight" />
      {/* @ts-expect-error — the size is a number */}
      <OpengridGrid windowSize="forty" />
      {/* @ts-expect-error — `presentation`, not an object on `columns` */}
      <OpengridGrid columns={{ amount: { aggregate: "sum" } }} />
      {/* @ts-expect-error — a pivot has no `groupBy` */}
      <OpengridPivot groupBy="country" />
      {/* @ts-expect-error — the callback gets the view, not an event */}
      <OpengridGrid onViewChange={(event: CustomEvent) => event.detail} />
    </>
  );
}
