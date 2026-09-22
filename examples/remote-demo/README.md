# Remote demo (browser → server)

The same `<opengrid-grid>` as the grid demo, but the rows arrive over
`POST /query/orders` from a running `opengrid-server`. The browser sends the
query AST, never SQL.

## Running it

Two processes, two ports:

```console
just wasm-build-components                                          # element module
cargo run -p opengrid-server -- examples/remote-demo/opengrid.toml  # :8081
just serve-demo                                                     # :8080
```

Then open <http://127.0.0.1:8080/examples/remote-demo/>.

## What the page shows

Three radio buttons switch the **token** — that is the only thing the page
changes. The server appends a **mandatory filter** from the token's context to
every query, so the two tokens see different rows:

| Token | Context | Result |
|---|---|---|
| `demo-token-de` | `country = "DE"` | 16 rows, all DE |
| `demo-token-fr` | `country = "FR"` | 10 rows, all FR |
| none | — | `401`, status line: "The data could not be loaded: a valid bearer token is required" |

The error case is the interesting one. The error shape carries the server's own
sentence all the way into the grid's status line, and the grid **stays where it
is** — table, rows and keyboard operation keep working. That is deliberate: a
transient failure should not throw away the context the user was working in.

## What the server does not allow

- A column outside `allowed_fields` (`note`, `flag`, `ratio`, `created_at`)
  simply **does not exist** for a client: the answer is
  `422 unknown field "note"`, the same message a typo gets. A client cannot tell
  the difference, which is the point.
- The mandatory filter cannot be worked around: ask the DE token for
  `country = FR` and you get zero rows, not the French ones.
- CORS is **off** by default. `allowed_origins` lists origins one by one; no
  wildcard, because a wildcard together with a bearer token would authorise
  every window the user happens to have open.

## With 100,000 rows

Point `path` in `opengrid.toml` at `../../target/grid-demo/orders-100k.csv`
(generating the data set is described in `examples/grid-demo/README.md`) and
adjust `allowed_fields` and `row_filter` to that schema.
