// SlickGrid (6pac, MIT), the way its examples set up a grid over a DataView: the view owns the
// rows, sorting and filtering go through it, and the grid redraws when the view says its rows
// changed. Row height 28, no cell navigation or reordering (nothing here edits or drags).
import { SlickDataView, SlickGrid } from "slickgrid";
import "slickgrid/dist/styles/css/slick.grid.css";
import "slickgrid/dist/styles/css/slick-default-theme.css";

let grid;
let dataView;
let root;
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));
const rowsOf = () => [...(root?.querySelectorAll(".slick-viewport .slick-row") ?? [])];
const top = (row) => Number.parseFloat(row.style.top || "0");

export default {
  name: "slickgrid",
  async mount(container, rows, columns) {
    root = container;
    dataView = new SlickDataView();
    grid = new SlickGrid(
      container,
      dataView,
      columns.map((c) => ({ id: c.field, field: c.field, name: c.title, width: c.field === "customer" ? 200 : 140, sortable: true })),
      { rowHeight: 28, enableCellNavigation: false, enableColumnReorder: false, forceFitColumns: false, explicitInitialization: false },
    );
    dataView.onRowCountChanged.subscribe(() => {
      grid.updateRowCount();
      grid.render();
    });
    dataView.onRowsChanged.subscribe((_event, args) => {
      grid.invalidateRows(args.rows);
      grid.render();
    });
    dataView.beginUpdate();
    dataView.setItems(rows);
    dataView.endUpdate();
  },
  firstRow() {
    const first = rowsOf().sort((a, b) => top(a) - top(b))[0];
    const cell = first?.querySelector(".slick-cell.l0");
    return cell ? { id: digits(cell.textContent) } : null;
  },
  visibleCountries() {
    return rowsOf().map((row) => row.querySelector(".slick-cell.l2")?.textContent.trim());
  },
  async sort(field, direction) {
    const sign = direction === "desc" ? -1 : 1;
    dataView.sort((a, b) => (a[field] < b[field] ? -sign : a[field] > b[field] ? sign : 0), true);
  },
  async filter(field, value) {
    dataView.setFilter((item) => item[field] === value);
    dataView.refresh();
  },
  async clearFilter() {
    dataView.setFilter(() => true);
    dataView.refresh();
  },
  scroller() {
    return root.querySelector(".slick-viewport");
  },
  rowHeight() {
    return 28;
  },
};
