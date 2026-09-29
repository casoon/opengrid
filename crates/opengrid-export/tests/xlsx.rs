//! The XLSX export (issue #72), opened again: every type's cell, the text
//! fallbacks where Excel cannot hold a value exactly, and Excel's limits.
#![cfg(feature = "xlsx")]

use std::io::Read;

use opengrid_datasource::QueryResult;
use opengrid_export::XlsxWriter;
use opengrid_types::{DataType, Date, Decimal, Field, FieldName, Schema, Timestamp, Value};

fn field(name: &str, data_type: DataType) -> Field {
    Field::new(FieldName::new(name).unwrap(), data_type)
}

/// One cell of the sheet: its type attribute and its content.
#[derive(Debug, PartialEq)]
struct Cell {
    kind: Option<String>,
    styled: bool,
    text: String,
}

/// The cells of sheet 1 by reference (`A1`, `B2`, …), read from the XML.
fn cells(file: &[u8]) -> std::collections::BTreeMap<String, Cell> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(file)).expect("a zip archive");
    let mut xml = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .expect("sheet 1")
        .read_to_string(&mut xml)
        .unwrap();
    let mut out = std::collections::BTreeMap::new();
    for chunk in xml.split("<c r=\"").skip(1) {
        let (reference, rest) = chunk.split_once('"').unwrap();
        let head = &rest[..rest.find('>').unwrap()];
        let attribute = |name: &str| {
            head.split(&format!("{name}=\""))
                .nth(1)
                .map(|value| value.split('"').next().unwrap().to_owned())
        };
        let body = &rest[rest.find('>').unwrap() + 1..rest.find("</c>").unwrap_or(rest.len())];
        let text = body
            .split("<t>")
            .nth(1)
            .or_else(|| body.split("<t xml:space=\"preserve\">").nth(1))
            .map(|text| text.split("</t>").next().unwrap())
            .or_else(|| {
                body.split("<v>")
                    .nth(1)
                    .map(|value| value.split("</v>").next().unwrap())
            })
            .unwrap_or("")
            .to_owned();
        out.insert(
            reference.to_owned(),
            Cell {
                kind: attribute("t"),
                styled: attribute("s").is_some(),
                text,
            },
        );
    }
    out
}

fn workbook(result: &QueryResult) -> Vec<u8> {
    let mut writer = XlsxWriter::new("orders").unwrap();
    writer.write(result).unwrap();
    writer.finish().unwrap()
}

