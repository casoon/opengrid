// Rows as objects, `{ column: value }` in the wire notation — the shape
// `@casoon/chartlet-mcp` reads (#140, point 11).

/** The rows of a result in the wire JSON, as objects. */
export function rowsOf(resultJson) {
  const result = typeof resultJson === "string" ? JSON.parse(resultJson) : resultJson;
  const rows = [];
  for (let index = 0; index < result.row_count; index += 1) {
    const row = {};
    for (const column of result.columns) {
      row[column.name] = column.values[index];
    }
    rows.push(row);
  }
  return rows;
}

/** Ascending positions as `[offset, limit]` runs — one query per run. */
export function runsOf(positions) {
  const runs = [];
  for (const position of positions) {
    const last = runs.at(-1);
    if (last && last[0] + last[1] === position) {
      last[1] += 1;
    } else {
      runs.push([position, 1]);
    }
  }
  return runs;
}
