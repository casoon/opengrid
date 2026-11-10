import { test, expect } from "@playwright/test";
import { inflateRawSync } from "node:zlib";

// A pivot exported as it is shown (issue #3): `get_pivot` against a real
// `opengrid-server` over the conformance data set.
//
// The claim is that the CSV **is** the element's table, row for row. So the
// expected rows are not written down here: they are read off the rendered
// table — its header composed the way the export composes it (value · measure),
// a spanning row header followed by the empty fields it spans — and the parsed
// CSV has to equal them. The first line is spelled out once, because "equal to
// the table" alone would not say that the header is one line.

async function open(page) {
  await page.goto("/tests/e2e/fixtures/pivot.html");
  await page.waitForFunction(() => window.__ready === true);
  await ready(page);
}

/** Waits until the pivot shows an answer. */
async function ready(page) {
  await page.waitForFunction(() => {
    const status = document
      .querySelector("opengrid-pivot")
      ?.shadowRoot?.querySelector('[part="status"]');
    return status?.getAttribute("data-state") === "ready";
  });
}

/**
 * `get_pivot` and the rendered table, read in **one** evaluation — so no answer
 * can arrive between the two and make them disagree for the wrong reason. The
 * table is rows of text, its header composed into one line the way the export
 * composes it.
 */
function exported(page, options) {
  return page.evaluate((options) => {
    const pivot = document.querySelector("opengrid-pivot");
    const csv = window.__opengridModule.get_pivot(pivot, options);
    const root = pivot.shadowRoot;
    const head = [...root.querySelectorAll("thead tr")];
    const said = (node) => {
      // What a reader hears: the direction mark is `aria-hidden`.
      const copy = node.cloneNode(true);
      copy.querySelectorAll('[aria-hidden="true"]').forEach((mark) => mark.remove());
      return copy.textContent;
    };
    let header;
    if (head.length === 2) {
      // Row-dimension names span both rows; each value spans its measures.
      const dimensions = [...head[0].querySelectorAll("th[rowspan]")].map(said);
      const groups = [...head[0].querySelectorAll('th[scope="colgroup"]')].flatMap((th) =>
        Array(Number(th.getAttribute("colspan"))).fill(th.textContent),
      );
      const measures = [...head[1].querySelectorAll("th")].map(said);
      header = [...dimensions, ...measures.map((measure, i) => `${groups[i]} · ${measure}`)];
    } else {
      header = [...head[0].querySelectorAll("th")].map(said);
    }
    const rows = [...root.querySelectorAll("tbody tr")].map((tr) =>
      [...tr.children].flatMap((cell) => [
        said(cell),
        ...Array(Number(cell.getAttribute("colspan") ?? 1) - 1).fill(""),
      ]),
    );
    return { csv, table: [header, ...rows] };
  }, options);
}

const getPivot = (page, options) =>
  page.evaluate(
    (options) =>
      window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot"), options),
    options,
  );

/** RFC 4180, as the export writes it: CRLF records, `"` quoting. */
function parse(csv) {
  const records = [];
  let record = [];
  let field = "";
  let quoted = false;
  for (let i = 0; i < csv.length; i++) {
    const c = csv[i];
    if (quoted) {
      if (c === '"' && csv[i + 1] === '"') {
        field += '"';
        i++;
      } else if (c === '"') {
        quoted = false;
      } else {
        field += c;
      }
    } else if (c === '"') {
      quoted = true;
    } else if (c === ",") {
      record.push(field);
      field = "";
    } else if (c === "\r" && csv[i + 1] === "\n") {
      record.push(field);
      records.push(record);
      record = [];
      field = "";
      i++;
    } else {
      field += c;
    }
  }
  expect(field, "the last line ends in CRLF").toBe("");
  expect(record).toEqual([]);
  return records;
}

test("the CSV of the pivot is its table, row for row, with one header line", async ({ page }) => {
  await open(page);
  const { csv, table } = await exported(page);

  expect(csv.split("\r\n")[0]).toBe(
    "\uFEFFcountry,2025 · total,2025 · n,2026 · total,2026 · n,(no value) · total,(no value) · n",
  );
  expect(parse(csv.slice(1))).toEqual(table);
  // The NULL country is a row of its own and named; the last row is the total.
  expect(table.map((row) => row[0])).toContain("(no value)");
  expect(table.map((row) => row[0])).toContain("(empty)");
  expect(table.at(-1)[0]).toBe("Total");
});

