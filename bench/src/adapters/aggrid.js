// AG Grid Community (MIT), as its documentation sets up a plain grid: rows in, columns with
// sorting and the text filter on, the row height the other grids use, animations off so that a
// moving row is not counted as a slow one.
import { AllCommunityModule, createGrid, ModuleRegistry } from "ag-grid-community";

ModuleRegistry.registerModules([AllCommunityModule]);

let api;
let root;
const cellOf = (rowIndex, colId) => root?.querySelector(`.ag-row[row-index="${rowIndex}"] [col-id="${colId}"]`);
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));

export default {
  name: "aggrid",
  async mount(container, rows, columns) {
    root = container;
    api = createGrid(container, {
      rowData: rows,
      columnDefs: columns.map((c) => ({
        field: c.field,
        headerName: c.title,
        sortable: true,
        filter: c.type === "number" ? "agNumberColumnFilter" : "agTextColumnFilter",
        width: c.field === "customer" ? 200 : 140,
      })),
      rowHeight: 28,
      headerHeight: 32,
      animateRows: false,
      suppressCellFocus: true,
      getRowId: (p) => String(p.data.id),
    });
  },
  firstRow() {
    const cell = cellOf(0, "id");
    return cell ? { id: digits(cell.textContent) } : null;
  },
  visibleCountries() {
    return [...(root?.querySelectorAll('.ag-row [col-id="country"]') ?? [])].map((c) => c.textContent.trim());
  },
  async sort(field, direction) {
    api.applyColumnState({ state: [{ colId: field, sort: direction }], defaultState: { sort: null } });
  },
  async filter(field, value) {
    api.setFilterModel({ [field]: { filterType: "text", type: "equals", filter: value } });
  },
  async clearFilter() {
    api.setFilterModel(null);
  },
  scroller() {
    return [...root.querySelectorAll("[class*=viewport]")].find((el) => el.scrollHeight > el.clientHeight + 50);
  },
  rowHeight() {
    return 28;
  },
};
