// Writes the export spec's 100 000 rows (export-data.js) to disk for the
// `opengrid-server` the Playwright config starts; opengrid-e2e.toml names the
// file. Under `target/`, which is ignored: the rows are made, not kept.
//
//   node tests/e2e/fixtures/write-export-data.mjs

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { exportCsv } from "./export-data.js";

const out = resolve(dirname(fileURLToPath(import.meta.url)), "../../../target/e2e/export.csv");
mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, exportCsv());