test("subtotals are rows with their label, spanning the dimensions", async ({ page }) => {
  await open(page);
  // The pivot of conformance case P6, over the whole data set.
  await page.evaluate(() => {
    const pivot = document.querySelector("opengrid-pivot");
    pivot.setAttribute("values", '[{"fn":"count","as":"n"}]');
    pivot.setAttribute("columns", "");
    pivot.setAttribute("rows", "country,customer");
  });
  // Each attribute asks again; wait for the answer to the last one.
  await page.waitForFunction(() => {
    const root = document.querySelector("opengrid-pivot").shadowRoot;
    // The name, without the direction mark beside it.
    const header = [...root.querySelectorAll("thead th")].map(
      (th) => th.querySelector("button > span:first-child")?.textContent ?? th.textContent,
    );
    return (
      header.join() === "country,customer,n" &&
      root.querySelectorAll("tbody tr[data-total]").length > 1
    );
  });
  const { csv, table } = await exported(page, { bom: false, delimiter: ",", null: "\\N" });
  expect(parse(csv)).toEqual(table);
  expect(table[0]).toEqual(["country", "customer", "n"]);
  expect(table).toContainEqual(["Total DE", "", "16"]);
  expect(table).toContainEqual(["Total (no value)", "", "7"]);
  expect(table.at(-1)).toEqual(["Total", "", "50"]);
});

test("the page's texts are the export's labels", async ({ page }) => {
  await open(page);
  await page.evaluate(() => {
    const pivot = document.querySelector("opengrid-pivot");
    window.__opengridModule.set_texts(pivot, {
      noValue: "(ohne Wert)",
      emptyValue: "(leer)",
      total: "Summe",
      subtotal: "Summe {value}",
    });
    pivot.setAttribute("rows", "country,customer");
  });
  await page.waitForFunction(() =>
    [...document.querySelector("opengrid-pivot").shadowRoot.querySelectorAll("tbody th")].some(
      (th) => th.textContent.endsWith("Summe (ohne Wert)"),
    ),
  );
  const { csv, table } = await exported(page, { bom: false });
  expect(parse(csv)).toEqual(table);
  expect(csv).toContain("(ohne Wert) · total");
  expect(csv).toContain("\r\n(leer),");
  expect(csv).not.toContain("(no value)");
});

test("nothing shown is null, and a wrong option is an error", async ({ page }) => {
  await open(page);
  const answers = await page.evaluate(() => {
    const module = window.__opengridModule;
    const pivot = document.querySelector("opengrid-pivot");
    const error = (options) => {
      try {
        module.get_pivot(pivot, options);
        return "no error";
      } catch (e) {
        return String(e.message);
      }
    };
    // A real grid and a real table, in the document: neither is a pivot.
    const grid = document.createElement("opengrid-grid");
    const table = document.createElement("opengrid-table");
    document.body.append(grid, table);
    return {
      grid: module.get_pivot(grid),
      table: module.get_pivot(table),
      misspelt: error({ delimeter: ";" }),
      mistyped: error({ bom: "false" }),
      tooLong: error({ delimiter: ";;" }),
    };
  });
  expect(answers.grid).toBeNull();
  expect(answers.table).toBeNull();
  expect(answers.misspelt).toContain("delimeter");
  expect(answers.mistyped).toContain("bom");
  expect(answers.tooLong).toContain("delimiter");

  // A broken measure list: the table is empty, and so is the export.
  await page.evaluate(() =>
    document.querySelector("opengrid-pivot").setAttribute("values", "sum(qty)"),
  );
  await page.waitForFunction(
    () =>
      document
        .querySelector("opengrid-pivot")
        .shadowRoot.querySelector('[part="status"]')
        .getAttribute("data-state") === "error",
  );
  expect(await getPivot(page)).toBeNull();
});

