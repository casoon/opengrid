// Tabulator (MIT) with its virtual DOM, which is its default for a fixed height. The layout is
// `fitColumns`: the fit-to-data layouts measure every row's content, which for a hundred thousand
// rows blocks the page for minutes — the documentation says as much for large data sets.
import { TabulatorFull as Tabulator } from "tabulator-tables";
import "tabulator-tables/dist/css/tabulator.min.css";

let table;
let root;
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));
const rowsOf = () => [...(root?.querySelectorAll(".tabulator-tableholder .tabulator-row") ?? [])];

export default {
  name: "tabulator",
  async mount(container, rows, columns) {
    root = container;
    await new Promise((built) => {
      table = new Tabulator(container, {
        data: rows,
        // A height in pixels: with a percentage Tabulator cannot tell how many rows fit and renders
        // them all (20,000 rows took 12 s instead of 27 ms in a trial).
        height: 600,
        index: "id",
        layout: "fitColumns",
        rowHeight: 28,
        columns: columns.map((c) => ({
          title: c.title,
          field: c.field,
          sorter: c.type === "text" ? "string" : "number",
          headerFilter: false,
        })),
      });
      table.on("tableBuilt", built);
    });
  },
  firstRow() {
    const first = rowsOf().sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top)[0];
    const cell = first?.querySelector('[tabulator-field="id"]');
    return cell ? { id: digits(cell.textContent) } : null;
  },
  visibleCountries() {
    return rowsOf().map((row) => row.querySelector('[tabulator-field="country"]')?.textContent.trim());
  },
  async sort(field, direction) {
    await table.setSort(field, direction);
  },
  async filter(field, value) {
    await table.setFilter(field, "=", value);
  },
  async clearFilter() {
    await table.clearFilter();
  },
  scroller() {
    return root.querySelector(".tabulator-tableholder");
  },
  rowHeight() {
    return 28;
  },
};
