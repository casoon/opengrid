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

## opengrid over versions

`pnpm bench:self` runs the two opengrid pages only (objects handed over, and CSV bytes handed
over). To see whether a version is slower than the one before: run it with the earlier version
(`pnpm add @casoon/opengrid@<earlier>`), keep the file in `results/`, install the version under
test, and run again with the file as the baseline:

```sh
pnpm bench:self --rows 100000 --runs 9 --baseline results/<earlier>.json --fail-on-regress
```

A figure is marked when it is more than `--tolerance` percent (20) worse **and** worse by more
than a frame (16.7 ms) or 10 MiB; the exit code is 1 then. The baseline has to come from the same
machine, the same rows and the same throttle, or nothing is compared (exit 2). The sizes of the
modules and the memory of the engine are peakmark's (`just measure-modules`), not this.

## Server mode

`pnpm bench --grids opengrid-server --rows 1000000 [--db postgres]` measures the same page with
the rows **in a database** behind `opengrid-server` (`crates/opengrid-server/examples/bench_server.rs`,
started and stopped by `run.mjs`; the first run for a size writes the table, which takes minutes
for ten million rows; `--db postgres` takes the database `DATABASE_URL` names). The page holds no
rows, asks with `createRestProvider`, and the network is the loopback: this is the server and the
database, not a network. The count cache of the server is on (its default, 5 s), as a reader
would have it. Indexes as a page tuned for these requests would have them: the key `id` and
`(country, amount)`; sorting by `amount` alone has none, and sorts the table.

What is added to the table of figures: the time from moving the scroll bar to a viewport whose
rows all show their own data (**step**: 120 moves of three rows; **jump**: 20 moves of 2,000 rows;
**middle**: one drag to the middle of the table), the **requests** and **KiB answered** over those
141 moves (counted from outside, by Playwright), and the memory of the server process.

The backend an application writes for it is that example: about fifteen lines of builder (a
token, one source, the six fields it may read, a row filter if there is one) around the
connector. What `opengrid-server` checks on every request: the bearer token, the fields against
the allowed list, the mandatory row filter, the limits (rows per window, query time), CORS for
one origin. Other grids' server modes are not in this tool yet; their backends would be measured
and described here the same way.

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
| group | `country` is grouped, with the sum of `amount`: … every country's header is painted with the right row count and the right sum (to the cent) — only for a grid whose adapter has `group()`; others say `—` |
| ungroup | the grouping is removed … row 1 is painted |
| scroll | 120 steps of three rows, then 20 jumps of 2,000 rows: the frame times (p95, max, frames over 33 ms) |
| KiB fetched, KiB wasm | every file the page requested, **from the built files** and brotli-compressed as a server would send them (not the wire, so a server that compresses or not changes nothing), and the WebAssembly among them, which the browser then compiles |
| memory | the **whole browser process tree**, resident, minus an empty page's, once with the rows loaded and once after everything above — a Worker and WebAssembly count like everything else |

The amounts are unique up to ten million rows (a prime of 10,000,019), so every grid sorts to the same first row at every size here. Every time is to a frame that **shows the right answer** — the check reads the DOM — plus the
frame after it, so a grid that has the answer and not yet the pixels has not answered. A figure
that is `—` was not reached within 30 s, and is reported as that, not left out.

## What is not measured

Accessibility, features beyond sorting, filtering, scrolling and one grouping (editing, pivot,
export), how a grid looks, a first load over a real network. A grid that does more is not slower
for it here, and a grid that does less is not faster; this is one question — sort, filter, group
and scroll a large table — and it says nothing about the others.

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
