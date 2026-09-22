# Grid demo (100k)

A page that loads a data set through the local WASM engine, is fully operable
from the keyboard, and holds a DOM window of about 40 rows while presenting
100,000 logical ones (row recycling). The e2e suite stays small on purpose;
**this demo is the check at scale.**

## Running it

```console
just wasm-build            # engine module (once, or after engine changes)
just wasm-build-components # element module (after component changes)
just serve-demo            # the server root is the repository
```

Then open <http://127.0.0.1:8080/examples/grid-demo/>.

## Optional: 100,000 rows

Without the data set the demo falls back to the small conformance one. The 100k
file lives under a gitignored path (`target/`) and is not checked in:

```console
mkdir -p target/grid-demo
cargo run -p xtask -- gen-orders --rows 100000 --seed 1 --out target/grid-demo/orders-100k.csv
```

## Keyboard

Arrow keys, `Home`/`End`, `Ctrl+Home`/`Ctrl+End`, `PageUp`/`PageDown`; `Enter`
or `Space` on a header cell sorts; `Shift`+`Enter`/`Space` adds or removes that
column as a further sort key; `Escape` jumps to the first cell; `Tab` and
`Shift+Tab` leave the grid. `Ctrl+End` loads the last logical row
(`aria-rowindex=100001`), renders it and focuses it.

The header shows the **direction** as ▲/▼, and with a multi-column sort also the
position (1, 2). Both marks are `aria-hidden`: assistive technology reads the
direction from `aria-sort`, so it is not announced twice.

A focused cell whose value is wider than its column **unfolds** and shows the
value in full over the rows below, folding back as focus moves on. The full
value was always in the DOM — only the display was truncated.

## Filter row and status line

Above the table sits a type-aware filter row (`part="filter"`): one operator
`<select>` and one value `<input>` per column, both with an `aria-label`. `Enter`
in the input applies the `and` filter, "Clear" empties it. The select shows
readable labels ("greater or equal") while its `value` stays the wire token
(`gte`), so the query is unaffected by the wording.

Below it is the **status line** (`part="status"`, `role="status"`,
`aria-live="polite"`) — the grid's only live region. It carries all four states:
"Loading …", "N matches", "No matches" and, on a failed query, "The data could
not be loaded: …". An error does **not** replace the grid: table, rows and
keyboard operation stay, only the line changes. Scrolling deliberately announces
nothing, or the live region would chatter once per frame.

## The language of the texts

The component's built-in texts are **English** and marked `lang="en"`. A page
overrides them with `set_texts(host, …)`, and this demo exposes
`window.opengrid` in the console so the states that have no UI of their own —
loading, error, another language — can be triggered by hand.

The rule that matters: a language is declared on the nodes carrying the
component's *own* words, never on the data. Column names and cell values are the
page's, in the page's language.

## Theming

Custom properties on the host (`--grid-row-height`, `--grid-header-height`,
`--grid-border-color`, `--grid-focus-width`, `--grid-filter-height`,
`--grid-status-height`) and `::part(…)` for structure and marks. A theme cannot
switch off the focus ring, the system colours under `forced-colors`, or
`prefers-reduced-motion`.

## Virtualization

The `<tbody>` is the sizer (`height = total_count * 32px`), the pool rows are
`position: absolute` and placed with `translateY(row * 32px)`, and `<thead>` is
`position: sticky`. Scrolling reloads only the window (`limit=40`,
`offset=window start`) and recycles the existing rows — no new nodes appear. The
row holding the focused cell is never recycled, so focus survives scrolling.

### Measured (headless Chrome, 100,000 rows, 5 columns)

A scroll across the full height in 300 steps (`requestAnimationFrame`, each step
setting `scrollTop`):

| Metric | Value |
|---|---|
| DOM rows before / after | 40 / 40 (the pool, constant) |
| DOM cells before / after | 200 / 200 |
| `aria-rowcount` | 100001 |
| Steps | 300 |
| Duration | 5000 ms (≈ 16.7 ms/step) |
| FPS | 60 |
| `Ctrl+End` | focuses `data-row=99999`, `aria-rowindex=100001` |

## Default sort

Paging needs a total order (rule S6: an `offset` without a `sort` is a
validation error). So the grid sorts by its first column ascending (`id`) at
startup, and falls back to that whenever the sort is reset.
