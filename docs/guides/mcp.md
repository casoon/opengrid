---
title: In a chat client (MCP)
description: The grid as an MCP App — the reader's keyboard and the model on one view.
order: 7
---

`@casoon/opengrid-mcp` brings `<opengrid-grid>` into chat clients that support
[MCP Apps](https://github.com/modelcontextprotocol/ext-apps). It is an adapter
over the public API, nothing in the grid knows about MCP. Installing it, its
sources and its tools are in the
[package README](https://github.com/casoon/opengrid/tree/main/packages/opengrid-mcp#readme).

## The view is the interface

Everything the reader chooses — sort, filters, columns, grouping, facets — is
one value, the [view](../api.md#the-view). The model works on the same value:
`opengrid_set_view` takes exactly what `get_view` returns, and `opengrid_describe`
answers with it. The server counts a view the model sets without a browser,
through the engine's [`view_query`](../api.md#exporting-the-view) — the same
translation the grid uses for `get_query`.

## Two directions

| | |
|---|---|
| Reader → model | Each `opengrid-view-change` and `opengrid-selection-change` goes to the server, and one line goes into the model's context: `Filter: country eq DE · 52 matches · 3 selected`. |
| Model → reader | The server keeps the session's view. The grid asks for it when it becomes visible or focused and every few seconds while it is, and applies it with [`set_view(host, view, { notice })`](../api.md#the-view): one query, one announcement, the focus where it was. |

## Rows

The grid's rows come through an app-only tool; the model never receives a
source. It receives rows only when the reader selects them and asks for an
analysis — as the grid's query plus the selected positions, read on the server.

## In the host's sandbox

A view runs under the host's Content Security Policy, which allows no
WebAssembly. The package therefore ships the elements translated with wasm2js,
built from the same Rust as `@casoon/opengrid`, inlined into one HTML file.
`tests/e2e/mcp-view.spec.js` runs that file in a sandboxed iframe under the
hosts' default policy, against the real server.
