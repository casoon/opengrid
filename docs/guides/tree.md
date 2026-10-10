---
title: A tree in the grid
description: Rows that name their parent shown as a tree — loaded a level at a time, filtered with their context, selected by key, summed over subtrees and exported flat.
order: 9
---

A tree is a mode of `<opengrid-grid>`, not an element of its own: rows that name their parent
in a field are shown as a hierarchy, and the grid becomes a `treegrid`. Everything else — the
filter row, the column menu, formats, texts, the view — works as it does without one. The
attribute and every rule are in [The public API → Tree](../../api/#tree); this page is about
using it.

## From a parent field

```html
<opengrid-grid label="Sales organisation" datasource="reps"
               columns="name,region,revenue" tree="manager_id" tree-key="id"></opengrid-grid>
```

`tree` names the field that holds a row's parent; `tree-key` the field it refers to (`id` when
absent). A row whose parent is NULL is a root. A row whose parent does not exist is a root too —
an **orphan** — and the status line says how many there are, once: nothing disappears quietly.
A key twice, or parents that lead in a circle, is an error with a sentence that names the row.

The key does not have to be one of the `columns`: the grid asks for it anyway, since a node is
named by it.

## A level at a time

The grid loads the roots first, and a node's children when the reader opens it — each level
with every node's child count, so a leaf offers nothing to open and nothing is asked twice. On a
node's first cell `→` opens it or goes to its first child, `←` closes it or goes to its parent; a
click on the chevron does the same. A level holds at most 10 000 nodes; a larger one is refused
with a sentence that asks for a filter.

Rows say where they are: `aria-level`, `aria-posinset` and `aria-setsize`, `aria-expanded` where
there is something to open, `aria-busy` while the children load. Opening and closing is said
once, with the result.

## Filtering keeps the context

A filter shows the matches **and the path to them**: an ancestor that does not match is shown as
context — muted, and described with `treeContext` — so a match never floats without its place.
The status line counts the matches, not the context. Sorting orders siblings; the hierarchy is
never broken by it.

## The selection names nodes

In a tree the selection is by key: it stays when nodes close over it and when the grid is sorted
or filtered. `opengrid-selection-change` carries `keys` beside the positions, and `count` counts
every selected node. A node is selected by itself, never with its subtree.

## Summaries over subtrees

A column's aggregate — from `set_columns`, the column menu or the view, as in groups — is shown
on every node **with children** as its subtree's summary, beside the node's own value:

```js
loader.module.set_columns(grid, { revenue: { aggregate: "sum" } });
```

```text
Sales      EU      0 · Σ 223
  North    EU    100 · Σ 133
    Alice  EU     30 · Σ 33
```

The summary is over the node and all its descendants, from the raw rows — never from the
subtotals below it — and with a filter over the **matches** only: context does not count. A
screen reader hears `subtreeCell`, "100. Sum of the subtree: 133" — or, for a node without a value of its own, `subtreeOnly`, "Sum of the subtree: 133". A leaf shows its own value.

## Exporting the tree

`get_query()` on a tree asks for **every node of the reader's view**, not only the open ones,
depth-first — so the export is the tree, not what happened to be expanded:

```js
const query = loader.module.get_query(grid);
const blob = await exportRows(provider, query, { format: "csv" });
```

Each row gets `level`, `path` — the keys from the root down, `1 / 2 / 4` in CSV and XLSX, an
array in JSON — and, with a filter, `match`. Details are in [Exporting](../export/).

## On a server

Every source answers a tree: one that can do it itself, and any other through the rows of the
tree's scope, which the engine then answers. A server's row filter applies to the tree as a
whole — another tenant's row is never a match, never context and never a parent — and to its
exports alike.

## What a tree does not do

- **Group:** a tree and `group-by` exclude each other; + Group says so.
- **Page:** `page-size` beside `tree` is refused — a tree scrolls.
- **Edit:** an edit would be reported against a display position, which moves on every toggle.
