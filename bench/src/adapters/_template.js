// The shape of an adapter. Copy to `src/adapters/local/<name>.js` for a grid whose licence does
// not let it into this repository (that folder is git-ignored), add `pages/local/<name>.html`
// and `src/local/entry-<name>.js` like the committed ones, and run `pnpm bench --grids <name>`.
//
// Rules, which are the whole of what makes a comparison fair:
//  - The grid shows the rows it is given, the six columns of `src/data.js`, sorted by `id`.
//  - Virtualisation is on. Row height 28 px where the grid lets you say so.
//  - Set the grid up as **its own documentation** sets up a plain grid, and read what it says
//    about large data. A setting that makes it slow by accident is a mistake of ours, not a
//    result (a Tabulator at `height: "100%"` took twelve seconds for 20,000 rows instead of 27 ms).
//  - Animations off: a row that glides is not a slow row.
//  - `firstRow()` reads what is **on screen**, from the DOM, and returns `{ id }` as a number.

export default {
  name: "template",
  async mount(container, rows, columns) {},
  firstRow() { return null; },
  visibleCountries() { return []; },
  async sort(field, direction) {},
  async filter(field, value) {},
  async clearFilter() {},
  scroller() { return null; },
  rowHeight() { return 28; },
};
