// w2ui (MIT): its grid with virtual scrolling, which it uses for any record set larger than a
// screen. w2ui keys a record by `recid`, so the adapter adds one — the cost of that copy is in
// the first-rows figure, as it would be in a page. Sorting and searching are the grid's own.
import { w2grid } from "w2ui";
import "w2ui/w2ui-2.0.min.css";

let grid;
let root;
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));
const rowsOf = () => [...(root?.querySelectorAll(".w2ui-grid-records tr[recid], .w2ui-grid-records tr[index]") ?? [])].filter((tr) => tr.querySelector("td[col]"));
const top = (tr) => tr.getBoundingClientRect().top;

export default {
  name: "w2ui",
  async mount(container, rows, columns) {
    root = container;
    const records = new Array(rows.length);
    for (let i = 0; i < rows.length; i++) records[i] = { recid: rows[i].id, ...rows[i] };
    grid = new w2grid({
      name: "bench",
      box: container,
      recordHeight: 28,
      show: { toolbar: false, footer: false, columnHeaders: true, lineNumbers: false },
      columns: columns.map((c) => ({ field: c.field, text: c.title, size: c.field === "customer" ? "200px" : "140px", sortable: true, searchable: true })),
      records,
    });
  },
  firstRow() {
    const first = rowsOf().sort((a, b) => top(a) - top(b))[0];
    const cell = first?.querySelector('td[col="0"]');
    return cell ? { id: digits(cell.textContent) } : null;
  },
  visibleCountries() {
    return rowsOf().map((tr) => tr.querySelector('td[col="2"]')?.textContent.trim());
  },
  async sort(field, direction) {
    grid.sort(field, direction);
  },
  async filter(field, value) {
    grid.search([{ field, type: "text", operator: "is", value }], "AND");
  },
  async clearFilter() {
    grid.searchReset();
    grid.sort("id", "asc");
  },
  scroller() {
    return root.querySelector(".w2ui-grid-records");
  },
  rowHeight() {
    return 28;
  },
};
