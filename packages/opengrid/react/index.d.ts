/**
 * Types for `@casoon/opengrid/react` (plan point 77).
 *
 * The props are the element's attributes in camelCase plus the options of
 * `connect` from `@casoon/opengrid` — no names of their own. Everything else
 * (`id`, `className`, `style`, `aria-*`, `data-*`) goes to the element.
 *
 * Three things to know, from `connect`:
 *
 * - `provider` and format functions compare by **identity**. An inline
 *   `provider={createRestProvider(…)}` is a new provider on every render, and
 *   each one asks the source again — create it once (`useMemo`, or outside the
 *   component).
 * - `view` is controlled at **render**: a view the page does not take back
 *   from `onViewChange` is written again the next time the component renders.
 * - `density` and `groupBy` are attributes the reader can change too (they are
 *   part of the view). As props they set the start; with a controlled `view`,
 *   leave them out and let the view carry them.
 */

import type { ForwardRefExoticComponent, HTMLAttributes, RefAttributes } from "react";
import type {
  ConnectOptions,
  Density,
  Theme,
  Mode,
  OpengridGridElement,
  OpengridPivotElement,
  OpengridTableElement,
  PivotConnectOptions,
} from "../loader.js";

/** What the element itself takes from React, minus what the component owns. */
type ElementProps = Omit<
  HTMLAttributes<HTMLElement>,
  "children" | keyof ConnectOptions
>;

export interface OpengridGridProps extends ConnectOptions, ElementProps {
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

export interface OpengridTableProps extends ConnectOptions, ElementProps {
  label?: string;
  datasource?: string;
  columns?: string;
  theme?: Theme;
}

/** A pivot's view is a `PivotView`: its `rows`, `columns` and `values`. */
export interface OpengridPivotProps extends PivotConnectOptions, ElementProps {
  label?: string;
  datasource?: string;
  /** Comma-separated row dimensions, outermost first. */
  rows?: string;
  /** Comma-separated column dimensions; V1 allows one. */
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

export const OpengridGrid: ForwardRefExoticComponent<
  OpengridGridProps & RefAttributes<OpengridGridElement>
>;
export const OpengridTable: ForwardRefExoticComponent<
  OpengridTableProps & RefAttributes<OpengridTableElement>
>;
export const OpengridPivot: ForwardRefExoticComponent<
  OpengridPivotProps & RefAttributes<OpengridPivotElement>
>;
