/* tslint:disable */
/* eslint-disable */

/**
 * The engine the browser talks to: a set of in-memory sources.
 *
 * One instance holds one [`LocalDataSource`] per name
 * (plan/spezifikation/03-datasource.md). The demo registers `orders` and runs
 * the example query of `spezifikation/02-query-modell.md` against it.
 */
export class Engine {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Runs a query JSON and answers with the result JSON.
     *
     * The result is the wire shape of a [`QueryResult`]: `total_count` for the
     * number before paging, `row_count` for this page, and one
     * `{ "name", "values" }` object per output column, in schema order.
     */
    execute(query_json: string): string;
    /**
     * Runs a query JSON and answers with the result in the binary form
     * (decision E35, `opengrid_columns::wire`): the same answer as
     * [`execute`](Engine::execute_js), but as bytes a worker can hand over
     * without copying and an element can read without parsing.
     */
    execute_columns(query_json: string): Uint8Array;
    /**
     * Reads CSV bytes against an explicit schema and registers the source.
     *
     * The schema is required — ingest never guesses types, and an inferred
     * proposal is a suggestion a UI may show
     * (plan/spezifikation/04-local-engine.md §Ingest).
     */
    load_csv(name: string, bytes: Uint8Array, schema_json: string): void;
    /**
     * Registers a source from rows the page already holds (issue #182): a
     * JSON array of objects keyed by field name, values in the wire notation
     * of E13. The schema is required, as for CSV. A name that is already
     * loaded is **replaced** — all or nothing: a row that does not fit is
     * refused with its row and field named, and the old source stays.
     */
    load_rows(name: string, rows_json: string, schema_json: string): void;
    /**
     * An engine without data. `load_csv` adds sources.
     */
    constructor();
    /**
     * Runs a pivot JSON over the source it names and answers the pivot's JSON
     * form (issue #28) — what `POST /pivot/{source}` answers, computed here.
     */
    pivot(pivot_json: string): string;
    /**
     * The same pivot in the binary form (E35) — what the tab and the worker
     * hand the element.
     */
    pivot_columns(pivot_json: string): Uint8Array;
    /**
     * The names of the registered sources, in unspecified order.
     */
    source_names(): string[];
    /**
     * What the engine holds (issue #70), as JSON:
     * `{ "memory": bytes, "sources": [{ "name", "rows", "columns", "bytes" }] }`.
     *
     * `memory` is the module's whole linear memory — what the browser gave
     * it, which only grows; `bytes` is what a source's columns take inside
     * it. Sources by name.
     */
    stats(): string;
    /**
     * The query of a view (issue #144): what `get_query()` answers on an
     * `<opengrid-grid datasource=source columns=…>` once `view_json` is
     * applied — without an element, so a server can count or export a saved
     * view. `columns` is the grid's `columns` attribute, as a JSON array.
     *
     * A view naming a column the grid does not have is refused as a whole,
     * every problem named; so is a filter value that is not a value of its
     * column.
     */
    view_query(source: string, columns_json: string, view_json: string): string;
}

/**
 * The planner in the browser: who answers which half of a query (point 28).
 *
 * The page holds a remote source it cannot change and an engine it can. This
 * object decides between them and does the local half:
 *
 * 1. [`plan`](Planner::plan) turns a query into the query for the source, the
 *    query for the client, and a sentence describing the split.
 * 2. The page sends the source query wherever it goes and gets an answer back.
 * 3. [`finish`](Planner::finish) runs the client query over that answer.
 *
 * It holds no state between the two calls on purpose: the plan travels as JSON
 * through the page, which is also what makes it readable from outside
 * (plan/spezifikation/05-planner.md: "Offengelegt für Debugging").
 */
export class Planner {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Runs `client_query_json` over `result_json` — the source's answer.
     *
     * This is the hybrid return path E14 names: the partial result becomes
     * a table again and the engine finishes the query over them, which
     * includes recounting `total_count` — filtering or grouping changed it.
     */
    finish(client_query_json: string, result_json: string): string;
    /**
     * [`finish`](Planner::finish) for a source that answered in the binary
     * form: the bytes in, the finished result as bytes out.
     */
    finish_columns(client_query_json: string, result: Uint8Array): Uint8Array;
    /**
     * A planner for a source with `schema_json` and `capabilities_json`, in
     * `mode` (`local`, `remote`, `hybrid` or `auto`).
     *
     * Both JSON documents are what `GET /source/{name}` answers, so a page can
     * hand them straight through without knowing their shape.
     */
    constructor(schema_json: string, capabilities_json: string, mode: string);
    /**
     * Splits `query_json`, answering the plan as JSON.
     *
     * `mode` overrides the planner's own mode when it is not empty — that is
     * how the element's `mode` attribute reaches the planner without rebuilding
     * it on every attribute change.
     *
     * The answer is `{ mode, describe, steps, source, client }`: `source` is the
     * query to send on, `client` the query to hand back to
     * [`finish`](Planner::finish), or `null` when the source answers it all.
     */
    plan(query_json: string, mode: string): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_engine_free: (a: number, b: number) => void;
    readonly __wbg_planner_free: (a: number, b: number) => void;
    readonly engine_execute: (a: number, b: number, c: number) => [number, number, number, number];
    readonly engine_execute_columns: (a: number, b: number, c: number) => [number, number, number, number];
    readonly engine_load_csv: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly engine_load_rows: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number];
    readonly engine_new: () => number;
    readonly engine_pivot: (a: number, b: number, c: number) => [number, number, number, number];
    readonly engine_pivot_columns: (a: number, b: number, c: number) => [number, number, number, number];
    readonly engine_source_names: (a: number) => [number, number];
    readonly engine_stats: (a: number) => [number, number];
    readonly engine_view_query: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => [number, number, number, number];
    readonly planner_finish: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly planner_finish_columns: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly planner_new: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
    readonly planner_plan: (a: number, b: number, c: number, d: number, e: number) => [number, number, number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_drop_slice: (a: number, b: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
