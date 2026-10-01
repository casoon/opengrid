// The grid in the chat (#140): an MCP App view over @casoon/opengrid-mcp.
//
// The grid asks its rows through the app-only `opengrid_query`; what the reader
// does goes to the server through `opengrid_sync`, and so does what the model
// set (E40): the view asks when it becomes visible or focused, and every few
// seconds while it is, and applies a model's view with one announcement.
import { App } from "@modelcontextprotocol/ext-apps";

import { register, set_provider, get_query, set_view } from "./elements.js";

const TEXTS = {
  en: {
    byAssistant: "Filtered by the assistant",
    analyse: (count) => `Analyse the selection (${count})`,
    ask: (count, source) => `Analyse the ${count} selected rows of ${source}.`,
    waiting: "Waiting for the grid …",
  },
  de: {
    byAssistant: "Vom Assistenten gefiltert",
    analyse: (count) => `Auswahl analysieren (${count})`,
    ask: (count, source) => `Analysiere die ${count} ausgewählten Zeilen aus ${source}.`,
    waiting: "Das Grid wird geladen …",
  },
};
const SYNC_EVERY_MS = 2000;

register();

const app = new App({ name: "opengrid", version: "0.0.0" }, {}, { autoResize: true });
const main = document.querySelector(".app");
const button = document.querySelector("#analyse");
let words = TEXTS.en;
let session = null;
let revision = 0;
let selected = [];
let grid = null;
let applying = false;

function errorText(result) {
  try {
    return JSON.parse(result.content[0].text).error.message;
  } catch {
    return result.content?.[0]?.text ?? "the server did not answer";
  }
}

async function tool(name, args) {
  const result = await app.callServerTool({ name, arguments: args });
  if (result.isError) throw new Error(errorText(result));
  return result;
}

/** What the model hears in its context: one line, no rows. */
function tell(summary) {
  app.updateModelContext({ content: [{ type: "text", text: `opengrid (${session.source}): ${summary}` }] }).catch(() => {});
}

async function sync(change) {
  if (!session) return;
  const { structuredContent: answer } = await tool("opengrid_sync", { sessionId: session.sessionId, revision, change });
  if (answer.revision > revision && answer.origin === "model" && !change) {
    // The model's view: one query, one announcement, the focus where it is.
    applying = true;
    try {
      set_view(grid, answer.view, { notice: words.byAssistant });
    } finally {
      applying = false;
    }
  }
  revision = answer.revision;
  if (change) tell(answer.summary);
}

function showButton() {
  button.hidden = selected.length === 0;
  button.textContent = words.analyse(selected.length);
}

function start(opened) {
  session = opened;
  revision = opened.revision;
  main.querySelector("[data-waiting]")?.remove();
  grid = document.createElement("opengrid-grid");
  grid.setAttribute("label", opened.title);
  grid.setAttribute("datasource", opened.source);
  grid.setAttribute("columns", opened.columns.map((column) => column.name).join(","));
  grid.setAttribute("selection", "");
  grid.setAttribute("toolbar", "");
  set_provider(grid, {
    execute: async (queryJson) =>
      (await tool("opengrid_query", { sessionId: opened.sessionId, query: JSON.parse(queryJson) })).content[0].text,
  });
  grid.addEventListener("opengrid-view-change", (event) => {
    if (applying) return;
    sync({ view: event.detail.view, query: get_query(grid) ?? undefined }).catch(() => {});
  });
  grid.addEventListener("opengrid-selection-change", (event) => {
    selected = event.detail.rows;
    showButton();
    sync({ selected }).catch(() => {});
  });
  main.prepend(grid);
  if (opened.view && Object.keys(opened.view).length) {
    applying = true;
    try {
      set_view(grid, opened.view);
    } finally {
      applying = false;
    }
  }
}

button.addEventListener("click", async () => {
  if (!session || selected.length === 0) return;
  const text = words.ask(selected.length, session.source);
  await app.updateModelContext({
    content: [{ type: "text", text: `${text} Read them with opengrid_selection (sessionId ${session.sessionId}).` }],
  });
  await app.sendMessage({ role: "user", content: [{ type: "text", text }] });
});

/** The host's look: its dark or light theme, its language. */
function adopt(context) {
  if (!context) return;
  if (context.theme === "dark") document.documentElement.dataset.theme = "dark";
  else delete document.documentElement.dataset.theme;
  grid?.setAttribute("theme", context.theme === "dark" ? "dark" : "base");
  if (context.locale) {
    const lang = context.locale.toLowerCase().startsWith("de") ? "de" : "en";
    words = TEXTS[lang];
    document.documentElement.lang = lang;
    showButton();
  }
}

app.ontoolresult = (params) => {
  if (params.structuredContent?.sessionId && !session) {
    start(params.structuredContent);
    adopt(app.getHostContext());
  }
};
app.onhostcontextchanged = (context) => adopt({ ...app.getHostContext(), ...context });

const visible = () => document.visibilityState === "visible";
setInterval(() => visible() && sync().catch(() => {}), SYNC_EVERY_MS);
document.addEventListener("visibilitychange", () => visible() && sync().catch(() => {}));
window.addEventListener("focus", () => sync().catch(() => {}));

await app.connect();
adopt(app.getHostContext());
showButton();
