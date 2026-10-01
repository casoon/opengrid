// The sources, in opengrid's engine (WebAssembly in Node) — the rows stay here;
// the view and the model see answers to queries (#140).
//
// The allow-list works as the server's does (`opengrid-server`, narrowing the
// schema): a query is first checked against an empty copy of the source that
// has only the allowed fields, then run on the rows.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { initSync, Engine } from "@casoon/opengrid/engine/opengrid_wasm.js";

const require = createRequire(import.meta.url);
initSync({ module: readFileSync(require.resolve("@casoon/opengrid/engine/opengrid_wasm_bg.wasm")) });

/** An error with a code, for the tools' answers. */
export class DataError extends Error {
  constructor(code, message, path) {
    super(message);
    this.code = code;
    this.path = path;
  }
}

function csvField(name) {
  return /[",\n]/.test(name) ? `"${name.replaceAll('"', '""')}"` : name;
}

export class Data {
  constructor(config) {
    this.engine = new Engine();
    this.checker = new Engine();
    this.sources = new Map();
    for (const [name, source] of Object.entries(config.sources)) {
      const schema = JSON.parse(readFileSync(source.schema, "utf8"));
      const allowed = source.fields ?? schema.fields.map((field) => field.name);
      const unknown = allowed.filter((field) => !schema.fields.some((f) => f.name === field));
      if (unknown.length) {
        throw new Error(`opengrid-mcp: source ${name}: fields not in the schema: ${unknown.join(", ")}`);
      }
      this.engine.load_csv(name, readFileSync(source.csv), JSON.stringify(schema));
            // A derived field (`from`) is computed, not read: it is no CSV column,
      // and it stays only when the field it is derived from is allowed.
      const narrowed = {
        ...schema,
        fields: schema.fields.filter(
          (field) => allowed.includes(field.name) && (!field.from || allowed.includes(field.from.field)),
        ),
      };
      const stored = narrowed.fields.filter((field) => !field.from);
      const header = `${stored.map((field) => csvField(field.name)).join(",")}\n`;
      this.checker.load_csv(name, new TextEncoder().encode(header), JSON.stringify(narrowed));
      const columns = (source.columns ?? narrowed.fields.map((field) => field.name)).filter((column) =>
        allowed.includes(column),
      );
      this.sources.set(name, { title: source.title ?? name, fields: narrowed.fields, columns });
    }
  }

  /** The names of the sources. */
  names() {
    return [...this.sources.keys()];
  }

  source(name) {
    const source = this.sources.get(name);
    if (!source) {
      throw new DataError("unknown_source", `no source ${JSON.stringify(name)}; there are: ${this.names().join(", ")}`, "source");
    }
    return source;
  }

  /**
   * Runs a query (an object) on the source it names, after checking it
   * against the allowed fields. Answers the result in the wire JSON.
   */
  execute(query) {
    const json = JSON.stringify(query);
    this.source(query.source);
    try {
      this.checker.execute(json);
    } catch (error) {
      throw new DataError("validation", String(error.message ?? error), "query");
    }
    return this.engine.execute(json);
  }

  /** How many rows a query matches. */
  count(query) {
    const { limit, offset, ...whole } = query;
    return JSON.parse(this.execute({ ...whole, limit: 0 })).total_count;
  }

  /**
   * The query of a view on the source's grid (issue #144), checked against the
   * allowed fields: a view naming any other column is refused as a whole.
   */
  viewQuery(name, view) {
    const source = this.source(name);
    try {
      return JSON.parse(this.checker.view_query(name, JSON.stringify(source.columns), JSON.stringify(view ?? {})));
    } catch (error) {
      throw new DataError("validation", String(error.message ?? error), "view");
    }
  }
}
