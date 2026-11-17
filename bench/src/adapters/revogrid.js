// RevoGrid (MIT), a virtual data grid web component. Sorting is the grid's own
// (`updateColumnSorting`). **Filtering is not**: RevoGrid's filter plugin is built around its
// filter panel, so the adapter does what an application does without it — hands the grid the
// filtered rows as its new source. That makes this figure a different thing from the others'
// (the filtering is a plain `Array.filter` in this file, and the grid redraws), and it is
// reported with that caveat, not hidden.
import { defineCustomElements } from "@revolist/revogrid/loader";

defineCustomElements();

let el;
let allRows;
let root;
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));
const rowsOf = () => [...(root?.querySelectorAll(".rgRow") ?? [])];
const indexOf = (row) => Number(row.getAttribute("data-rgrow"));

export default {
  name: "revogrid",
  async mount(container, rows, columns) {
    root = container;
    allRows = rows;
    el = document.createElement("revo-grid");
    el.style.height = "100%";
    el.rowSize = 28;
    el.theme = "compact";
    el.columns = columns.map((c) => ({ prop: c.field, name: c.title, size: c.field === "customer" ? 200 : 140, sortable: true }));
    el.source = rows;
    container.append(el);
  },
  firstRow() {
    const first = rowsOf().sort((a, b) => indexOf(a) - indexOf(b))[0];
    const cell = first?.querySelector('.rgCell[data-rgcol="0"]');
    return cell ? { id: digits(cell.textContent) } : null;
  },
  visibleCountries() {
    return rowsOf().map((row) => row.querySelector('.rgCell[data-rgcol="2"]')?.textContent.trim());
  },
  async sort(field, direction) {
    await el.updateColumnSorting({ prop: field }, direction, false);
  },
  async filter(field, value) {
    el.source = allRows.filter((row) => row[field] === value);
  },
  async clearFilter() {
    el.source = allRows;
    await el.updateColumnSorting({ prop: "id" }, "asc", false);
  },
  scroller() {
    return [...root.querySelectorAll("*")].find((e) => e.scrollHeight > e.clientHeight + 50 && /auto|scroll/.test(getComputedStyle(e).overflowY));
  },
  rowHeight() {
    return 28;
  },
};