/** A pivot answer a page's own provider could send: one row and the total. */
function answer(columns) {
  return {
    row_dimensions: ["country"],
    columns,
    levels: [1, 0],
    result: {
      total_count: 2,
      row_count: 2,
      columns: [
        { name: "country", type: "utf8", nullable: true, values: ["DE", null] },
        { name: "total_0", type: "float64", nullable: true, values: [2, 2] },
      ],
    },
  };
}

/** Gives the pivot a provider that answers `body`, and waits for it to show. */
async function provide(page, body) {
  // The provider alone is swapped: an attribute change would ask the server
  // again, and its answer could land after this one.
  await page.evaluate((body) => {
    const pivot = document.querySelector("opengrid-pivot");
    window.__opengridModule.set_provider(pivot, { execute: () => JSON.stringify(body) });
  }, body);
  await page.waitForFunction(
    () =>
      document.querySelector("opengrid-pivot").shadowRoot.querySelector("tbody td")
        ?.textContent === "2",
  );
  await ready(page);
}

test("a custom provider's values are written in the canonical notation", async ({ page }) => {
  await open(page);
  await provide(page, answer([{ path: [2025], measure: "total" }]));
  const { csv, table } = await exported(page, { bom: false });

  // The table shows the text as it came; the export writes the float's notation.
  expect(table).toEqual([["country", "2025 · total"], ["DE", "2"], ["Total", "2"]]);
  expect(csv).toBe("country,2025 · total\r\nDE,2.0\r\nTotal,2.0\r\n");
});

test("an answer of the wrong shape is an error with a sentence, not a trap", async ({ page }) => {
  await open(page);
  // Two generated columns announced, one sent: the element draws what it can,
  // the export refuses to guess which column is which.
  await provide(
    page,
    answer([
      { path: [2025], measure: "total" },
      { path: [2026], measure: "total" },
    ]),
  );
  const message = await page.evaluate(() => {
    try {
      window.__opengridModule.get_pivot(document.querySelector("opengrid-pivot"));
      return "no error";
    } catch (e) {
      return String(e.message);
    }
  });
  expect(message).toContain("the shown pivot");
  expect(message).toContain("need 3");
});

// The pivot as a workbook (issue #331): `exportPivot` against the real server.
//
// The claim is that the sheet **is** the CSV's table — one header row, the
// same rows and columns in the same order — with the cells typed. So the file
// is opened here, not only received: the archive is read with `node:zlib`, the
// sheet's cells are taken from its XML, and each is held against the field at
// the same place in `get_pivot`'s CSV.

/** The files of a ZIP archive by name, read from its central directory. */
function unzip(file) {
  const end = file.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  expect(end, "a ZIP archive ends in its directory").toBeGreaterThan(-1);
  const files = new Map();
  let at = file.readUInt32LE(end + 16);
  for (let entry = file.readUInt16LE(end + 10); entry > 0; entry--) {
    const method = file.readUInt16LE(at + 10);
    const size = file.readUInt32LE(at + 20);
    const nameLength = file.readUInt16LE(at + 28);
    const local = file.readUInt32LE(at + 42);
    const name = file.toString("utf8", at + 46, at + 46 + nameLength);
    const start = local + 30 + file.readUInt16LE(local + 26) + file.readUInt16LE(local + 28);
    const data = file.subarray(start, start + size);
    files.set(name, method === 0 ? data : inflateRawSync(data));
    at += 46 + nameLength + file.readUInt16LE(at + 30) + file.readUInt16LE(at + 32);
  }
  return files;
}

const unescape = (text) =>
  text
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&amp;", "&");

/**
 * The first sheet of a workbook: its XML, and its cells by reference (`A1`)
 * as `{ type, styled, value }` — `type` is the cell's `t` attribute, absent
 * for a number or a date.
 */
