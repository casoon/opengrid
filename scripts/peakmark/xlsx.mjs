// Scenario for `peakmark check`: write a workbook of 50 000 rows in five pieces, as the export does.
// The memory it needs is the number the xlsx module's budget guards.
const PIECES = 5;
const ROWS = 10_000;

function piece(start) {
  const id = [];
  const customer = [];
  const country = [];
  const amount = [];
  const qty = [];
  for (let i = start; i < start + ROWS; i++) {
    id.push(i);
    customer.push(`Customer ${i % 500}`);
    country.push(["DE", "FR", "PL", "NL", "ES"][i % 5]);
    amount.push((i % 997) + 0.25);
    qty.push(i % 40);
  }
  const column = (name, type, values) => ({ name, type, values });
  return JSON.stringify({
    total_count: PIECES * ROWS,
    row_count: ROWS,
    columns: [
      column("id", "int64", id),
      column("customer", "utf8", customer),
      column("country", "utf8", country),
      column("amount", "float64", amount),
      column("qty", "int64", qty),
    ],
  });
}

export default async function (xlsx) {
  const book = new xlsx.XlsxBook("orders");
  for (let n = 0; n < PIECES; n++) {
    book.add(piece(n * ROWS));
  }
  const bytes = book.finish();
  if (bytes.length === 0) {
    throw new Error("the workbook is empty");
  }
  book.free();
}
