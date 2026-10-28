/**
 * Types for `@casoon/opengrid/vue` (plan point 78).
 *
 * The props are the element's attributes in camelCase plus the options of
 * `connect` from `@casoon/opengrid`; the callbacks are Vue events instead:
 * `update:view` (so `v-model:view` works), `selectionChange`, `cellChange`,
 * `paste`, `bulkChange`, `rowAction` and `sourceReplaced` — `@selection-change`
 * and so on in a template. `onRowAdd` stays a prop, because its answer is the
 * new row's state; a template writes it as `@row-add` all the same.
 */

import type { DefineSetupFnComponent } from "vue";
import type {
  BulkChangeDetail,
  CellChangeDetail,
  CellWrites,
  ConnectOptions,
  Density,
  Theme,
  Mode,
  PivotConnectOptions,
  OpengridGridElement,
  PasteDetail,
  PivotView,
  RowActionDetail,
  SelectionChangeDetail,
  SourceReplacedDetail,
  View,
} from "../loader.js";

/** The callbacks of `connect` that are events here. */
type Callbacks =
  | "onViewChange"
  | "onSelectionChange"
  | "onCellChange"
  | "onPaste"
  | "onBulkChange"
  | "onRowAction"
  | "onSourceReplaced";

/** `connect`'s options, less the callbacks that are events. */
type Options = Omit<ConnectOptions, Callbacks>;

/** A type, not an interface: Vue's `EmitsOptions` wants an index signature. */
export type OpengridEmits = {
  /** The reader changed the view; `v-model:view` writes it back. */
  "update:view": (view: View) => void;
  selectionChange: (detail: SelectionChangeDetail) => void;
  cellChange: (detail: CellChangeDetail) => void;
  paste: (detail: PasteDetail) => void;
  bulkChange: (detail: BulkChangeDetail) => void;
  rowAction: (detail: RowActionDetail) => void;
  sourceReplaced: (detail: SourceReplacedDetail) => void;
};

/**
 * What a template ref on `OpengridGrid` holds: the connection's `setCellState`
 * and `setValues`, and the element as `$el` — `ref<OpengridGridExposed | null>(null)`.
 */
export type OpengridGridExposed = CellWrites & { $el: OpengridGridElement };

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
  /** The field holding a row's parent key: the rows show as a tree. */
  tree?: string;
  /** The field the parent key refers to; `id` when absent. */
  treeKey?: string;
  /** The field that names a record: `key` in the editing events. */
  rowKey?: string;
  /** An empty row after the last one to type a new record into (`@row-add`). */
  addRow?: boolean;
  search?: boolean;
  facets?: boolean;
  toolbar?: boolean;
  columnMenu?: boolean;
  selection?: boolean;
  density?: Density;
  /** One of the built-in looks; Base when absent. */
  theme?: Theme;
}

export interface OpengridTableProps extends Options {
  label?: string;
  datasource?: string;
  columns?: string;
  theme?: Theme;
}

/** The pivot's events: its view is a `PivotView`. */
export type OpengridPivotEmits = Omit<OpengridEmits, "update:view"> & {
  "update:view": (view: PivotView) => void;
};

export interface OpengridPivotProps
  extends Omit<PivotConnectOptions, Callbacks> {
  label?: string;
  datasource?: string;
  /** Comma-separated row dimensions, outermost first. */
  rows?: string;
  /** Comma-separated column dimensions, outermost first; V1 allows two. */
  columns?: string;
  /** The measures as the contract's JSON. */
  values?: string;
  /** How each level is ordered, as the wire's JSON; part of the view. */
  sort?: string;
  /** The folded groups, as a JSON list of group paths; part of the view. */
  collapsed?: string;
  /** Shows the field toolbar. */
  toolbar?: boolean;
  /** The fields a reader may pivot by, comma-separated. */
  fields?: string;
  /** The measures a reader may add, as the contract's JSON. */
  measures?: string;
  /** The filter on the raw rows, as the wire's JSON; part of the view. */
  filter?: string;
  theme?: Theme;
}

export const OpengridGrid: DefineSetupFnComponent<OpengridGridProps, OpengridEmits>;
export const OpengridTable: DefineSetupFnComponent<OpengridTableProps, OpengridEmits>;
export const OpengridPivot: DefineSetupFnComponent<OpengridPivotProps, OpengridPivotEmits>;