function sheetOf(base64) {
  const files = unzip(Buffer.from(base64, "base64"));
  const xml = files.get("xl/worksheets/sheet1.xml")?.toString("utf8");
  expect(xml, "the workbook has a first sheet").toBeTruthy();
  const cells = new Map();
  for (const [, attributes, body] of xml.matchAll(/<c ([^>]*?)(?:\/>|>(.*?)<\/c>)/gs)) {
    const attribute = (name) => attributes.match(new RegExp(`(?:^| )${name}="([^"]*)"`))?.[1];
    const type = attribute("t");
    // This writer puts a text into its cell; a shared string would be an
    // index, and reading it as the text would compare nothing.
    expect(type === undefined || type === "inlineStr" || type === "b", `cell type ${type}`).toBe(true);
    const content = body?.match(type === "inlineStr" ? /<t[^>]*>(.*?)<\/t>/s : /<v>(.*?)<\/v>/s)?.[1];
    cells.set(attribute("r"), {
      type,
      styled: attribute("s") !== undefined,
      value: unescape(content ?? ""),
    });
  }
  return { xml, cells };
}

/** `A`, `B`, … `Z`, `AA`: the letters of the column at `index`. */
function letters(index) {
  let name = "";
  for (let n = index + 1; n > 0; n = Math.floor((n - 1) / 26)) {
    name = String.fromCharCode(65 + ((n - 1) % 26)) + name;
  }
  return name;
}

/** The day of `YYYY-MM-DD` as Excel counts them. */
const excelDay = (date) => Date.parse(`${date}T00:00:00Z`) / 86_400_000 + 25_569;

/**
 * The shown pivot as CSV and as the server's workbook, with the requests the
 * export made.
 */
async function workbook(page) {
  const requests = [];
  const seen = (request) => {
    const url = new URL(request.url());
    if (url.port === "8082" && request.method() === "POST") {
      requests.push(url.pathname + url.search);
    }
  };
  page.on("request", seen);
  const answer = await page.evaluate(async () => {
    const { createPivotProvider, exportPivot } = await import("/packages/opengrid/loader.js");
    const pivot = document.querySelector("opengrid-pivot");
    const provider = createPivotProvider({
      url: "http://127.0.0.1:8082",
      source: "orders",
      token: "e2e-token",
    });
    const csv = window.__opengridModule.get_pivot(pivot, { bom: false });
    // Nothing but the format: the words are read from the element.
    const blob = await exportPivot(provider, pivot, { format: "xlsx" });
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return { csv, type: blob.type, base64: btoa(binary) };
  });
  page.off("request", seen);
  expect(answer.type).toBe("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet");
  // One request, to the pivot's endpoint; `sent` is its parameters.
  expect(requests).toHaveLength(1);
  const { pathname, searchParams } = new URL(requests[0], "http://127.0.0.1:8082");
  expect(pathname).toBe("/pivot/orders");
  const sent = Object.fromEntries(searchParams);
  return { table: parse(answer.csv), ...sheetOf(answer.base64), sent };
}

/**
 * Holds every cell of the sheet against the CSV's field at its place.
 * `dimensions` row-dimension columns come first; `dates` names those of them
 * that hold dates. Answers how many cells of each kind it saw.
 */
function expectSheetIsTable(cells, table, { dimensions, dates = [] }) {
  const seen = { text: 0, number: 0, date: 0, empty: 0 };
  for (const [row, record] of table.entries()) {
    for (const [column, field] of record.entries()) {
      const reference = `${letters(column)}${row + 1}`;
      const cell = cells.get(reference);
      if (field === "") {
        // Spanned by a total, or a NULL measure: no cell at all.
        expect(cell, `${reference} is empty in the CSV`).toBeUndefined();
        seen.empty++;
      } else if (row > 0 && column >= dimensions) {
        // A measure: a number, the CSV's number.
        expect(cell, reference).toMatchObject({ type: undefined });
        expect(cell.value, reference).not.toBe("");
        expect(Number(cell.value), reference).toBe(Number(field));
        seen.number++;
      } else if (row > 0 && dates.includes(column) && /^\d{4}-\d{2}-\d{2}$/.test(field)) {
        // A date in a dimension column: Excel's day number, with a format.
        expect(cell, reference).toEqual({ type: undefined, styled: true, value: String(excelDay(field)) });
        seen.date++;
      } else {
        // The header, a text dimension, a label.
        expect(cell, reference).toMatchObject({ type: "inlineStr", value: field });
        seen.text++;
      }
    }
  }
  const filled = table.flat().filter((field) => field !== "").length;
  expect(cells.size, "no cell beside the table").toBe(filled);
  return seen;
}

