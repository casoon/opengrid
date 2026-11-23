# bench

Measures opengrid over versions and, for comparison, other grids — the same rows, the same
columns, the same questions, the same way. Not part of a release: nothing here is published,
shipped or installed by CI. It has its own lockfile, as `site/` has.

```sh
cd bench && pnpm install
pnpm bench                          # every grid, 100,000 rows, 5 runs
pnpm bench --grids opengrid,aggrid --rows 100000 --runs 9
pnpm bench --throttle 4             # the main thread four times slower (not a Worker)
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

## This checkout against a release, and the stored figures

`just bench-local` builds the package from this checkout and measures it (`BENCH_LOCAL=1` aliases
`@casoon/opengrid` to `packages/opengrid`), instead of the version pinned in `package.json`.
`baselines/` holds opengrid's own figures (the three opengrid pages, 100,000 and 1,000,000 rows,
median of five, Apple M4 Pro) for released 0.17.0 and for `main` on 17 November 2026, in the
tool's format; `--baseline baselines/<file>.json` compares with them. A baseline only compares on
the machine, rows and throttle it was made with: on another computer, make your own first
(`just bench-self` on the release, keep the file, then `just bench-local --baseline`). When a
`main-2026-11-17-x4-*` and `-x6-*` are the same two browser pages (objects and CSV; not server mode) with the CPU four and six times slower (`--throttle 4`, `--throttle 6`): compare with the same `--throttle`. The throttle slows the
page's main thread, not a Worker (Chromium refuses the emulation there), so opengrid's engine ran
at full speed in them: they compare a change in the page's own work, and are not a mid-range
laptop. When a
release or a deliberate change is accepted, add its files here. Only opengrid's figures belong in
`baselines/`; a run that includes other grids stays in `results/`.

On a GitHub Linux runner: `gh workflow run bench.yml -f rows=100000 -f throttle=1` (workflow
`bench.yml`, by hand only); the result is an artifact. `-f grids=opengrid,aggrid,…` picks the
pages (opengrid's two by default); only pages in the repository run there, so only the MIT grids.
The log and the artifact of a public repository can be read by anyone signed in to GitHub.

On a pull request, the label `bench` measures the base and the pull request one after the other
in the same job (opengrid's two pages, 100,000 rows, median of nine), so on the same CPU, and
marks every figure the pull request makes worse; the job fails on one, and the table is in the
job's summary. It runs again on every push while the label is on. A noisy runner can mark a
figure that a second run does not: read the range before believing it. A shared runner is a noisy machine, so its
figures compare with another run of that job, not with the Mac baselines;
`baselines/linux-runner-2026-11-17-100000.json` is one such run (AMD EPYC runner, 100,000 rows, a
frame of 16.7 ms everywhere it should be, first rows 146 ms), and it only compares when a later run
lands on the same CPU model.

For a slower machine that includes the Worker, give the job a CPU quota: `-f cpu=25` runs the
whole tool — node, the browser, the page and opengrid's Worker — in a cgroup that may use a
quarter of one core (`systemd-run -p CPUQuota=25%`). The kernel enforces it on every thread,
which Chrome's `--throttle` cannot; the runner reads the quota it ran under, prints it, writes it
into the result as `cpuLimit` and compares only with a run under the same quota. It limits CPU
time, not cores or memory: a quarter core is the whole tool on one slow core, not a laptop model.
On 23 November 2026 (AMD EPYC, 100,000 objects, median of five) first rows went from 210 ms
without a quota to 286 ms at `cpu=100` and 1,990 ms at `cpu=25`, the filter from 50 to 520 ms.

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
- One machine, one browser. The CPU can be throttled; that is not another computer, and the
  throttle reaches the page's main thread only, not a Worker.
- Results are written to `results/` and **stay there** (git-ignored). The figures of the MIT-licensed
  grids are published in the performance guide; a product that is not MIT stays local, and its
  figures are not published unless its licence says they may be.

## Adding a grid

Copy `src/adapters/_template.js`, `pages/opengrid.html` and `src/entry-opengrid.js`, and follow
the rules at the top of the template. A grid whose licence does not allow it into this
repository goes into `src/adapters/local/` and `pages/local/` (both git-ignored) and is run
with the owner's own licence.
