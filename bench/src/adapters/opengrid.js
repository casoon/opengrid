// opengrid, as a page installs it from npm: the element, a worker provider, the rows handed over
// as the same objects every grid gets. `amount` is a float64 here, as it is a JS number in every
// other grid; the engine's exact decimal is a feature the others do not have, and not measured.
import { createWorkerProvider, loadOpengrid } from "@casoon/opengrid";

const SCHEMA = JSON.stringify({
  fields: [
    { name: "id", type: "int64", nullable: false },
    { name: "customer", type: "utf8", nullable: true },
    { name: "country", type: "utf8", nullable: true },
    { name: "amount", type: "float64", nullable: true },
    { name: "qty", type: "int64", nullable: true },
    { name: "ordered_on", type: "date", nullable: true },
  ],
});

let host;
let module;
let state = { sort: [{ field: "id", direction: "asc" }], filters: [] };
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));
const rowsOf = () => [...host.shadowRoot.querySelectorAll("tbody tr")].filter((tr) => tr.querySelector("td[data-row]"));

export default {
  name: "opengrid",
  async mount(container, rows, columns) {
    ({ module } = await loadOpengrid());
    host = document.createElement("opengrid-grid");
    host.setAttribute("label", "Orders");
    host.setAttribute("datasource", "orders");
    host.setAttribute("columns", columns.map((c) => c.field).join(","));
    host.style.display = "block";
    host.style.height = "100%";
    container.append(host);
    const provider = createWorkerProvider();
    await provider.loadRows("orders", rows, SCHEMA);
    module.set_provider(host, provider);
  },
  firstRow() {
    if (!host?.shadowRoot) return null;
    const first = rowsOf().sort((a, b) => Number(a.querySelector("td[data-row]").dataset.row) - Number(b.querySelector("td[data-row]").dataset.row))[0];
    const cell = first?.querySelectorAll("td")[0];
    return cell ? { id: digits(cell.textContent) } : null;
  },
  visibleCountries() {
    if (!host?.shadowRoot) return [];
    return rowsOf().map((tr) => tr.querySelectorAll("td")[2]?.textContent.trim());
  },
  async sort(field, direction) {
    state = { ...state, sort: [{ field, direction }] };
    module.set_view(host, { sort: state.sort, filters: state.filters });
  },
  async filter(field, value) {
    state = { ...state, filters: [{ column: field, op: "eq", value }] };
    module.set_view(host, { sort: state.sort, filters: state.filters });
  },
  async clearFilter() {
    state = { ...state, filters: [] };
    module.set_view(host, { sort: state.sort, filters: state.filters });
  },
  scroller() {
    const all = [host, ...host.shadowRoot.querySelectorAll("*")];
    return all.find((el) => el.scrollHeight > el.clientHeight + 50 && /auto|scroll/.test(getComputedStyle(el).overflowY));
  },
  rowHeight() {
    return rowsOf()[0].querySelector("td").getBoundingClientRect().height;
  },
};
