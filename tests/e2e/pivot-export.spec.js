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

/** The rendered table as rows of text, the header composed into one line. */
function shown(page) {
  return page.evaluate(() => {
    const root = document.querySelector("opengrid-pivot").shadowRoot;
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
    return [header, ...rows];
  });
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
  const csv = await getPivot(page);

  expect(csv.split("\r\n")[0]).toBe(
    "﻿country,2025 · total,2025 · n,2026 · total,2026 · n,(no value) · total,(no value) · n",
  );
  const table = await shown(page);
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
  const csv = await getPivot(page, { bom: false, delimiter: ",", null: "\\N" });

  const table = await shown(page);
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
  const csv = await getPivot(page, { bom: false });

  const table = await shown(page);
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
    return {
      grid: module.get_pivot(document.body),
      misspelt: error({ delimeter: ";" }),
      mistyped: error({ bom: "false" }),
      tooLong: error({ delimiter: ";;" }),
    };
  });
  expect(answers.grid).toBeNull();
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
