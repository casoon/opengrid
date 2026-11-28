---
title: Introduction
description: What opengrid is for, how to start in a few minutes, and what it deliberately leaves out.
order: 0
---

opengrid is a free data grid, table and pivot for the web, as Web Components. You put an element
on the page and hand it your rows; sorting, filtering, grouping, editing and export already work.

The aim is three things: **simple to use**, **work taken off you**, and **free**. A page that
shows tables should not have to write paging, sorting or export code, or pay for a pivot.

## How it takes the work off you

The product is one query engine, written in Rust. It runs as WebAssembly in the browser, natively
on a server, and against PostgreSQL, with the same query model and the same results everywhere.
Small data stays in the tab; data that does not fit stays on your server, and the grid asks it
for the window of rows it shows. A conformance suite of JSON cases pins what "the same" means.
[What sets it apart](what-sets-it-apart/) compares that with a grid written in JavaScript.

## Start

1. [Install](getting-started/installation/) `@casoon/opengrid`.
2. Put an element on the page and give it your rows: the [Quickstart](getting-started/quickstart/)
   needs one HTML file, and [Data you already have](guides/data-you-have/) shows how rows become a
   grid.
3. Use it from your framework: [React, Vue, Svelte and Solid](guides/frameworks/) have components,
   everything else, Angular included, uses `connect`.

Every framework has a [live demo site](https://og-vanilla.casoon.dev/) with the same demos and
their source.

## The three elements

| Element | Use it when |
|---|---|
| `<opengrid-table>` | The answer is **read**: a plain native `<table>`, ordinary copy and paste, browser find. |
| `<opengrid-grid>` | The answer is **worked with**: selection, editing, filtering, sorting, column control, virtualization or paging. |
| `<opengrid-pivot>` | Rows crossed with up to two column dimensions, with subtotals. |

All three ship in one module (387.8 KiB brotli; the grid alone is 357.6 KiB, a pivot-only build
135.0 KiB). The Cargo features `grid` and `pivot` exist for anyone who wants only one.

## Status

Pre-1.0. On npm are `@casoon/opengrid`, with the elements, the engine and the adapters for React,
Vue, Svelte and Solid, and `@casoon/opengrid-mcp`. The portable Rust crates and `opengrid-server`
are on crates.io. What each release changed is in
[`CHANGELOG.md`](https://github.com/casoon/opengrid/blob/main/CHANGELOG.md).

The public API is written down in [The public API](api/), and a test fails when it changes.
That freezes the list, not the design: it may still change before 1.0.

## What it leaves out

- **No calculated fields.** A column is stored or derived from a date part (year, month); there
  is no expression language.
- **A pivot is bounded**: two column dimensions, 256 generated columns, 10 000 rows, and
  131 072 cells. Over a limit you get an error with a sentence, never a silently truncated result.
- **Paging and virtualization are exclusive**, not combined.
- **No CDN build.** Angular, Lit and the rest use `connect` directly.
