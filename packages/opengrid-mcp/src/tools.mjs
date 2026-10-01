// The tools of @casoon/opengrid-mcp (#140). opengrid knows nothing of MCP:
// everything here is the public API — a view, the query of a view, a query.
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { registerAppResource, registerAppTool, RESOURCE_MIME_TYPE } from "@modelcontextprotocol/ext-apps/server";
import { z } from "zod";

import { DataError } from "./data.mjs";
import { rowsOf, runsOf } from "./rows.mjs";

export const GRID_URI = "ui://opengrid/grid";
const VIEW_HTML = fileURLToPath(new URL("../dist/view.html", import.meta.url));

/** The selection's default and largest answer (point 11). */
export const SELECTION_LIMIT = 100;
export const SELECTION_MAX = 500;

const View = z.record(z.string(), z.unknown()).describe(
  "A grid view, exactly the JSON of opengrid's get_view: sort, filters, columns, density, group, expanded, aggregates, filter_row, facets. Fields left out keep their defaults.",
);

function answer(structured, text) {
  return { structuredContent: structured, content: [{ type: "text", text: text ?? JSON.stringify(structured) }] };
}

function failure(error) {
  const body =
    error instanceof DataError
      ? { code: error.code, message: error.message, path: error.path }
      : { code: "internal", message: String(error.message ?? error) };
  return { isError: true, content: [{ type: "text", text: JSON.stringify({ error: body }) }] };
}

function guarded(handler) {
  return async (args) => {
    try {
      return await handler(args);
    } catch (error) {
      return failure(error);
    }
  };
}

function sessionOf(sessions, id) {
  const session = sessions.get(id);
  if (!session) {
    throw new DataError("unknown_session", `no session ${JSON.stringify(id)}; open one with opengrid_open`, "sessionId");
  }
  return session;
}

/** The columns a session shows, with their types. */
function columnsOf(data, source) {
  const { fields, columns } = data.source(source);
  return columns.map((name) => {
    const field = fields.find((f) => f.name === name);
    return { name, type: field.type, nullable: field.nullable ?? true };
  });
}

/** One line for the model: what the grid shows. */
function summary(session) {
  const view = session.view ?? {};
  const parts = [];
  const filters = (view.filters ?? []).map((entry) => `${entry.column} ${entry.op} ${entry.value ?? ""}`.trim());
  if (filters.length) parts.push(`Filter: ${filters.join(", ")}`);
  const sort = (view.sort ?? []).map((key) => `${key.field} ${key.direction ?? "asc"}`);
  if (sort.length) parts.push(`Sort: ${sort.join(", ")}`);
  if (view.group?.length) parts.push(`Grouped by: ${view.group.join(", ")}`);
  parts.push(`${session.total} matches`);
  if (session.selected.length) parts.push(`${session.selected.length} selected`);
  return parts.join(" · ");
}

