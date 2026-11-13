/**
 * The collator of the engine (rule S4a): loaded when a query first sorts by a
 * locale.
 *
 * A sort key may name a locale — `{ "field": "name", "collation": "de" }` — and
 * the engine then needs the collation's tables, some hundred kilobytes it does
 * not carry. They are a module of their own (`collation/`, built by
 * `just wasm-build-collation`); a page that never sorts by a locale does not
 * fetch it. The providers of the package call `withCollator` before the engine
 * runs a query.
 */

/** A sort key's collation that is not `binary`, in a query or a plan as JSON. */
const NAMES_A_LOCALE = /"collation"\s*:\s*"(?!binary")/;

/** The engines and planners that have their collator. */
const given = new WeakSet();

/** The collation module, loaded once; a failed load is not kept. */
let module;

/**
 * Whether `json` — a query, or the client half of a plan — sorts by a locale.
 *
 * @param {string} json
 * @returns {boolean}
 */
export function sortsByLocale(json) {
  return NAMES_A_LOCALE.test(json);
}

/**
 * Loads the collation module and gives `engine` its collator. Once per engine;
 * later calls answer at once.
 *
 * @param {{set_collator: Function}} engine an `Engine` or a `Planner`.
 * @returns {Promise<void>}
 */
export async function giveCollator(engine) {
  if (given.has(engine)) {
    return;
  }
  module ??= import("./collation/opengrid_collation.js").then(async (glue) => {
    await glue.default();
    return glue;
  });
  let glue;
  try {
    glue = await module;
  } catch (error) {
    module = undefined;
    const failed = new Error(
      `the collation module did not load (${error?.message ?? error})`,
    );
    failed.code = "module_not_loaded";
    throw failed;
  }
  engine.set_collator(glue.sort_keys);
  given.add(engine);
}

/**
 * Runs `run` once `engine` can answer `json`: at once when the query sorts by
 * no locale or the engine has its collator — the answer is then `run`'s own,
 * not a promise — and after the module has loaded otherwise.
 *
 * @template T
 * @param {{set_collator: Function}} engine an `Engine` or a `Planner`.
 * @param {string} json the query, or the client half of a plan.
 * @param {() => T} run
 * @returns {T | Promise<T>}
 */
export function withCollator(engine, json, run) {
  if (given.has(engine) || !sortsByLocale(json)) {
    return run();
  }
  return giveCollator(engine).then(run);
}
