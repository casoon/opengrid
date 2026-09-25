/**
 * Types for `@casoon/opengrid-react` (plan point 77).
 *
 * The props are the element's attributes in camelCase plus the options of
 * `connect` from `@casoon/opengrid` — no names of their own. Everything else
 * (`id`, `className`, `style`, `aria-*`, `data-*`) goes to the element.
 */

import type { ForwardRefExoticComponent, HTMLAttributes, RefAttributes } from "react";
import type {
  ConnectOptions,
  Density,
  Mode,
  OpengridGridElement,
  OpengridPivotElement,
  OpengridTableElement,
} from "@casoon/opengrid";

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
}

export interface OpengridTableProps extends ConnectOptions, ElementProps {
  label?: string;
  datasource?: string;
  columns?: string;
}

export interface OpengridPivotProps extends ConnectOptions, ElementProps {
  label?: string;
  datasource?: string;
  /** Comma-separated row dimensions, outermost first. */
  rows?: string;
  /** Comma-separated column dimensions; V1 allows one. */
  columns?: string;
  /** The measures as the contract's JSON. */
  values?: string;
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
