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

## In practice

<figure>
<video controls preload="none" playsinline width="1920" height="1080" poster="../../../videos/opengrid-mcp-poster.webp" aria-label="Five requests to opengrid through MCP, as a chat with tool calls and the grid beside it" style="width:100%;height:auto">
<source src="../../../videos/opengrid-mcp.mp4" type="video/mp4">
</video>
<figcaption>Five requests to an assistant with the opengrid server, 1:45, no sound. The chat is staged; every tool call and result is real output of the server, shortened. The grids are screenshots of the package's view after those calls; the selection in the third request was made by keyboard in the grid.</figcaption>
</figure>

The source is `orders`: 240 rows, whose CSV also holds `margin` and `note` — fields the
operator left out of `fields`.

1. **Open a source:** "Open the orders as a grid." `opengrid_open` answers with the session,
   the six columns and `total: 240` — no rows.
2. **Filter and sort:** "Only Germany, largest amounts first." `opengrid_set_view` with a
   filter on `country` and a sort on `amount` answers `Filter: country eq DE · Sort: amount desc
   · 88 matches`; the grid applies it and says "Filtered by the assistant".
3. **The reader's selection:** the reader selects the four largest German orders in the grid
   and asks "What's odd about the rows I selected?" `opengrid_selection` returns exactly those
   four rows: two identical orders, an amount with a quantity of 0, and a date after today.
4. **Group and sum:** "Revenue per country." `opengrid_set_view` groups by `country` with
   `amount` summed; the model gets `Grouped by: country · 240 matches`, the grid shows six
   group sums and a total. The new view clears the selection.
5. **A field it may not name:** "Which orders have the lowest margin?" `opengrid_set_view` with
   a sort on `margin` is refused: `sort: margin is not a column of this grid`.
   `opengrid_describe` confirms the view is unchanged.

## The view is the interface

Everything the reader chooses — sort, filters, columns, grouping, facets — is
one value, the [view](../../api/#the-view). The model works on the same value:
`opengrid_set_view` takes exactly what `get_view` returns, and `opengrid_describe`
answers with it. The server counts a view the model sets without a browser,
through the engine's [`view_query`](../../api/#exporting-the-view) — the same
translation the grid uses for `get_query`.

## Two directions

| | |
|---|---|
| Reader → model | Each `opengrid-view-change` and `opengrid-selection-change` goes to the server, and one line goes into the model's context: `Filter: country eq DE · 52 matches · 3 selected`. |
| Model → reader | The server keeps the session's view. The grid asks for it when it becomes visible or focused and every few seconds while it is, and applies it with [`set_view(host, view, { notice })`](../../api/#the-view): one query, one announcement, the focus where it was. |

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
