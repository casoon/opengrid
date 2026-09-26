/// <reference lib="dom" />
/**
 * Types for `@casoon/opengrid` (plan point 75).
 *
 * Written by hand after `docs/api.md`, which is the source: everything a page
 * can rely on is typed here, and nothing else. The freeze test in
 * `crates/opengrid-web-components/src/api.rs` fails when a frozen name is
 * missing from this file, so the two cannot drift apart silently.
 *
 * The module functions (`set_provider`, `set_view`, …) are reached through
 * `loadOpengrid()`: `(await loadOpengrid()).module`.
 */

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

export interface LoadOptions {
  /** The wasm-bindgen glue of the element module; defaults to the packaged one. */
  moduleUrl?: URL | string;
  /** An explicit `.wasm` URL, if it is not next to the glue. */
  wasmUrl?: URL | string;
}

/**
 * What `loadOpengrid()` answers. `fallback` is `true` when the WebAssembly
 * module could not be loaded and a plain-DOM stand-in was installed instead —
 * then there is no module to call.
 */
export type LoadResult =
  | { fallback: false; module: OpengridModule }
  | { fallback: true; module?: undefined };

/** Loads the WASM module and registers the three elements; runs once. */
export function loadOpengrid(options?: LoadOptions): Promise<LoadResult>;

