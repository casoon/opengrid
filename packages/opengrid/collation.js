/**
 * The collator of the engine (rule S4a): loaded when a query first sorts by a
 * locale.
 *
 * A sort key may name a locale — `{ "field": "name", "collation": "de" }` — and
 * the engine then needs the collation's tables, some hundred kilobytes it does
 * not carry. They are a module of their own (`collation/`, built by
 * `just wasm-build-collation`); a page that never sorts by a locale does not
 * fetch it. `loader.js` and `worker.js` import **this file lazily**, on the first
 * query that names a locale, so a page that serves the package's files itself and
 * leaves this one out breaks locale sorts only (issue #415).
 */

/** The engines and planners that have their collator. */
const given = new WeakSet();

/** The collation module, loaded once; a failed load is not kept. */
let module;

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
