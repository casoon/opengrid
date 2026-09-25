/**
 * Types for `@casoon/opengrid-vue` (plan point 78).
 *
 * The props are the element's attributes in camelCase plus the options of
 * `connect` from `@casoon/opengrid`; the callbacks are Vue events instead:
 * `update:view` (so `v-model:view` works), `selectionChange` and `cellChange`
 * — `@selection-change` and `@cell-change` in a template.
 */

import type { DefineSetupFnComponent } from "vue";
import type {
  CellChangeDetail,
  ConnectOptions,
  Density,
  Mode,
  SelectionChangeDetail,
  View,
} from "@casoon/opengrid";

/** `connect`'s options, less the callbacks — those are events here. */
type Options = Omit<ConnectOptions, "onViewChange" | "onSelectionChange" | "onCellChange">;

/** A type, not an interface: Vue's `EmitsOptions` wants an index signature. */
export type OpengridEmits = {
  /** The reader changed the view; `v-model:view` writes it back. */
  "update:view": (view: View) => void;
  selectionChange: (detail: SelectionChangeDetail) => void;
  cellChange: (detail: CellChangeDetail) => void;
};

export interface OpengridGridProps extends Options {
  /** The accessible name of the table. */
  label?: string;
  /** The source; becomes the query's `source`. */
  datasource?: string;
  /** Comma-separated output fields, in order. */
  columns?: string;
  /** Rows rendered and fetched at once while scrolling. Default 40. */
  windowSize?: number;
  /** Switches from scrolling to paging. */
  pageSize?: number;
  mode?: Mode;
  /** Up to two columns, outermost first: `"country,customer"`. */
  groupBy?: string;
  search?: boolean;
  facets?: boolean;
  toolbar?: boolean;
  columnMenu?: boolean;
  selection?: boolean;
  density?: Density;
}

export interface OpengridTableProps extends Options {
  label?: string;
  datasource?: string;
  columns?: string;
}

export interface OpengridPivotProps extends Options {
  label?: string;
  datasource?: string;
  /** Comma-separated row dimensions, outermost first. */
  rows?: string;
  /** Comma-separated column dimensions; V1 allows one. */
  columns?: string;
  /** The measures as the contract's JSON. */
  values?: string;
}

export const OpengridGrid: DefineSetupFnComponent<OpengridGridProps, OpengridEmits>;
export const OpengridTable: DefineSetupFnComponent<OpengridTableProps, OpengridEmits>;
export const OpengridPivot: DefineSetupFnComponent<OpengridPivotProps, OpengridEmits>;