/** The functions of the element module. */
export interface OpengridModule {
  /** Attaches the data source. Call {@link OpengridModule.set_texts} first. */
  set_provider(host: HTMLElement, provider: Provider): void;
  /** Overrides any subset of the texts; call it before `set_provider`. */
  set_texts(host: HTMLElement, texts: Texts): void;
  /** Per-column display formatting. Never reaches a query. */
  set_formats(host: HTMLElement, formats: Formats): void;
  /** Per-column editor choices: `{ customer: ["Alpha", "Beta"] }`. */
  set_choices(host: HTMLElement, choices: Choices): void;
  /** The whole view as one value; `null` before the grid is connected. */
  get_view(host: HTMLElement): View | null;
  /** Applies a view in one step and one query. Parts left out keep their default. */
  set_view(host: HTMLElement, view: ViewInput): void;
  /** Per-column presentation; narrows what the schema allows, never widens it. */
  set_columns(host: HTMLElement, columns: Columns): void;
  /**
   * The query of the current view without a window — what an export sends.
   * `null` without a query, with a filter that does not hold, and for the
   * table and the pivot.
   */
  get_query(host: HTMLElement): ViewQuery | null;
  /**
   * The pivot as it is shown, as CSV text: one header line (`2025 · total`),
   * every row including subtotals and the grand total, the element's labels.
   * `null` while nothing is shown, and for the grid and the table. Throws on an
   * unknown or mistyped option, and on a shown answer it cannot read (cells
   * that do not match its row dimensions and columns — a custom provider's).
   * Values are in the canonical wire notation: a float `2` reads `2.0`.
   */
  get_pivot(host: HTMLElement, options?: CsvOptions): string | null;
  /** Defines the three elements. `loadOpengrid()` calls it. */
  register(): void;
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/**
 * What `connect` supplies an element with. Each is one module function —
 * `presentation` is `set_columns`, named apart from the `columns` attribute —
 * and the three callbacks receive the event's `detail`.
 */
export interface ConnectOptions {
  provider?: Provider;
  texts?: Texts;
  formats?: Formats;
  /** Per-column presentation, as `set_columns` takes it (type {@link Columns}). */
  presentation?: Columns;
  choices?: Choices;
  /**
   * The view, controlled: written whenever it differs from what the grid shows
   * — after the reader sorted, passing the same view again puts it back. Writing
   * back what the grid just reported costs nothing. `undefined` or `null`: the
   * grid leads.
   */
  view?: ViewInput | null;
  /** The view, uncontrolled: applied once, then the grid leads. */
  defaultView?: ViewInput;
  /** The reader changed the view. Not called for views `connect` wrote. */
  onViewChange?: (view: View) => void;
  onSelectionChange?: (detail: SelectionChangeDetail) => void;
  onCellChange?: (detail: CellChangeDetail) => void;
}

export interface Connection {
  /** Settles once the module is loaded and the options are applied. */
  ready: Promise<LoadResult>;
  /**
   * Applies what changed. A key left out keeps its value; a key given as
   * `undefined` resets it — except `provider`, and `view`, where it means the
   * grid leads.
   */
  update(options?: ConnectOptions): void;
  /** Removes the listeners. The element keeps its state. */
  disconnect(): void;
}

/**
 * Supplies an element from one options object — in the right order, once the
 * module is loaded, asking once — and keeps it supplied. The view needs the
 * element in the document: on one that is not, the view and the provider wait
 * together for the next `update`. The module is loaded with `loadOpengrid()`;
 * a page that needs its own URLs calls `loadOpengrid(options)` first.
 */
export function connect(host: HTMLElement, options?: ConnectOptions): Connection;

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/**
 * Where the data comes from. A seam, not a class: anything with an `execute`
 * method fits. `queryJson` is the query as JSON; the answer is the result as
 * JSON, or a Promise of it. `mode` is the element's `mode` attribute, `""`
 * without one. `options` is optional and new: a provider written for two
 * arguments still fits.
 */
export interface Provider {
  execute(queryJson: string, mode: string, options?: ExecuteOptions): string | Promise<string>;
  /**
   * Optional: the whole export of `query` in one go. `exportRows` uses it when
   * it is there instead of fetching pieces over `execute` —
   * `createRestProvider` has it, as `POST /export/{source}`.
   */
  export?(query: ViewQuery, options?: ProviderExportOptions): Promise<Blob>;
}

/**
 * The third argument of `execute`. `signal` aborts the request: the REST,
 * pivot and hybrid providers hand it to `fetch`; the tab and the worker cannot
 * stop a query that has started and ignore it — their answer is dropped.
 */
export interface ExecuteOptions {
  signal?: AbortSignal;
}

/** A provider over an engine that holds data: the tab or a worker. */
export interface EngineProvider extends Provider {
  /** Loads CSV bytes as the source `name`, against a schema (JSON). */
  load(name: string, bytes: ArrayBuffer | Uint8Array, schema: string): Promise<void>;
  /** Stops a worker; a no-op in the tab. */
  terminate(): void;
}

/** The engine of the engine module, as `createLocalProvider` uses it. */
export interface Engine {
  load_csv(name: string, bytes: Uint8Array, schema: string): void;
  execute(queryJson: string): string;
}

/** What a server says a source is. */
export interface SourceDescription {
  name: string;
  schema: unknown;
  capabilities: unknown;
  pivot_limits: unknown;
}

export interface RestProvider extends Provider {
  /** `GET /source/{name}`: what a `Planner` needs to split queries. */
  describe(): Promise<SourceDescription>;
  /**
   * `POST /export/{source}`: every row of `query` in one streamed request, in
   * the notation `exportRows` writes. The server's rules are `/query`'s; its
   * bound is `max_export_rows`. `maxRows` refuses before the body is read,
   * `onProgress` hears the end, `signal` aborts the download too.
   */
  export(query: ViewQuery, options?: ProviderExportOptions): Promise<Blob>;
}

/** The `Planner` of the engine module, as `createHybridProvider` uses it. */
export interface PlannerLike {
  /** The plan as JSON: `{ mode, describe, steps, source, client }`. */
  plan(queryJson: string, mode: string): string;
  /** Finishes the client half over the source's answer; the result as JSON. */
  finish(clientQueryJson: string, resultJson: string): string;
}

/** The plan `createHybridProvider` hands to `onPlan` before anything is sent. */
export interface ExecutionPlan {
  mode: Mode;
  /** One line for tools: `source: filter · sort | client: group · aggregate`. */
  describe: string;
  steps: string[];
  source: unknown;
  /** The half the tab finishes; `null` when the source answers everything. */
  client: unknown;
}

export interface ServerOptions {
  /** The server's base URL. */
  url: string;
  /** The configured data source name. */
  source: string;
  /** Bearer token; the server refuses without one. */
  token?: string;
}

/** The engine on the main thread. */
export function createLocalProvider(engine: Engine): EngineProvider;

/**
 * The engine in a module worker, started lazily and once. Without `moduleUrl`
 * it is the engine the package ships, `engine/opengrid_wasm.js` next to the
 * loader. `moduleUrl` and `wasmUrl` travel to the worker by `postMessage`, so
 * they are strings — a `URL` object cannot be copied there; pass `url.href`.
 */
export function createWorkerProvider(options?: {
  moduleUrl?: string;
  wasmUrl?: string;
  workerUrl?: URL | string;
}): EngineProvider & { readonly worker: Worker | undefined };

/** `POST /query/{source}` of an `opengrid-server`. */
export function createRestProvider(options: ServerOptions): RestProvider;

/** `POST /pivot/{source}` — a whole pivot in one request. */
export function createPivotProvider(options: ServerOptions): Provider;

/** Splits each query between a remote source and the engine in the tab. */
export function createHybridProvider(options: {
  remote: Provider;
  planner: PlannerLike;
  mode?: Mode;
  onPlan?: (plan: ExecutionPlan) => void;
}): Provider;

// ---------------------------------------------------------------------------
// Exporting
// ---------------------------------------------------------------------------

/** What `onProgress` hears after each piece. */
export interface ExportProgress {
  /** Rows written so far. */
  rows: number;
  /** Every match, from the first piece's `total_count`. */
  total: number;
}

/**
 * How `exportRows` fetches and writes: its own keys, plus the
 * {@link CsvOptions} `get_pivot` takes too. Any other key is an error, and so
 * is a CSV option on a JSON export.
 */
export interface ExportOptions extends CsvOptions {
  /** `"csv"` (the default) or `"json"` — an array of row objects. */
  format?: "csv" | "json";
  /** Rows per request; 10 000 by default, the server's `max_limit`. */
  chunkSize?: number;
  /** More matches than this is an error, never a truncated file; 1 000 000 by default. */
  maxRows?: number;
  /** Called after each piece. */
  onProgress?: (progress: ExportProgress) => void;
  /** Aborts the export: it rejects with an `AbortError` and gives no `Blob`. */
  signal?: AbortSignal;
}

/**
 * What a provider's `export` takes: {@link ExportOptions} without
 * `chunkSize` — there are no pieces to size. Any other key is an error.
 */
export type ProviderExportOptions = Omit<ExportOptions, "chunkSize">;

/**
 * Every match of `query` — as `get_query(host)` gives it — through `provider`,
 * in pieces, as a `Blob` of `text/csv;charset=utf-8` or `application/json`.
 * Raw values in the wire notation, a header of field names. The sort is made
 * total by appending every selected column not yet in it, ascending; within a
 * tie the export follows the columns, not the grid. A source whose count
 * changes between two pieces is refused with an error, not exported. A
 * provider with an `export` method gets the same query in one request instead.
 */
export function exportRows(provider: Provider, query: ViewQuery, options?: ExportOptions): Promise<Blob>;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/**
 * What went wrong, as a closed list a page can switch on. The first seven are
 * the server's, from its error form; the last three are `exportRows`' own.
 */
export type ErrorCode =
  | "validation"
  | "unknown_source"
  | "limit_exceeded"
  | "busy"
  | "unauthorized"
  | "backend"
  | "malformed"
  | "too_many_rows"
  | "source_changed"
  | "module_not_loaded";

/**
 * What the REST and pivot providers and `exportRows` reject with: a plain
 * `Error` with these fields — not a class of its own, so test `error.code`,
 * not `instanceof`. The message is a sentence for the developer, the same as
 * without the fields. A wrong option is a `TypeError`, an abort a
 * `DOMException` named `"AbortError"` — test its `name`: its `code` is the
 * DOM's legacy number, never one of these strings.
 *
 * ```ts
 * try {
 *   await exportRows(provider, query);
 * } catch (error) {
 *   if ((error as Error).name === "AbortError") return;
 *   switch ((error as CodedError).code) {
 *     case "busy": // try again in a moment
 *     case "too_many_rows": // narrow the view
 *     default: // the export failed
 *   }
 * }
 * ```
 */
export interface CodedError extends Error {
  /** The HTTP status, when a server refused the request. */
  status?: number;
  /**
   * Absent when neither the server nor the loader named one. A newer server
   * may send a code this list does not have yet: keep a `default:` branch.
   */
  code?: ErrorCode;
  /** Where in the query, when the server says: `filter.and[1].value`. */
  path?: string;
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Where a query may run; handed to the provider unchanged. */
export type Mode = "local" | "remote" | "hybrid" | "auto";

export type Density = "compact" | "normal" | "comfortable";

/** The filter operators, by wire token. */
export type FilterOperator =
  | "eq"
  | "ne"
  | "gt"
  | "gte"
  | "lt"
  | "lte"
  | "contains"
  | "starts_with"
  | "is_null"
  | "is_not_null";

/** An aggregate in group rows and the grand total. */
export type Aggregate = "sum" | "avg" | "count" | "min" | "max" | "range";

/** How a column is offered as a facet. */
export type FacetKind = "list" | "pills" | "range" | "period";

/** A value as the wire format writes it: decimals and dates are strings. */
export type WireValue = string | number | boolean | null;

/** Per-column presentation for `set_columns` — `presentation` in `connect`. */
export interface ColumnConfig {
  /** The starting width in pixels; a reader's resize leads after that. */
  width?: number;
  align?: "start" | "end" | "center";
  mono?: boolean;
  emphasis?: boolean;
  muted?: boolean;
  aggregate?: Aggregate;
  facet?: FacetKind;
}

export type Columns = Record<string, ColumnConfig>;

/** A display format: a function, or `Intl` options with a `kind`. */
export type Format =
  | ((text: string, value: WireValue) => string)
  | ({ kind?: "number"; locale?: string } & Intl.NumberFormatOptions)
  | ({ kind: "date"; locale?: string } & Intl.DateTimeFormatOptions);

export type Formats = Record<string, Format>;

export type Choices = Record<string, string[]>;

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

export interface SortKey {
  field: string;
  direction: "asc" | "desc";
}

/** A sort key of a query, which may also say where NULLs land. */
export interface QuerySortKey extends SortKey {
  /**
   * `"last"` when left out, whatever the direction. `get_query` writes it for
   * the group keys.
   */
  nulls?: "first" | "last";
}

export interface FilterEntry {
  column: string;
  op: FilterOperator;
  /** The value as typed, in the notation of the column's type. */
  value: string;
}

/** A facet selection: values for `list`/`pills`, bounds for `range`/`period`. */
export type FacetSelection =
  | { values: WireValue[] }
  | { min: string; max: string }
  | { from: string; to: string };

/**
 * Sort, filters, column layout, density, grouping, aggregates and facets as
 * one value — as `get_view` writes it, every field present. `set_view` reads
 * some fields more leniently (a missing direction is `asc`, a missing operator
 * `eq`); the types ask for the full form on purpose, so a saved view says what
 * it means. A saved view is this value with a name on it. The selection and
 * the free-text search are deliberately not part of it.
 */
export interface View {
  sort: SortKey[];
  filters: FilterEntry[];
  columns: { order: string[]; hidden: string[]; widths: Record<string, number> };
  density: Density;
  group: string[];
  /** The open groups, each as its path of keys. */
  expanded: WireValue[][];
  /** The reader's aggregate per column; leads over `set_columns`. */
  aggregates: Record<string, Aggregate>;
  /** Whether the filter row shows. */
  filterRow: boolean;
  facets: Record<string, FacetSelection>;
}

/** The query of a view, as `get_query` gives it: every match, no window. */
export interface ViewQuery {
  source: string;
  select: string[];
  /** The filter expression of the query model, when anything restricts. */
  filter?: unknown;
  /** Never empty: a grid without a sort pages under its first column. */
  sort: QuerySortKey[];
}

/** How a CSV is written. Every key is optional; no other key is accepted. */
export interface CsvOptions {
  /** One character: `,` by default, `;` for a German Excel. */
  delimiter?: string;
  /** Start with a UTF-8 byte order mark. `true` by default (Excel). */
  bom?: boolean;
  /** Prefix a text cell that a spreadsheet would run as a formula. `true` by default. */
  protectFormulas?: boolean;
  /** How NULL is written. Empty by default; `"\\N"` reads back into opengrid. */
  null?: string;
}

/**
 * What `set_view` takes: any part of a view. A part left out is not "unchanged"
 * but its default — no sort falls back to the first column, no filters is none.
 */
export type ViewInput = Partial<Omit<View, "columns">> & {
  columns?: Partial<View["columns"]>;
};

// ---------------------------------------------------------------------------
// Texts
// ---------------------------------------------------------------------------

/** Every text key, `lang` and `operators` aside. */
export type TextKey =
  | "loading"
  | "matchesOne"
  | "matchesOther"
  | "empty"
  | "error"
  | "errorUnknown"
  | "filterGroup"
  | "operatorLabel"
  | "valueLabel"
  | "clear"
  | "selectAll"
  | "selectedAll"
  | "groupRow"
  | "rowsOne"
  | "rowsOther"
  | "groupExpanded"
  | "groupCollapsed"
  | "groupInvalid"
  | "totalRow"
  | "aggregateCell"
  | "aggregateSum"
  | "aggregateAvg"
  | "aggregateCount"
  | "aggregateMin"
  | "aggregateMax"
  | "aggregateRange"
  | "columnMenu"
  | "sortAscending"
  | "sortDescending"
  | "filterColumn"
  | "aggregateGroup"
  | "aggregateNone"
  | "groupByColumn"
  | "groupSecondLevel"
  | "ungroupColumn"
  | "hideColumn"
  | "toolbarGroup"
  | "filterRowToggle"
  | "densityGroup"
  | "densityCompact"
  | "densityNormal"
  | "densityComfortable"
  | "chipsGroup"
  | "chipsClear"
  | "chipRemove"
  | "filterRemoved"
  | "filtersCleared"
  | "groupChip"
  | "facetsGroup"
  | "facetsToggle"
  | "facetsReset"
  | "facetFrom"
  | "facetTo"
  | "facetQueries"
  | "facetChipValues"
  | "searchLabel"
  | "searchPlaceholder"
  | "queryAnd"
  | "searchHint"
  | "searchSuggestions"
  | "typeText"
  | "typeBool"
  | "typeInteger"
  | "typeNumber"
  | "typeDate"
  | "typeTime"
  | "searchChip"
  | "queryUnknownColumn"
  | "queryMissingValue"
  | "queryWrongOperator"
  | "emptyFiltered"
  | "emptySource"
  | "emptyReset"
  | "filterInvalid"
  | "cellRequired"
  | "selectionCleared"
  | "columnWidth"
  | "columnMoved"
  | "columnAtEdge"
  | "columnHidden"
  | "columnShown"
  | "columnsGroup"
  | "pageFirst"
  | "pagePrevious"
  | "pageNext"
  | "pageLast"
  | "pageOf"
  | "total"
  | "subtotal"
  | "noValue"
  | "emptyValue";

/**
 * Any subset of the texts; what is left out keeps its English default. `lang`
 * goes onto the elements that carry these texts, never onto the data.
 */
export type Texts = Partial<Record<TextKey, string>> & {
  lang?: string;
  operators?: Partial<Record<FilterOperator, string>>;
};

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/** `opengrid-selection-change`: logical row numbers, ascending. */
export interface SelectionChangeDetail {
  rows: number[];
  count: number;
}

/** `opengrid-cell-change`: everything a page needs to persist an edit. */
export interface CellChangeDetail {
  row: number;
  column: string;
  value: string;
  previous: string;
}

/** `opengrid-view-change`: the whole view after the change. */
export interface ViewChangeDetail {
  view: View;
}

/** The events, by name. All bubble, all are composed, none is cancelable. */
export interface OpengridEventMap {
  "opengrid-selection-change": CustomEvent<SelectionChangeDetail>;
  "opengrid-cell-change": CustomEvent<CellChangeDetail>;
  "opengrid-view-change": CustomEvent<ViewChangeDetail>;
}

// ---------------------------------------------------------------------------
// Elements and their attributes
// ---------------------------------------------------------------------------

/**
 * The attributes of `<opengrid-grid>`, as strings the way HTML has them — for
 * adapters and JSX typings to build on. For
 * the boolean ones — `selection`, `toolbar`, `search`, `facets`,
 * `column-menu` — presence is what counts, whatever the value.
 */
export interface OpengridGridAttributes {
  label?: string;
  datasource?: string;
  columns?: string;
  "window-size"?: string;
  "page-size"?: string;
  mode?: Mode;
  "group-by"?: string;
  search?: string;
  facets?: string;
  toolbar?: string;
  "column-menu"?: string;
  selection?: string;
  density?: Density;
}

export interface OpengridTableAttributes {
  label?: string;
  datasource?: string;
  columns?: string;
}

export interface OpengridPivotAttributes {
  label?: string;
  datasource?: string;
  /** Comma-separated row dimensions, outermost first. */
  rows?: string;
  /** Comma-separated column dimensions; V1 allows one. */
  columns?: string;
  /** The measures as the contract's JSON: `[{"field":"qty","fn":"sum","as":"total"}]`. */
  values?: string;
}

/** `<opengrid-grid>`; it fires the three events. */
export interface OpengridGridElement extends HTMLElement {}
/** `<opengrid-table>`. */
export interface OpengridTableElement extends HTMLElement {}
/** `<opengrid-pivot>`. */
export interface OpengridPivotElement extends HTMLElement {}

declare global {
  interface HTMLElementTagNameMap {
    "opengrid-grid": OpengridGridElement;
    "opengrid-table": OpengridTableElement;
    "opengrid-pivot": OpengridPivotElement;
  }
  /**
   * The events bubble and are composed, so a listener on an ancestor, the
   * document or the window hears them too — hence the global handler map.
   */
  interface GlobalEventHandlersEventMap extends OpengridEventMap {}
}
