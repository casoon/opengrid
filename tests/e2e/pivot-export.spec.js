import { test, expect } from "@playwright/test";

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
    let header;
    if (head.length === 2) {
      // Row-dimension names span both rows; each value spans its measures.
      const dimensions = [...head[0].querySelectorAll("th[rowspan]")].map((th) => th.textContent);
      const groups = [...head[0].querySelectorAll('th[scope="colgroup"]')].flatMap((th) =>
        Array(Number(th.getAttribute("colspan"))).fill(th.textContent),
      );
      const measures = [...head[1].querySelectorAll("th")].map((th) => th.textContent);
      header = [...dimensions, ...measures.map((measure, i) => `${groups[i]} · ${measure}`)];
    } else {
      header = [...head[0].querySelectorAll("th")].map((th) => th.textContent);
    }
    const rows = [...root.querySelectorAll("tbody tr")].map((tr) =>
      [...tr.children].flatMap((cell) => [
        cell.textContent,
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
    const header = [...root.querySelectorAll("thead th")].map((th) => th.textContent);
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
      (th) => th.textContent === "Summe (ohne Wert)",
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