test("the workbook of the pivot is its CSV's table, with typed cells", async ({ page }) => {
  await open(page);
  // A text and a date as row dimensions, a decimal sum and a count as
  // measures, a year across: every kind of cell a pivot has.
  await page.evaluate(() => {
    const pivot = document.querySelector("opengrid-pivot");
    window.__opengridModule.set_view(pivot, {
      rows: ["country", "ordered_on"],
      columns: ["ordered_year"],
      values: [
        { field: "amount", fn: "sum", as: "total" },
        { fn: "count", as: "n" },
      ],
    });
  });
  await page.waitForFunction(() =>
    window.__opengridModule
      .get_pivot(document.querySelector("opengrid-pivot"), { bom: false })
      ?.startsWith("country,ordered_on,"),
  );

  const { table, cells, xml, sent } = await workbook(page);
  // The element's words went along: here its English defaults.
  expect(sent).toEqual({
    format: "xlsx",
    noValue: "(no value)",
    emptyValue: "(empty)",
    total: "Total",
    subtotal: "Total {value}",
  });

  // One header row, the CSV's.
  expect(table[0].slice(0, 4)).toEqual(["country", "ordered_on", "2025 · total", "2025 · n"]);
  const seen = expectSheetIsTable(cells, table, { dimensions: 2, dates: [1] });
  // The comparison met every kind: dates, numbers, labels, spanned cells.
  expect(seen.date).toBeGreaterThan(5);
  expect(seen.number).toBeGreaterThan(20);
  expect(seen.empty).toBeGreaterThan(5);

  // Spelled out once: a subtotal row, and the grand total last.
  const row = table.findIndex((record) => record[0] === "Total DE") + 1;
  expect(row).toBeGreaterThan(1);
  expect(cells.get(`A${row}`)).toMatchObject({ type: "inlineStr", value: "Total DE" });
  expect(cells.has(`B${row}`)).toBe(false);
  expect(cells.get(`A${table.length}`)).toMatchObject({ type: "inlineStr", value: "Total" });
  const labels = table.map((record) => record[0]);
  expect(labels).toContain("(no value)");
  expect(labels).toContain("(empty)");
  // A decimal sum is a number with its scale as the format.
  expect(table[0][2]).toBe("2025 · total");
  expect(cells.get(`C${table.length}`)).toMatchObject({ type: undefined, styled: true });

  // A flat sheet: nothing merged, no outline, no formula.
  expect(xml).not.toContain("<mergeCell");
  expect(xml).not.toContain("outlineLevel");
  expect(xml).not.toContain("<f>");
});

test("the workbook reads in the element's texts, the German ones included", async ({ page }) => {
  await open(page);
  // The translation the package ships, set the way a page sets it — and
  // nothing about texts is passed to the export.
  const german = await page.evaluate(async () => {
    const { default: de } = await import("/packages/opengrid/texts/de.js");
    const pivot = document.querySelector("opengrid-pivot");
    window.__opengridModule.set_texts(pivot, de);
    pivot.setAttribute("rows", "country,customer");
    return { noValue: de.noValue, emptyValue: de.emptyValue, total: de.total, subtotal: de.subtotal };
  });
  expect(german).toEqual({
    noValue: "(kein Wert)",
    emptyValue: "(leer)",
    total: "Gesamt",
    subtotal: "Gesamt {value}",
  });
  await page.waitForFunction(() =>
    [...document.querySelector("opengrid-pivot").shadowRoot.querySelectorAll("tbody th")].some(
      (th) => th.textContent.endsWith("Gesamt (kein Wert)"),
    ),
  );

  const { table, cells, sent } = await workbook(page);
  expect(sent).toEqual({ format: "xlsx", ...german });
  // The sheet is the CSV's table, and its first column is the row headers
  // the table on the screen shows: the labels in the file are the ones shown.
  expectSheetIsTable(cells, table, { dimensions: 2 });
  const shown = (await exported(page, { bom: false })).table;
  const labels = table.slice(1).map((record, index) => cells.get(`A${index + 2}`)?.value);
  expect(labels).toEqual(shown.slice(1).map((row) => row[0]));
  expect(labels).toEqual(
    expect.arrayContaining(["(kein Wert)", "(leer)", "Gesamt DE", "Gesamt (kein Wert)", "Gesamt"]),
  );
  const column = shown[0].indexOf("(kein Wert) · total");
  expect(column).toBeGreaterThan(1);
  expect(cells.get(`${letters(column)}1`)).toMatchObject({ value: "(kein Wert) · total" });
  expect(labels.join()).not.toMatch(/Total|no value|empty/);

  // A text set afterwards is the next file's, without a new answer.
  await page.evaluate(() =>
    window.__opengridModule.set_texts(document.querySelector("opengrid-pivot"), { total: "Alles" }),
  );
  const next = await workbook(page);
  expect(next.sent).toEqual({ format: "xlsx", ...german, total: "Alles" });
  expect(next.cells.get(`A${next.table.length}`)).toMatchObject({ value: "Alles" });
});

