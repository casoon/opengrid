// The 100 000 rows the export spec runs over (issue #1), made rather than
// committed: four megabytes of CSV in the repository would be noise in every
// clone. Deterministic — the same bytes in the browser (export.html loads them
// into the tab's engine) and on disk (write-export-data.mjs, for the
// `opengrid-server` of the Playwright config), so the two sources hold the
// same table and an export over either can be compared with the other.
//
// The columns are chosen for the export, not for realism:
//
//   id      unique — "every row exactly once" is checked on it
//   region  five values and NULL: 20 000-row ties, and a NULL key
//   amount  an exact decimal, negative ones included
//   day     a date
//   note    text a CSV has to quote (comma, quote, line break), a formula a
//           spreadsheet would run, the empty string and NULL

export const EXPORT_ROWS = 100_000;

const REGIONS = ["north", "east", "south", "west", "centre"];
const NOTES = ["plain", "with, comma", 'with "quote"', "two\nlines", "=1+1", ""];

/** The CSV text of the first `rows` rows, with its header. */
export function exportCsv(rows = EXPORT_ROWS) {
  const lines = ["id,region,amount,day,note"];
  for (let id = 1; id <= rows; id += 1) {
    const region = id % 97 === 0 ? "\\N" : REGIONS[(id * 7) % REGIONS.length];
    const cents = ((id * 7919) % 2_000_001) - 1_000_000;
    const sign = cents < 0 ? "-" : "";
    const abs = Math.abs(cents);
    const amount = `${sign}${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
    const day = new Date(Date.UTC(2020, 0, 1) + (id % 1500) * 86_400_000).toISOString().slice(0, 10);
    const note = id % 89 === 0 ? "\\N" : NOTES[id % NOTES.length];
    lines.push([id, region, amount, day, quote(note)].join(","));
  }
  return `${lines.join("\n")}\n`;
}

/** RFC 4180 quoting where the text needs it; `\N` (NULL) stays bare. */
function quote(text) {
  return /[",\n]/.test(text) || text === "" ? `"${text.replaceAll('"', '""')}"` : text;
}
