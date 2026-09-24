# Prototype host page

The design prototype as a running page: an orders grid with search, toolbar,
facets, column menu and a selection column, German throughout, plus the two
things around it that are **the page's, not the element's** — saved views as
tabs, and a studio for the look.

## Running it

```console
just wasm-build            # engine module (once, or after engine changes)
just wasm-build-components # element module (after component changes)
just serve-demo            # the server root is the repository
```

Then open <http://127.0.0.1:8080/examples/prototype/>.

## What it shows

**Views are values.** A tab is `get_view(grid)` with a name on it; switching a
tab is `set_view(grid, saved)`. The dirty mark compares filters, facets and
grouping with the saved view; *Verwerfen* applies the saved one again; *Als
Ansicht speichern* stores the current view in `localStorage`. The whole tab bar
is about 70 lines of page code, ten of them the four views as data — no
element code. Clearing `localStorage` brings back the four prototype views.

**The look is custom properties.** The five presets (Base, Papier, Violett,
Orange, Dunkel) and every change in the studio set `--og-*` properties on the
element. The generated `theme.css` names only `opengrid-grid`: the table and the
pivot ship no stylesheet of their own. The page's own background (*Seite*) is
not a grid token and goes to the page.

**The data has edges on purpose.** The 50 rows of the prototype contain NULL
(`\N` in the CSV), the empty string, `-0.01`, `0.00` and `999999999.99`. NULL
and the empty string are different values and read differently (*(kein Wert)*,
*(leer)*); amounts are formatted on the exact text, never through a float.

## Deliberately left out

- The prototype's *Exportieren* and *Neue Bestellung* buttons. They did nothing
  there, and a button that does nothing is not an example.
- The web fonts. The presets name *Geist* and *IBM Plex Sans* and fall back to
  the system face when they are not installed — an example that loads fonts
  from a third party would send every visitor there.
