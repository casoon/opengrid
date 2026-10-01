/**
 * Types for `@casoon/opengrid/svelte` (plan point 79).
 *
 * The props are the element's attributes in camelCase plus the options of
 * `connect` from `@casoon/opengrid`, with the callbacks in Svelte's own
 * spelling — `onviewchange`, `onselectionchange`, `oncellchange` — `view`
 * bindable (`bind:view`), and the element itself through `bind:element`.
 * Everything else (`id`, `class`, `style`, `aria-*`) goes to the element.
 *
 * `view` is controlled through `bind:view`: a binding whose setter refuses the
 * reader's change gets its view written back. A `view={…}` without `bind:`
 * follows Svelte's rule for bindable props — the reader's change becomes the
 * component's own value, as on an `<input>` without `bind:`.
 */

import type { Component } from "svelte";
import type { HTMLAttributes } from "svelte/elements";
import type {
  CellChangeDetail,
  ConnectOptions,
  Density,
  Theme,
  Mode,
  PivotConnectOptions,
  PivotView,
  SelectionChangeDetail,
  View,
} from "../loader.js";

/** `connect`'s options, the callbacks in Svelte's spelling. */
type Options = Omit<ConnectOptions, "onViewChange" | "onSelectionChange" | "onCellChange"> & {
  /** The element, for `bind:element`. */
  element?: HTMLElement;
  /** The reader changed the view; `bind:view` has it already. */
  onviewchange?: (view: View) => void;
  onselectionchange?: (detail: SelectionChangeDetail) => void;
  oncellchange?: (detail: CellChangeDetail) => void;
};

/** What the element itself takes, minus what the component owns. */
type ElementProps = Omit<HTMLAttributes<HTMLElement>, "children" | keyof Options>;

export interface OpengridGridProps extends Options, ElementProps {
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
  /** One of the built-in looks; Base when absent. */
  theme?: Theme;
}

export interface OpengridTableProps extends Options, ElementProps {
  label?: string;
  datasource?: string;
  columns?: string;
  theme?: Theme;
}

/** The pivot's options: its view is a `PivotView`. */
type PivotOptions = Omit<
  PivotConnectOptions,
  "onViewChange" | "onSelectionChange" | "onCellChange"
> & {
  element?: HTMLElement;
  onviewchange?: (view: PivotView) => void;
  onselectionchange?: (detail: SelectionChangeDetail) => void;
  oncellchange?: (detail: CellChangeDetail) => void;
};

export interface OpengridPivotProps extends PivotOptions, ElementProps {
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

export const OpengridGrid: Component<OpengridGridProps, {}, "view" | "element">;
export const OpengridTable: Component<OpengridTableProps, {}, "view" | "element">;
export const OpengridPivot: Component<OpengridPivotProps, {}, "view" | "element">;
