// The sources — a CSV in opengrid's engine (WebAssembly in Node), or a source
// of an opengrid-server (#150). The rows stay here or there; the view and the
// model see answers to queries (#140).
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

/** The fields of a schema a CSV holds: a derived field (`from`) is computed. */
function headerOf(fields) {
  return `${fields.filter((field) => !field.from).map((field) => csvField(field.name)).join(",")}\n`;
}

/** An `opengrid-server`'s error answer as a DataError, or its status in words. */
function refusal(status, text) {
  try {
    const { error } = JSON.parse(text);
    return new DataError(error.code, error.message, error.path);
  } catch {
    return new DataError("backend", `the server answered ${status}`);
  }
}

/**
 * A source of an `opengrid-server` (issue #150): its token, its allow-list and
 * its row filter are the server's; here it is described once and asked by
 * query.
 */
async function serverSource(name, server) {
  const base = server.url.replace(/\/$/, "");
  const remote = server.source ?? name;
  const token = server.tokenEnv ? process.env[server.tokenEnv] : server.token;
  if (server.tokenEnv && !token) {
    throw new Error(`opengrid-mcp: source ${name}: the environment variable ${server.tokenEnv} is not set`);
  }
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  let response;
  try {
    response = await fetch(`${base}/source/${encodeURIComponent(remote)}`, { headers, redirect: "error" });
  } catch (error) {
    throw new Error(`opengrid-mcp: source ${name}: ${base} cannot be reached: ${error.cause?.message ?? error.message}`);
  }
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`opengrid-mcp: source ${name}: ${base} refused: ${refusal(response.status, text).message}`);
  }
  const { schema } = JSON.parse(text);
  const run = async (query) => {
    const answer = await fetch(`${base}/query/${encodeURIComponent(remote)}`, {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify({ ...query, source: remote }),
      redirect: "error",
    });
    const body = await answer.text();
    if (!answer.ok) throw refusal(answer.status, body);
    return body;
  };
  return { schema, run };
}

export class Data {
  /** Reads the sources — a server source is described once, here. */
  static async load(config) {
    const data = new Data();
    for (const [name, source] of Object.entries(config.sources)) {
      if (source.server) {
        const { schema, run } = await serverSource(name, source.server);
        data.add(name, source, schema, schema.fields.map((field) => field.name), run);
      } else {
        const schema = JSON.parse(readFileSync(source.schema, "utf8"));
        const allowed = source.fields ?? schema.fields.map((field) => field.name);
        const unknown = allowed.filter((field) => !schema.fields.some((f) => f.name === field));
        if (unknown.length) {
          throw new Error(`opengrid-mcp: source ${name}: fields not in the schema: ${unknown.join(", ")}`);
        }
        data.engine.load_csv(name, readFileSync(source.csv), JSON.stringify(schema));
        data.add(name, source, schema, allowed, async (query) => data.engine.execute(JSON.stringify(query)));
      }
    }
    return data;
  }

  constructor() {
    this.engine = new Engine();
    this.checker = new Engine();
    this.sources = new Map();
  }

  /**
   * Registers a source: its narrowed schema in the checker (an empty copy a
   * query is checked against first, as `opengrid-server` narrows its schema),
   * and how it runs a query.
   */
  add(name, source, schema, allowed, run) {
    // A derived field stays only when the field it is derived from is allowed.
    const narrowed = {
      ...schema,
      fields: schema.fields.filter(
        (field) => allowed.includes(field.name) && (!field.from || allowed.includes(field.from.field)),
      ),
    };
    this.checker.load_csv(name, new TextEncoder().encode(headerOf(narrowed.fields)), JSON.stringify(narrowed));
    const names = narrowed.fields.map((field) => field.name);
    const columns = (source.columns ?? names).filter((column) => names.includes(column));
    this.sources.set(name, { title: source.title ?? name, fields: narrowed.fields, columns, run });
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
  async execute(query) {
    const source = this.source(query.source);
    try {
      this.checker.execute(JSON.stringify(query));
    } catch (error) {
      throw new DataError("validation", String(error.message ?? error), "query");
    }
    return source.run(query);
  }

  /** How many rows a query matches. */
  async count(query) {
    const { limit, offset, ...whole } = query;
    return JSON.parse(await this.execute({ ...whole, limit: 0 })).total_count;
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