test("a workbook needs a provider that exports on the server", async ({ page }) => {
  await open(page);
  const posts = [];
  page.on("request", (request) => request.method() === "POST" && posts.push(request.url()));
  const refused = await page.evaluate(async () => {
    const loader = await import("/packages/opengrid/loader.js");
    const pivot = document.querySelector("opengrid-pivot");
    const server = loader.createPivotProvider({
      url: "http://127.0.0.1:8082",
      source: "orders",
      token: "e2e-token",
    });
    const error = async (provider, host, options) => {
      try {
        await loader.exportPivot(provider, host, options);
        return "no error";
      } catch (e) {
        return `${e.name}: ${e.message}`;
      }
    };
    const xlsx = { format: "xlsx" };
    return {
      local: await error(loader.createLocalProvider({}), pivot, xlsx),
      worker: await error(loader.createWorkerProvider(), pivot, xlsx),
      csv: await error(server, pivot, { format: "csv" }),
      none: await error(server, pivot, {}),
      option: await error(server, pivot, { ...xlsx, delimiter: ";" }),
      texts: await error(server, pivot, { ...xlsx, texts: { total: "Sum" } }),
      grid: await error(server, document.createElement("opengrid-grid"), xlsx),
    };
  });
  const needsServer = "TypeError: exportPivot: an XLSX export needs a provider that exports on the server";
  expect(refused.local).toContain(needsServer);
  expect(refused.worker).toContain(needsServer);
  expect(refused.csv).toContain('TypeError: exportPivot: format is "xlsx", not "csv"');
  expect(refused.none).toContain('format is "xlsx", not undefined');
  expect(refused.option).toContain('unknown option "delimiter"');
  // The words are the element's: handing some over is refused, not ignored.
  expect(refused.texts).toContain('unknown option "texts"');
  expect(refused.grid).toContain("no pivot to export");
  // Each was refused before anything was sent.
  expect(posts).toEqual([]);
});

test("the server's refusal of a pivot's workbook reaches the page", async ({ page }) => {
  await open(page);
  const refused = await page.evaluate(async () => {
    const loader = await import("/packages/opengrid/loader.js");
    const provider = loader.createRestProvider({
      url: "http://127.0.0.1:8082",
      source: "orders",
      token: "e2e-token",
    });
    const query = { source: "orders", rows: ["nowhere"], columns: [], values: [{ fn: "count", as: "n" }] };
    try {
      await provider.exportPivot(query, { format: "xlsx" });
      return "no error";
    } catch (e) {
      return { status: e.status, code: e.code, message: e.message };
    }
  });
  expect(refused).toMatchObject({ status: 422, code: "validation" });
  expect(refused.message).toContain("nowhere");

  // A server from before this export answers the pivot itself: an error, not
  // a JSON file under a workbook's name.
  const old = await page.evaluate(async () => {
    const loader = await import("/packages/opengrid/loader.js");
    const provider = loader.createRestProvider({ url: "http://127.0.0.1:8082", source: "orders" });
    const fetchBefore = window.fetch;
    window.fetch = async () =>
      new Response('{"row_dimensions":[]}', { headers: { "Content-Type": "application/json" } });
    try {
      await provider.exportPivot({ source: "orders" }, { format: "xlsx" });
      return "no error";
    } catch (e) {
      return e.message;
    } finally {
      window.fetch = fetchBefore;
    }
  });
  expect(old).toContain("does not write a pivot as XLSX");
});
