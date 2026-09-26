// The props the grid is rendered with on the server — and, in hydrate.html,
// hydrated with in the browser (plan point 81). One place, so the two cannot
// drift apart; the provider comes on top on the client only, as a page does.
export const SSR_PROPS = {
  label: "Orders",
  datasource: "orders",
  columns: "id,customer",
  windowSize: 40,
  selection: true,
  toolbar: false,
  class: "orders",
  texts: { lang: "de" },
};
