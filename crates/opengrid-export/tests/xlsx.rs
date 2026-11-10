//! The XLSX export (issue #72), opened again: every type's cell, the text
//! fallbacks where Excel cannot hold a value exactly, and Excel's limits.
#![cfg(feature = "xlsx")]

use std::io::Read;

use opengrid_datasource::QueryResult;
use opengrid_export::{CsvOptions, PivotLabels, XlsxWriter, pivot_csv};
use opengrid_pivot::{PivotColumn, PivotResult};
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

/// The element's English defaults.
struct English;

impl PivotLabels for English {
    fn dimension(&self, value: Option<&str>) -> String {
        match value {
            None => "(no value)".to_owned(),
            Some("") => "(empty)".to_owned(),
            Some(text) => text.to_owned(),
        }
    }
    fn total(&self) -> String {
        "Total".to_owned()
    }
    fn subtotal(&self, value: &str) -> String {
        format!("Total {value}")
    }
}

/// Two row dimensions — a text and a date — under one column dimension with a
/// NULL value, and a decimal measure: three data rows (a formula for a name,
/// the empty string, NULL), each with its subtotal, and the grand total.
fn pivot() -> PivotResult {
    let text = |value: &str| Value::Utf8(value.to_owned());
    let day = |day: u32| Value::Date(Date::from_ymd(2026, 9, day).unwrap());
    let amount = |cents: i128| Value::Decimal(Decimal::new(cents, 2));
    let measure = FieldName::new("total").unwrap();
    let fields = vec![
        field("customer", DataType::Utf8),
        field("day", DataType::Date),
        field("total_0", DataType::decimal(20, 2).unwrap()),
        field("total_1", DataType::decimal(20, 2).unwrap()),
    ];
    let null = Value::Null;
    let columns = vec![
        vec![
            text("=cmd"),
            text("=cmd"),
            text(""),
            text(""),
            null.clone(),
            null.clone(),
            null.clone(),
        ],
        vec![
            day(28),
            null.clone(),
            null.clone(),
            null.clone(),
            day(29),
            null.clone(),
            null.clone(),
        ],
        vec![
            amount(1250),
            amount(1250),
            amount(300),
            amount(300),
            null.clone(),
            null.clone(),
            amount(1550),
        ],
        vec![
            null.clone(),
            null.clone(),
            amount(-5),
            amount(-5),
            amount(700),
            amount(700),
            amount(695),
        ],
    ];
    PivotResult {
        data: QueryResult::new(Schema::new(fields), columns, 7),
        row_levels: vec![2, 1, 2, 1, 2, 1, 0],
        columns: vec![
            PivotColumn {
                path: vec![Value::Int64(2025)],
                measure: measure.clone(),
            },
            PivotColumn {
                path: vec![Value::Null],
                measure,
            },
        ],
    }
}

/// Issue #331: a pivot's sheet is its CSV — one header row, the same rows in
/// the same order — with the cells typed.
#[test]
fn a_pivot_is_the_flat_sheet_its_csv_is_with_typed_cells() {
    const COLUMNS: [&str; 4] = ["A", "B", "C", "D"];
    let pivot = pivot();
    let mut writer = XlsxWriter::new("orders").unwrap();
    writer.write_pivot(&pivot, &English).unwrap();
    let file = writer.finish().unwrap();
    let cells = cells(&file);

    // The same grid as the CSV: every line a row, every non-empty field a
    // cell at its place, and no cell anywhere else.
    let options = CsvOptions {
        bom: false,
        protect_formulas: false,
        ..CsvOptions::default()
    };
    let csv = pivot_csv(&pivot, &English, &options);
    let lines: Vec<Vec<&str>> = csv.lines().map(|line| line.split(',').collect()).collect();
    assert_eq!(lines.len(), 8, "{csv}");
    let mut filled = 0;
    for (row, line) in lines.iter().enumerate() {
        for (column, field) in line.iter().enumerate() {
            let reference = format!("{}{}", COLUMNS[column], row + 1);
            assert_eq!(
                cells.contains_key(&reference),
                !field.is_empty(),
                "{reference} against {field:?}"
            );
            filled += usize::from(!field.is_empty());
        }
    }
    assert_eq!(cells.len(), filled);
    // The header line is the CSV's, as text.
    assert_eq!(
        lines[0],
        ["customer", "day", "2025 · total", "(no value) · total"]
    );
    for (column, name) in lines[0].iter().enumerate() {
        let cell = &cells[&format!("{}1", COLUMNS[column])];
        assert_eq!(
            (cell.kind.as_deref(), cell.text.as_str()),
            (Some("inlineStr"), *name)
        );
    }

    let cell = |reference: &str| {
        let cell = &cells[reference];
        (cell.kind.as_deref(), cell.text.as_str())
    };
    // A data row: the name is a string — never a formula —, the date a date,
    // the measure a number with its scale.
    assert_eq!(cell("A2"), (Some("inlineStr"), "=cmd"));
    assert_eq!(cell("B2"), (None, "46293"));
    assert!(cells["B2"].styled, "a date carries its format");
    assert_eq!(cell("C2"), (None, "12.5"));
    assert!(cells["C2"].styled, "a decimal carries its scale");
    assert!(!cells.contains_key("D2"), "a NULL measure is an empty cell");
    // Its subtotal: the label, then the dimension it spans left empty.
    assert_eq!(cell("A3"), (Some("inlineStr"), "Total =cmd"));
    assert!(!cells.contains_key("B3"));
    assert_eq!(cell("C3"), (None, "12.5"));
    // The empty string and NULL are named, in a text column and in a date
    // column alike; a negative number stays a number.
    assert_eq!(cell("A4"), (Some("inlineStr"), "(empty)"));
    assert_eq!(cell("B4"), (Some("inlineStr"), "(no value)"));
    assert_eq!(cell("D4"), (None, "-0.05"));
    assert_eq!(cell("A5"), (Some("inlineStr"), "Total (empty)"));
    assert_eq!(cell("A6"), (Some("inlineStr"), "(no value)"));
    assert_eq!(cell("B6"), (None, "46294"));
    assert_eq!(cell("A7"), (Some("inlineStr"), "Total (no value)"));
    // The grand total.
    assert_eq!(cell("A8"), (Some("inlineStr"), "Total"));
    assert!(!cells.contains_key("B8"));
    assert_eq!((cell("C8"), cell("D8")), ((None, "15.5"), (None, "6.95")));

    // One flat sheet: nothing merged, no outline, no formula.
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&file[..])).unwrap();
    let mut xml = String::new();
    archive
        .by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    for absent in ["<mergeCell", "outlineLevel", "<f>"] {
        assert!(!xml.contains(absent), "{absent} in the sheet");
    }
}

/// A pivot without a row dimension is its grand total, a row of measures.
#[test]
fn a_pivot_without_a_row_dimension_is_one_row_of_measures() {
    let pivot = PivotResult {
        data: QueryResult::new(
            Schema::new(vec![field("n", DataType::Int64)]),
            vec![vec![Value::Int64(50)]],
            1,
        ),
        row_levels: vec![0],
        columns: vec![PivotColumn {
            path: Vec::new(),
            measure: FieldName::new("n").unwrap(),
        }],
    };
    let mut writer = XlsxWriter::new("orders").unwrap();
    writer.write_pivot(&pivot, &English).unwrap();
    let cells = cells(&writer.finish().unwrap());
    assert_eq!(cells.len(), 2);
    assert_eq!(cells["A1"].text, "n");
    assert_eq!(
        (cells["A2"].kind.as_deref(), cells["A2"].text.as_str()),
        (None, "50")
    );
}
