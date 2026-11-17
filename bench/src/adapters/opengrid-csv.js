// opengrid, as a page installs it from npm: the element, a worker provider, the rows handed over
// as the same objects every grid gets (`opengrid-csv.js` hands over CSV bytes, the engine's own
// input). `amount` is a float64 here, as it is a JS number in every
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
let state = { sort: [{ field: "id", direction: "asc" }], filters: [], group: [], aggregates: {} };
const digits = (text) => Number(String(text ?? "").replace(/[^0-9]/g, ""));
const rowsOf = () => [...host.shadowRoot.querySelectorAll("tbody tr")].filter((tr) => tr.querySelector("td[data-row]"));

export default {
  name: "opengrid-csv",
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
    // The same rows as CSV bytes — the form the engine reads without a copy through JSON. Building
    // the text is in the first-rows figure: a page that has objects pays it; a page that has CSV
    // (a file, a response) does not.
    const lines = ["id,customer,country,amount,qty,ordered_on"];
    for (const r of rows) lines.push(`${r.id},${r.customer},${r.country},${r.amount},${r.qty},${r.ordered_on}`);
    await provider.load("orders", new TextEncoder().encode(lines.join("\n")), SCHEMA);
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
    module.set_view(host, state);
  },
  async filter(field, value) {
    state = { ...state, filters: [{ column: field, op: "eq", value }] };
    module.set_view(host, state);
  },
  async clearFilter() {
    state = { ...state, filters: [] };
    module.set_view(host, state);
  },
  async group(field, aggregates) {
    state = { ...state, group: [field], aggregates };
    module.set_view(host, state);
  },
  async ungroup() {
    state = { ...state, group: [], aggregates: {} };
    module.set_view(host, state);
  },
  // The group headers on screen, read from the DOM: { key, count, sum } — the label says
  // "country: DE (2500 rows)", the amount's cell holds the sum.
  groups() {
    if (!host?.shadowRoot) return [];
    return [...host.shadowRoot.querySelectorAll("tbody tr[data-kind=group]")].flatMap((tr) => {
      const cells = [...tr.querySelectorAll("td")].map((td) => td.textContent.trim());
      const match = /^country: (\S+) \((\d+) rows\)/.exec(cells[0]);
      return match ? [{ key: match[1], count: Number(match[2]), sum: Number(cells[3]) }] : [];
    });
  },
  scroller() {
    const all = [host, ...host.shadowRoot.querySelectorAll("*")];
    return all.find((el) => el.scrollHeight > el.clientHeight + 50 && /auto|scroll/.test(getComputedStyle(el).overflowY));
  },
  rowHeight() {
    return rowsOf()[0].querySelector("td").getBoundingClientRect().height;
  },
};
