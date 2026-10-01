// The operator's configuration (#140): which sources exist. The model names a
// source; it never names a path, writes SQL or reaches a network.
//
// opengrid-mcp.json:
//   { "sources": { "orders": { "csv": "data/orders.csv", "schema": "data/orders.schema.json",
//                              "columns": ["id", "customer", "country", "amount"],
//                              "fields": ["id", "customer", "country", "amount", "qty"] } } }
//
// Paths are relative to the configuration file. `columns` are the grid's
// columns (its `columns` attribute); `fields` is the allow-list of fields a
// query may name — absent, every field of the schema.
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { z } from "zod";

const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;

const Source = z
  .object({
    csv: z.string().min(1),
    schema: z.string().min(1),
    columns: z.array(z.string().regex(NAME)).min(1).optional(),
    fields: z.array(z.string().regex(NAME)).min(1).optional(),
    title: z.string().optional(),
  })
  .strict();

const Config = z
  .object({
    sources: z.record(z.string().regex(NAME), Source).refine((sources) => Object.keys(sources).length > 0, {
      message: "at least one source",
    }),
  })
  .strict();

/** Reads and checks the configuration; paths come back absolute. */
export function loadConfig(path) {
  const file = resolve(path);
  let raw;
  try {
    raw = JSON.parse(readFileSync(file, "utf8"));
  } catch (error) {
    throw new Error(`opengrid-mcp: cannot read ${file}: ${error.message}`);
  }
  const parsed = Config.safeParse(raw);
  if (!parsed.success) {
    const problems = parsed.error.issues.map((issue) => `${issue.path.join(".") || "(root)"}: ${issue.message}`);
    throw new Error(`opengrid-mcp: ${file} is not a valid configuration:\n  ${problems.join("\n  ")}`);
  }
  const base = dirname(file);
  const sources = {};
  for (const [name, source] of Object.entries(parsed.data.sources)) {
    sources[name] = { ...source, csv: resolve(base, source.csv), schema: resolve(base, source.schema) };
  }
  return { sources };
}
