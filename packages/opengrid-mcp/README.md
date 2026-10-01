# @casoon/opengrid-mcp

[opengrid](https://github.com/casoon/opengrid)'s data grid as an
[MCP App](https://github.com/modelcontextprotocol/ext-apps): a chat client shows
the grid in the conversation, and the reader's keyboard and the model work on
**one** view. The model filters, sorts and groups by setting that view; the
reader does the same with the grid; each sees what the other did.

The rows stay with the server. The model gets columns, counts and — when the
reader hands them over — the selected rows, never a whole source.

## Install

Claude Desktop runs the server locally; nothing is hosted. In **Settings →
Developer → Edit Config**:

```json
{
  "mcpServers": {
    "opengrid": {
      "command": "npx",
      "args": ["-y", "@casoon/opengrid-mcp", "--config", "/path/to/opengrid-mcp.json"]
    }
  }
}
```

Restart the app, then ask for a source: *"Open the orders as a grid."* Claude
asks once whether it may show the app.

| Client | |
|---|---|
| Claude Desktop | local, as above |
| Claude Code | the tools work; a terminal shows no grid |
| claude.ai, ChatGPT | need a server reachable over HTTP (a custom connector); this package speaks stdio |

## Sources

The operator decides what exists, in `opengrid-mcp.json`. Paths are relative to
the file:

```json
{
  "sources": {
    "orders": {
      "csv": "data/orders.csv",
      "schema": "data/orders.schema.json",
      "title": "Orders",
      "columns": ["id", "customer", "country", "amount", "ordered_on"],
      "fields": ["id", "customer", "country", "amount", "qty", "ordered_on"]
    }
  }
}
```

| Key | |
|---|---|
| `csv`, `schema` | the data and its schema, as opengrid's `load_csv` reads them |
| `columns` | the grid's columns, in order — its `columns` attribute |
| `fields` | the fields any query may name; absent, every field of the schema |
| `title` | the grid's label |

The model names a source. It never names a path, writes SQL or reaches a
network, and a query naming a field outside `fields` is refused.

## Tools

| Tool | Called by | |
|---|---|---|
| `opengrid_open` | model | Opens a source as a grid in the chat. Answers with the session, the columns and the count — no rows. |
| `opengrid_set_view` | model | Sets the whole view (opengrid's `get_view` JSON). A view naming an unknown column is refused as a whole. Answers with the new count. |
| `opengrid_describe` | model | What the reader sees: view, query, count, size of the selection. No rows. |
| `opengrid_selection` | model | The selected rows as `{ column: value }`, 100 by default, 500 at most; more are cut and said. |
| `opengrid_query` | grid only | The grid's own queries — its window, its counts. |
| `opengrid_sync` | grid only | The reader's changes in, the model's view out. |

**The model's view reaches a grid already shown** through the server: it keeps
each session's view, and the grid asks for it when it becomes visible or
focused and every few seconds while it is. It is applied in one step and said
once, with the result: *"52 matches · Filtered by the assistant"*. The focus
stays where it was.

**A selection** is the grid's query plus the positions the reader selected
under it. *Analyse the selection* hands it to the model, which reads the rows
with `opengrid_selection`.

## How it runs in a client

MCP Apps hosts give a view a strict Content Security Policy: its own inline
code, nothing from elsewhere — and no WebAssembly. opengrid's elements are
WebAssembly, so this package carries them translated to JavaScript (wasm2js) and
inlined into one HTML file. The engine runs in the server, in Node.

## Limits

The grid only — no pivot, no tree. No editing: a surface through which a model
could change data needs its own approval path.

## Licence

MIT OR Apache-2.0, like opengrid.
