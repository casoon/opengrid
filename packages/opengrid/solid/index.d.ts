/**
 * Types for `@casoon/opengrid/solid` — Solid 2.
 *
 * The props are the element's attributes in camelCase plus the options of
 * `connect` from `@casoon/opengrid` — no names of their own. Everything else
 * (`id`, `class`, `style`, `aria-*`, `data-*`, `hidden`) goes to the element,
 * written the Solid way: `class`, not `className`; lowercase attribute names.
 *
 * Four things to know, from `connect` and from Solid:
 *
 * - `provider` and format functions compare by **identity**. Solid makes
 *   `provider={createRestProvider(…)}` a getter, and every read is a new
 *   provider that asks the source again — create it once, in the component
 *   or outside it.
 * - `view` is controlled: the reader's change goes to `onViewChange`, and a
 *   view the page does not take into the signal it holds is written again
 *   once the page's updates have flushed.
 * - `density` and `groupBy` are attributes the reader can change too (they are
 *   part of the view). As props they set the start; with a controlled `view`,
 *   leave them out and let the view carry them.
 * - The component is plain JavaScript. The page's own JSX is compiled by
 *   Solid's compiler (`@solidjs/vite-plugin`) with `"jsxImportSource":
 *   "@solidjs/web"` in `tsconfig.json`; the adapter needs neither.
 */

import type { Component } from "solid-js";
import type { JSX } from "@solidjs/web";
import type {
  CellWrites,
  ConnectOptions,
  Density,
  Theme,
  Mode,
  OpengridGridElement,
  OpengridPivotElement,
  OpengridTableElement,
  PivotConnectOptions,
} from "../loader.js";

/** What the element itself takes from Solid, minus what the component owns. */
type ElementProps = Omit<
  JSX.HTMLAttributes<HTMLElement>,
  "children" | "ref" | keyof ConnectOptions
>;

/**
 * Solid's `ref`: the element — a variable declared `let grid!: …` that
 * `ref={grid}` fills — or a function that gets it, or a list of those.
 */
type ElementRef<Element, Given = Element> =
  | Element
  | ((element: Given) => void)
  | undefined
  | ElementRef<Element, Given>[];

export interface OpengridGridProps extends ConnectOptions, ElementProps {
  /** The element — `let grid!: OpengridGridRef`, `ref={grid}` — or a function that gets it. */
  ref?: ElementRef<OpengridGridElement, OpengridGridRef>;
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
  /** Up to three columns, outermost first: `"country,customer"`. */
  groupBy?: string;
  /** The field holding a row's parent key: the rows show as a tree. */
  tree?: string;
  /** The field the parent key refers to; `id` when absent. */
  treeKey?: string;
  /** The field that names a record: `key` in the editing events. */
  rowKey?: string;
  /** The boolean field that mutes a row: drawn in the muted ink, described with the text `rowMuted`. */
  rowMuted?: string;
  /** The page's filter, as the wire's JSON; and-ed onto every query, not in the view. */
  filter?: string;
  /** The locale text columns sort by (`de`, `sr-Latn`); by code point without it. */
  collation?: string;
  /** An empty row after the last one to type a new record into (`onRowAdd`). */
  addRow?: boolean;
  /** Locks editing for the whole grid; the page unlocks it at run time. */
  editLocked?: boolean;
  /** An *Edit* switch in the toolbar that locks and unlocks editing (needs `toolbar`). */
  editToggle?: boolean;
  /** `"down"`: `Enter` that commits an editor moves the focus to the next data row (`Shift`+`Enter`: the previous one). */
  enterMoves?: "down";
  search?: boolean;
  facets?: boolean;
  toolbar?: boolean;
  columnMenu?: boolean;
  /** *Advanced filter* in the toolbar: the builder of the nested filter. */
  filterBuilder?: boolean;
  selection?: boolean;
  density?: Density;
  /** One of the built-in looks; Base when absent. */
  theme?: Theme;
}

export interface OpengridTableProps extends ConnectOptions, ElementProps {
  ref?: ElementRef<OpengridTableElement>;
  label?: string;
  datasource?: string;
  columns?: string;
  theme?: Theme;
}

/** A pivot's view is a `PivotView`: its `rows`, `columns` and `values`. */
export interface OpengridPivotProps extends PivotConnectOptions, ElementProps {
  ref?: ElementRef<OpengridPivotElement>;
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
  /** Makes every value a button that calls `onDrillDown`. */
  drillDown?: boolean;
  theme?: Theme;
}

/**
 * What the ref holds: the element, carrying the connection's `setCellState`
 * and `setValues` — `let grid!: OpengridGridRef`. The prop takes a variable of
 * the plain element type too.
 */
export type OpengridGridRef = OpengridGridElement & CellWrites;

export const OpengridGrid: Component<OpengridGridProps>;
export const OpengridTable: Component<OpengridTableProps>;
export const OpengridPivot: Component<OpengridPivotProps>;
