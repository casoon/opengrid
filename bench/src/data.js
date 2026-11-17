// The same rows for every grid: deterministic, so a run can be repeated and a result checked.
// Six columns, one of each kind a grid has to handle — an integer key, text, a short code, an
// exact amount, a count and a date.

const COUNTRIES = ["DE", "FR", "PL", "NL", "ES", "IT", "AT", "CH"];

export const COLUMNS = [
  { field: "id", title: "ID", type: "int" },
  { field: "customer", title: "Customer", type: "text" },
  { field: "country", title: "Country", type: "text" },
  { field: "amount", title: "Amount", type: "number" },
  { field: "qty", title: "Qty", type: "int" },
  { field: "ordered_on", title: "Ordered", type: "text" },
];

export function makeRows(count) {
  const rows = new Array(count);
  for (let i = 1; i <= count; i++) {
    const day = String((i % 28) + 1).padStart(2, "0");
    rows[i - 1] = {
      id: i,
      customer: `Customer ${i % 500}`,
      country: COUNTRIES[i % 8],
      // Two decimals, and **unique** for every id up to a million: 7919 is coprime to the
      // prime 1,000,003, so no two rows tie and every grid sorts to the same first row.
      amount: ((i * 7919) % 1_000_003) / 100 + 0.5,
      qty: i % 40,
      ordered_on: `2026-03-${day}`,
    };
  }
  return rows;
}

/** What a correct answer looks like, so a grid that shows something else is marked wrong. */
export function expected(rows) {
  let top = rows[0];
  let topDe = null;
  let deCount = 0;
  for (const row of rows) {
    if (row.amount > top.amount) top = row;
    if (row.country === "DE") {
      deCount++;
      if (topDe === null || row.amount > topDe.amount) topDe = row;
    }
  }
  return { topId: top.id, topDeId: topDe.id, deCount };
}
