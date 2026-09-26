//! An export read back by opengrid's own ingest is the data it came from
//! (plan point 83).
//!
//! 2 000 generated orders — every stored type: int64, text, decimal, float,
//! bool, date, timestamp, NULLs — go through the engine, out as CSV and JSON,
//! and the CSV back in through `load_csv`. With NULL spelled `\N` (the
//! ingest's convention) nothing may change. The formula protection is on: the
//! generated text never starts like a formula, so it must not touch it.
//!
//! The generated text is tame, so a second test carries the hostile values —
//! delimiters, quotes, line breaks, formula starts, far-out floats — with the
//! guard off, which is the export meant for a machine. Its limit: the ingest
//! does not tell a quoted `"\N"` from `\N`, so the text `\N` itself reads back
//! as NULL.

use opengrid_arrow_engine::datasource::LocalDataSource;
use opengrid_arrow_engine::ingest::{CsvOptions as IngestOptions, load_csv};
use opengrid_conformance::block_on;
use opengrid_datasource::wire::result_to_json;
use opengrid_datasource::{DataSource, QueryResult};
use opengrid_export::{CsvOptions, CsvWriter, JsonWriter};
use opengrid_query::{Limits, Query};

const STORED: [&str; 10] = [
    "id",
    "customer",
    "country",
    "amount",
    "qty",
    "ratio",
    "flag",
    "ordered_on",
    "created_at",
    "note",
];

/// Every stored column of a CSV, in id order, through the engine.
fn everything(csv: &str) -> QueryResult {
    let schema = xtask::orders_schema();
    let batches = load_csv(csv.as_bytes(), &schema, IngestOptions::default()).expect("ingest");
    let source = LocalDataSource::new(batches).expect("a source");
    let query: Query = serde_json::from_value(serde_json::json!({
        "source": "orders",
        "select": STORED,
        "sort": [{ "field": "id", "direction": "asc" }],
    }))
    .expect("a query");
    let query = query.validate(
        &schema,
        &Limits {
            max_limit: u64::MAX,
            ..Limits::default()
        },
    );
    block_on(source.execute(query.expect("a valid query"))).expect("the engine answers")
}

#[test]
fn a_csv_export_reads_back_unchanged() {
    let original = everything(&xtask::orders_csv(2_000, 11));
    assert_eq!(original.row_count(), 2_000);

    let options = CsvOptions {
        bom: false,
        null: "\\N".to_owned(),
        ..CsvOptions::default()
    };
    let mut writer = CsvWriter::new(options);
    let exported = writer.write(&original);

    let reread = everything(&exported);
    // Compared in the wire form: `NaN` is data here, and `NaN != NaN`.
    assert_eq!(result_to_json(&reread), result_to_json(&original));
}

#[test]
fn a_json_export_is_one_array_of_every_row() {
    let original = everything(&xtask::orders_csv(500, 3));
    let mut writer = JsonWriter::new();
    let json = writer.write(&original) + &writer.finish();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(rows.len(), 500);
    // Keys in the order of the columns, values in the wire notation.
    // Keys in the order of the columns (read from the text: a parsed map
    // sorts them), values in the wire notation.
    let first_row = &json[..json.find('}').expect("a row")];
    let positions: Vec<usize> = STORED
        .iter()
        .map(|name| {
            first_row
                .find(&format!("\"{name}\":"))
                .expect("every column")
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{first_row}"
    );
    assert!(rows[0]["amount"].is_string(), "a decimal is exact text");
}

#[test]
fn hostile_text_and_far_out_floats_read_back_unchanged_with_the_guard_off() {
    let hostile = [
        "=HYPERLINK(\"http://x\")",
        "+1",
        "-5",
        "@SUM(A1)",
        "a,b",
        "x;=cmd",
        "say \"hi\"",
        "one\ntwo",
        "cr\r\nlf",
        "tab\there",
        " spaced ",
        "",
        "ä – €",
    ];
    let floats = [f64::INFINITY, f64::NEG_INFINITY, 1e21, 5e-324, -0.0, 1e-7];
    // Generated rows, their `note` and `ratio` replaced by hostile values.
    let original_rows = everything(&xtask::orders_csv(hostile.len(), 5));
    let schema = original_rows.schema.clone();
    let note = schema.index_of("note").expect("a note column");
    let ratio = schema.index_of("ratio").expect("a ratio column");
    let mut columns = original_rows.columns.clone();
    for (row, text) in hostile.iter().enumerate() {
        columns[note][row] = opengrid_types::Value::Utf8((*text).to_owned());
        columns[ratio][row] = opengrid_types::Value::Float64(floats[row % floats.len()]);
    }
    let original = QueryResult::new(schema, columns, hostile.len() as u64);

    let options = CsvOptions {
        bom: false,
        protect_formulas: false,
        null: "\\N".to_owned(),
        ..CsvOptions::default()
    };
    let csv = CsvWriter::new(options).write(&original);
    let reread = everything(&csv);
    assert_eq!(result_to_json(&reread), result_to_json(&original), "{csv}");
}