export function registerTools(server, data, sessions) {
  registerAppResource(
    server,
    "opengrid grid",
    GRID_URI,
    { description: "An accessible data grid over a source of the opengrid MCP server" },
    async () => {
      if (!existsSync(VIEW_HTML)) {
        throw new Error("the grid view is not built: run `just mcp-elements` and `pnpm run build`");
      }
      return {
        contents: [
          {
            uri: GRID_URI,
            mimeType: RESOURCE_MIME_TYPE,
            text: readFileSync(VIEW_HTML, "utf8"),
            // Everything is inline: no network, no other origin (E40).
            _meta: { ui: { csp: {}, prefersBorder: true } },
          },
        ],
      };
    },
  );

  registerAppTool(
    server,
    "opengrid_open",
    {
      title: "Open a data grid",
      description: `Opens a source as an interactive, accessible data grid in the chat. Answers with the session, the columns and the number of rows — never the rows themselves. Sources: ${data.names().join(", ")}.`,
      inputSchema: z.object({
        source: z.string().describe("The source to open."),
        view: View.optional(),
      }),
      _meta: { ui: { resourceUri: GRID_URI } },
    },
    guarded(async ({ source, view }) => {
      const query = data.viewQuery(source, view ?? {});
      const total = data.count(query);
      const session = sessions.open(source, view ?? {}, query, total);
      return answer(
        {
          sessionId: session.id,
          source,
          title: data.source(source).title,
          columns: columnsOf(data, source),
          total,
          view: session.view,
          revision: session.revision,
        },
        `Opened ${source} as a grid (session ${session.id}): ${summary(session)}.`,
      );
    }),
  );

  registerAppTool(
    server,
    "opengrid_set_view",
    {
      title: "Set the grid's view",
      description:
        "Filters, sorts, groups or rearranges the grid by setting its whole view. A view naming a column the grid does not have is refused as a whole. Answers with the applied view and the new number of matches; the grid in the chat shows it when it next syncs.",
      inputSchema: z.object({ sessionId: z.string(), view: View }),
      _meta: { ui: { visibility: ["model"] } },
    },
    guarded(async ({ sessionId, view }) => {
      const session = sessionOf(sessions, sessionId);
      const query = data.viewQuery(session.source, view);
      const total = data.count(query);
      sessions.setView(session, view, query, total);
      return answer({ sessionId, view, total, revision: session.revision }, `${summary(session)}.`);
    }),
  );

  registerAppTool(
    server,
    "opengrid_describe",
    {
      title: "Describe the grid",
      description:
        "What the reader sees now: the view, the query of the view, the number of matches and how many rows are selected. No rows.",
      inputSchema: z.object({ sessionId: z.string() }),
      _meta: { ui: { visibility: ["model"] } },
    },
    guarded(async ({ sessionId }) => {
      const session = sessionOf(sessions, sessionId);
      return answer(
        {
          sessionId,
          source: session.source,
          columns: columnsOf(data, session.source),
          view: session.view,
          query: session.query,
          total: session.total,
          selected: session.selected.length,
        },
        `${summary(session)}.`,
      );
    }),
  );

  registerAppTool(
    server,
    "opengrid_selection",
    {
      title: "The selected rows",
      description: `The rows the reader selected, as objects { column: value }, in the grid's order. At most ${SELECTION_LIMIT} by default, ${SELECTION_MAX} at most; more are cut and said.`,
      inputSchema: z.object({
        sessionId: z.string(),
        limit: z.number().int().min(1).max(SELECTION_MAX).optional(),
      }),
      _meta: { ui: { visibility: ["model"] } },
    },
    guarded(async ({ sessionId, limit }) => {
      const session = sessionOf(sessions, sessionId);
      const wanted = session.selected.slice(0, limit ?? SELECTION_LIMIT);
      const rows = [];
      for (const [offset, count] of runsOf(wanted)) {
        rows.push(...rowsOf(data.execute({ ...session.query, offset, limit: count })));
      }
      const cut = session.selected.length - wanted.length;
      return answer(
        { sessionId, selected: session.selected.length, rows, truncated: cut > 0 },
        cut > 0
          ? `${rows.length} of ${session.selected.length} selected rows (cut at ${wanted.length}; ask with a larger limit, up to ${SELECTION_MAX}).\n${JSON.stringify(rows)}`
          : JSON.stringify(rows),
      );
    }),
  );

  // App only: the grid's provider. Rows reach the view, never the model.
  registerAppTool(
    server,
    "opengrid_query",
    {
      title: "Rows for the grid",
      description: "Runs one query of the grid in the chat (its window, a count, a group).",
      inputSchema: z.object({ sessionId: z.string(), query: z.record(z.string(), z.unknown()) }),
      _meta: { ui: { visibility: ["app"] } },
    },
    guarded(async ({ sessionId, query }) => {
      const session = sessionOf(sessions, sessionId);
      if (query.source !== session.source) {
        throw new DataError("validation", `the session's source is ${session.source}`, "query.source");
      }
      return { content: [{ type: "text", text: data.execute(query) }] };
    }),
  );

  // App only: the reader's changes in, the model's view out (E40).
  registerAppTool(
    server,
    "opengrid_sync",
    {
      title: "Sync the grid",
      description: "Reports the reader's change and answers with the session's view and revision.",
      inputSchema: z.object({
        sessionId: z.string(),
        revision: z.number().int().min(0),
        change: z
          .object({
            view: z.record(z.string(), z.unknown()).optional(),
            query: z.record(z.string(), z.unknown()).nullable().optional(),
            total: z.number().int().min(0).optional(),
            selected: z.array(z.number().int().min(0)).optional(),
          })
          .optional(),
      }),
      _meta: { ui: { visibility: ["app"] } },
    },
    guarded(async ({ sessionId, change }) => {
      const session = sessionOf(sessions, sessionId);
      if (change) {
        sessions.readerChanged(session, { ...change, query: change.query ?? undefined });
      }
      return answer({
        sessionId,
        revision: session.revision,
        origin: session.origin,
        view: session.view,
        total: session.total,
        summary: summary(session),
      });
    }),
  );
}