#[test]
fn every_type_is_the_excel_type_that_holds_it_exactly() {
    let decimal = DataType::decimal(20, 2).unwrap();
    let schema = Schema::new(vec![
        field("flag", DataType::Bool),
        field("count", DataType::Int64),
        field("ratio", DataType::Float64),
        field("amount", decimal),
        field("note", DataType::Utf8),
        field("day", DataType::Date),
        field("at", DataType::Timestamp),
    ]);
    let result = QueryResult::new(
        schema,
        vec![
            vec![Value::Bool(true), Value::Null],
            vec![Value::Int64(-42), Value::Int64(i64::MAX)],
            vec![Value::Float64(1.5), Value::Float64(f64::NAN)],
            vec![
                Value::Decimal(Decimal::new(1_234_567, 2)),
                Value::Decimal(Decimal::new(123_456_789_012_345_678, 2)),
            ],
            vec![Value::Utf8("=1+1".into()), Value::Utf8(String::new())],
            vec![
                Value::Date(Date::from_ymd(2026, 9, 28).unwrap()),
                Value::Date(Date::from_ymd(1800, 1, 1).unwrap()),
            ],
            vec![
                Value::Timestamp(Timestamp::parse("2026-09-28T12:34:56.789Z").unwrap()),
                Value::Timestamp(Timestamp::parse("2026-09-28T12:34:56.789123Z").unwrap()),
            ],
        ],
        2,
    );
    let cells = cells(&workbook(&result));

    // The header: the field names.
    assert_eq!(cells["A1"].text, "flag");
    assert_eq!(cells["G1"].text, "at");

    // Row 2: every value in its Excel type.
    assert_eq!(cells["A2"].kind.as_deref(), Some("b"));
    assert_eq!(cells["A2"].text, "1");
    assert_eq!(
        (cells["B2"].kind.as_deref(), cells["B2"].text.as_str()),
        (None, "-42")
    );
    assert_eq!(
        (cells["C2"].kind.as_deref(), cells["C2"].text.as_str()),
        (None, "1.5")
    );
    assert_eq!(
        (cells["D2"].kind.as_deref(), cells["D2"].text.as_str()),
        (None, "12345.67")
    );
    assert!(
        cells["D2"].styled,
        "a decimal carries its scale as a format"
    );
    // A string is a string, never a formula.
    assert_eq!(cells["E2"].kind.as_deref(), Some("inlineStr"));
    assert_eq!(cells["E2"].text, "=1+1");
    // 2026-09-28 is Excel serial 46293; 12:34:56.789 is its fraction.
    assert_eq!(cells["F2"].text, "46293");
    assert!(
        cells["G2"].text.starts_with("46293.524268"),
        "{}",
        cells["G2"].text
    );

    // Row 3: NULL is no cell; what Excel cannot hold exactly is text.
    assert!(!cells.contains_key("A3"), "NULL is an empty cell");
    assert_eq!(cells["B3"].kind.as_deref(), Some("inlineStr"));
    assert_eq!(cells["B3"].text, i64::MAX.to_string());
    assert_eq!(cells["C3"].text, "NaN");
    assert_eq!(cells["D3"].text, "1234567890123456.78", "past 15 digits");
    assert_eq!(cells["D3"].kind.as_deref(), Some("inlineStr"));
    assert_eq!(cells["F3"].text, "1800-01-01", "before Excel's first year");
    assert_eq!(
        cells["G3"].text, "2026-09-28T12:34:56.789123Z",
        "microseconds"
    );
    // Excel has no empty text apart from an empty cell: in a workbook the
    // empty string and NULL look the same. CSV and JSON keep them apart (S14).
    assert!(!cells.contains_key("E3"));
}

#[test]
fn a_text_longer_than_a_cell_is_an_error() {
    let schema = Schema::new(vec![field("note", DataType::Utf8)]);
    let result = QueryResult::new(schema, vec![vec![Value::Utf8("x".repeat(32_768))]], 1);
    let mut writer = XlsxWriter::new("orders").unwrap();
    let error = writer.write(&result).unwrap_err();
    assert!(error.contains("32768 characters"), "{error}");
    let fits = QueryResult::new(
        Schema::new(vec![field("note", DataType::Utf8)]),
        vec![vec![Value::Utf8("x".repeat(32_767))]],
        1,
    );
    assert!(XlsxWriter::new("orders").unwrap().write(&fits).is_ok());
}

#[test]
fn pieces_continue_the_rows_and_the_header_comes_once() {
    let schema = Schema::new(vec![field("id", DataType::Int64)]);
    let piece = |from: i64| {
        QueryResult::new(
            schema.clone(),
            vec![(from..from + 3).map(Value::Int64).collect()],
            6,
        )
    };
    let mut writer = XlsxWriter::new("orders").unwrap();
    writer.write(&piece(1)).unwrap();
    writer.write(&piece(4)).unwrap();
    let cells = cells(&writer.finish().unwrap());
    assert_eq!(cells["A1"].text, "id");
    let ids: Vec<&str> = (2..=7)
        .map(|row| cells[&format!("A{row}")].text.as_str())
        .collect();
    assert_eq!(ids, ["1", "2", "3", "4", "5", "6"]);
    assert!(!cells.contains_key("A8"));
}

#[test]
fn an_empty_answer_is_a_sheet_with_its_header() {
    let schema = Schema::new(vec![
        field("id", DataType::Int64),
        field("note", DataType::Utf8),
    ]);
    let cells = cells(&workbook(&QueryResult::new(
        schema,
        vec![vec![], vec![]],
        0,
    )));
    assert_eq!(cells.len(), 2);
    assert_eq!(cells["B1"].text, "note");
}
