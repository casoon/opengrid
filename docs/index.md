---
title: Introduction
description: What opengrid is, what is in the repository today, and what V1 deliberately leaves out.
order: 0
---

opengrid is a portable Rust data and query engine with client/server execution, and on top of
it an accessible **data grid**, **table** and **pivot** delivered as Web Components.

The product is the engine, not a JavaScript grid. The same query model, the same semantics and
the same results run in WebAssembly in a browser, natively on a server, and against PostgreSQL.
A conformance suite of JSON cases pins what "the same" means.

## Status

Pre-1.0 and **not published**: the workspace version is `0.0.0`, there is no package on npm or
crates.io yet, and `CHANGELOG.md` has no release entry. To use it today, build it from the
repository — see [Installation](getting-started/installation/).

The public API is written down in [The public API](api/), and a test fails when it changes.
That freezes the list, not the design: it may still change before 1.0.

## The three elements

| Element | Use it when |
|---|---|
| `<opengrid-table>` | The answer is **read**: a plain native `<table>`, ordinary copy and paste, browser find. |
| `<opengrid-grid>` | The answer is **worked with**: selection, editing, filtering, sorting, column control, virtualization or paging. |
| `<opengrid-pivot>` | Rows crossed with one column dimension, with subtotals. |

All three ship in one module. The Cargo features `grid` and `pivot` exist for anyone who wants
only one. In React, Vue and Svelte they come as components — see
[Frameworks](guides/frameworks/).

## What V1 deliberately does not do

- **No calculated fields.** A column is stored or derived from a date part (year, month); there
  is no expression language.
- **One column dimension in a pivot**, at most 256 generated columns and 2 000 rows. Over a
  limit you get an error with a sentence, never a silently truncated result.
- **Paging and virtualization are exclusive**, not combined.
- **No CDN build.** Adapters exist for React, Vue and Svelte; Angular, Lit and the rest use
  `connect` directly.
