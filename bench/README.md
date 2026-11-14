# bench

Measures opengrid over versions and, for comparison, other grids — the same rows, the same
columns, the same questions, the same way. Not part of a release: nothing here is published,
shipped or installed by CI. It has its own lockfile, as `site/` has.

```sh
cd bench && pnpm install
pnpm bench                          # every grid, 100,000 rows, 5 runs
pnpm bench --grids opengrid,aggrid --rows 100000 --runs 9
pnpm bench --throttle 4             # the CPU four times slower: a mid-range laptop
```

## What is measured

Each grid has a page of its own (`pages/<name>.html`), built by Vite and opened in a fresh
Chromium page for every run. The page generates the same deterministic rows
(`src/data.js`; the amounts are unique, so every grid sorts to the same first row), mounts the
grid, and then, in this order:

| Figure | From … to … |
|---|---|
| first rows | the rows are in memory … the first row is painted |
| sort | `amount` descending is asked for … the right first row is painted |
| filter | `country = DE` while sorted … the right first row, and only DE rows, are painted |
| undo | the filter and the sort are cleared … row 1 is painted |
| scroll | 120 steps of three rows, then 20 jumps of 2,000 rows: the frame times (p95, max, frames over 33 ms) |
| memory | the **whole browser process tree**, resident, minus an empty page's, once with the rows loaded and once after everything above — a Worker and WebAssembly count like everything else |

Every time is to a frame that **shows the right answer** — the check reads the DOM — plus the
frame after it, so a grid that has the answer and not yet the pixels has not answered. A figure
that is `—` was not reached within 30 s, and is reported as that, not left out.

## What is not measured

Accessibility, features (grouping, editing, pivot, export), how a grid looks, a first load over a
real network. A grid that does more is not slower for it here, and a grid that does less is not
faster; this is one question — sort, filter and scroll a large table — and it says nothing about
the others.

## Fairness, and where it can fail

- Each grid is set up **as its own documentation sets up a plain grid**, by an adapter
  (`src/adapters/<name>.js`), by me, from its documentation. An adapter can be wrong. Two were
  while this was written: a Tabulator at `height: "100%"` rendered every row (twelve seconds for
  20,000), and a timer that started after a synchronous mount gave negative times. **An adapter
  needs the eye of someone who knows the grid before a figure that names it is quoted.**
- One machine, one browser. The CPU can be throttled; that is not another computer.
- Results are written to `results/` and **stay there** (git-ignored): they name grids that are
  not ours, and the licence of a product that is not MIT may not allow publishing figures.

## Adding a grid

Copy `src/adapters/_template.js`, `pages/opengrid.html` and `src/entry-opengrid.js`, and follow
the rules at the top of the template. A grid whose licence does not allow it into this
repository goes into `src/adapters/local/` and `pages/local/` (both git-ignored) and is run
with the owner's own licence.
