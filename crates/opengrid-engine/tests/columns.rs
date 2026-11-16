//! Rows that arrive as packs of columns (issue #445): the table is the table the JSON route makes
//! of the same rows, however the rows are cut into packs, and what the pack gets wrong is refused.
//! The pack is built here from the written-down format, not by the code under test.

use opengrid_engine::Table;
use opengrid_engine::ingest::{ColumnsLoader, load_json};
use opengrid_types::{DataType, DatePart, Field, FieldName, Schema};

/// One cell as a page holds it.
#[derive(Clone, Debug)]
enum Cell {
    Null,
    Bool(bool),
    Number(f64),
    Text(&'static str),
}

fn name(text: &str) -> FieldName {
    FieldName::new(text).unwrap()
}

/// A pack of `rows` (one `Vec<Cell>` per row, one cell per **stored** column), by the format.
fn pack(schema: &Schema, rows: &[Vec<Cell>]) -> Vec<u8> {
    let stored = schema.stored();
    let width = stored.fields().len();
    let n = rows.len();
    let mut head = Vec::new();
    head.extend_from_slice(&(n as u32).to_le_bytes());
    let mut body: Vec<u8> = Vec::new();
    let mut sections: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for (index, field) in stored.fields().iter().enumerate() {
        let cells: Vec<&Cell> = rows.iter().map(|row| &row[index]).collect();
        let has_nulls = cells.iter().any(|cell| matches!(cell, Cell::Null));
        let mut section = Vec::new();
        if has_nulls {
            let mut bits = vec![0u8; n.div_ceil(8)];
            for (row, cell) in cells.iter().enumerate() {
                if !matches!(cell, Cell::Null) {
                    bits[row / 8] |= 1 << (row % 8);
                }
            }
            section.extend_from_slice(&bits);
            while section.len() % 8 != 0 {
                section.push(0);
            }
        }
        let mut text_len = 0u32;
        match field.data_type {
            DataType::Bool => {
                for cell in &cells {
                    section.push(match cell {
                        Cell::Bool(flag) => u8::from(*flag),
                        _ => 0,
                    });
                }
            }
            DataType::Int64 | DataType::Float64 => {
                for cell in &cells {
                    let number = match cell {
                        Cell::Number(number) => *number,
                        _ => 0.0,
                    };
                    section.extend_from_slice(&number.to_le_bytes());
                }
            }
            _ => {
                let mut offsets = vec![0u32];
                let mut text = Vec::new();
                for cell in &cells {
                    if let Cell::Text(value) = cell {
                        text.extend_from_slice(value.as_bytes());
                    }
                    offsets.push(text.len() as u32);
                }
                for offset in offsets {
                    section.extend_from_slice(&offset.to_le_bytes());
                }
                text_len = text.len() as u32;
                section.extend_from_slice(&text);
            }
        }
        while section.len() % 8 != 0 {
            section.push(0);
        }
        sections.push((u32::from(has_nulls), text_len, section));
    }
    for (flags, text_len, _) in &sections {
        head.extend_from_slice(&flags.to_le_bytes());
        head.extend_from_slice(&text_len.to_le_bytes());
    }
    assert_eq!(head.len(), 4 + 8 * width);
    while head.len() % 8 != 0 {
        head.push(0);
    }
    body.extend_from_slice(&head);
    for (_, _, section) in sections {
        body.extend_from_slice(&section);
    }
    body
}

/// The same rows as the JSON the page would have sent.
fn json(schema: &Schema, rows: &[Vec<Cell>]) -> String {
    let stored = schema.stored();
    let objects: Vec<String> = rows
        .iter()
        .map(|row| {
            let members: Vec<String> = stored
                .fields()
                .iter()
                .zip(row)
                .map(|(field, cell)| {
                    let value = match cell {
                        Cell::Null => "null".to_owned(),
                        Cell::Bool(flag) => flag.to_string(),
                        Cell::Number(number) => format!("{number}"),
                        Cell::Text(text) => format!("{text:?}"),
                    };
                    format!("\"{}\": {value}", field.name.as_str())
                })
                .collect();
            format!("{{{}}}", members.join(", "))
        })
        .collect();
    format!("[{}]", objects.join(", "))
}

fn columns(schema: &Schema, rows: &[Vec<Cell>], cuts: &[usize]) -> Result<Table, String> {
    let mut loader = ColumnsLoader::new(schema, rows.len()).map_err(|error| error.to_string())?;
    let mut from = 0;
    for cut in cuts.iter().copied().chain([rows.len()]) {
        loader
            .feed(&pack(schema, &rows[from..cut]))
            .map_err(|error| error.to_string())?;
        from = cut;
    }
    loader.finish().map_err(|error| error.to_string())
}

/// Every type, a NULL in each nullable column, text that is empty, multi-byte and quoted, and the
/// derived columns: one before its source, two of one source, one of a timestamp.
fn everything() -> (Schema, Vec<Vec<Cell>>) {
    let schema = Schema::new(vec![
        Field::derived(name("made_month"), DatePart::Month, name("made")),
        Field::required(name("id"), DataType::Int64),
        Field::derived(name("day_year"), DatePart::Year, name("day")),
        Field::new(name("flag"), DataType::Bool),
        Field::new(name("note"), DataType::Utf8),
        Field::new(name("ratio"), DataType::Float64),
        Field::new(name("amount"), DataType::decimal(12, 2).unwrap()),
        Field::new(name("day"), DataType::Date),
        Field::new(name("made"), DataType::Timestamp),
        Field::derived(name("day_month"), DatePart::Month, name("day")),
        Field::derived(name("made_year"), DatePart::Year, name("made")),
    ]);
    schema.check().unwrap();
    let rows = vec![
        vec![
            Cell::Number(1.0),
            Cell::Bool(true),
            Cell::Text("plain"),
            Cell::Number(1.5),
            Cell::Text("10.5"),
            Cell::Text("2019-02-03"),
            Cell::Text("2031-11-30T23:59:59Z"),
        ],
        vec![
            Cell::Number(2.0),
            Cell::Bool(false),
            Cell::Text(""),
            Cell::Number(-0.25),
            Cell::Text("-0.01"),
            Cell::Null,
            Cell::Text("2024-07-01T00:00:00.123456Z"),
        ],
        vec![
            Cell::Number(-9_007_199_254_740_991.0),
            Cell::Null,
            Cell::Null,
            Cell::Null,
            Cell::Null,
            Cell::Text("2022-12-31"),
            Cell::Null,
        ],
        vec![
            Cell::Number(4.0),
            Cell::Bool(true),
            Cell::Text("ä ö ü — 日本語"),
            Cell::Number(1e300),
            Cell::Text("100"),
            Cell::Null,
            Cell::Null,
        ],
        vec![
            Cell::Number(5.0),
            Cell::Null,
            Cell::Text("say \"hi\"\nline"),
            Cell::Number(-1e-300),
            Cell::Text("999999999.99"),
            Cell::Text("1970-01-01"),
            Cell::Text("1969-12-31T23:59:59.999999Z"),
        ],
    ];
    (schema, rows)
}

#[test]
fn a_table_of_columns_is_the_table_of_json() {
    let (schema, rows) = everything();
    let want = load_json(json(&schema, &rows).as_bytes(), &schema).unwrap();
    assert_eq!(columns(&schema, &rows, &[]).unwrap(), want);
    // Cut into packs anywhere, a pack of one row among them, the table is the same.
    for cuts in [vec![1], vec![2, 3], vec![1, 2, 3, 4], vec![4]] {
        assert_eq!(
            columns(&schema, &rows, &cuts).unwrap(),
            want,
            "cut at {cuts:?}"
        );
    }
}

#[test]
fn what_the_pack_gets_wrong_is_refused() {
    let (schema, rows) = everything();
    let refused = |rows: &[Vec<Cell>]| columns(&schema, rows, &[]).unwrap_err();

    // A NULL in a required column, naming the row and the column.
    let mut bad = rows.clone();
    bad[3][0] = Cell::Null;
    assert!(
        refused(&bad).contains("row 3, field id"),
        "{}",
        refused(&bad)
    );
    // An int64 that is not an integer, or too big to be exact.
    for number in [1.5, 9_007_199_254_740_994.0, 1e19] {
        let mut bad = rows.clone();
        bad[0][0] = Cell::Number(number);
        assert!(refused(&bad).contains("is not an int64"), "{number}");
    }
    // A float64 that is not finite.
    for number in [f64::NAN, f64::INFINITY] {
        let mut bad = rows.clone();
        bad[0][3] = Cell::Number(number);
        assert!(refused(&bad).contains("is not a float64"), "{number}");
    }
    // Text that the column's own parser refuses: a decimal with too many places, a date that does
    // not exist, a timestamp without its Z.
    for (column, text) in [(4, "1.005"), (5, "2026-02-30"), (6, "2026-03-01T12:00:00")] {
        let mut bad = rows.clone();
        bad[0][column] = Cell::Text(text);
        assert!(!refused(&bad).is_empty(), "{text}");
    }
}

#[test]
fn a_pack_that_does_not_hold_together_is_refused() {
    let (schema, rows) = everything();
    let good = pack(&schema, &rows);
    let feed = |bytes: &[u8]| {
        let mut loader = ColumnsLoader::new(&schema, rows.len()).unwrap();
        loader.feed(bytes).map_err(|error| error.to_string())
    };
    assert!(feed(&good).is_ok());
    // Cut short, padded, a header that says more rows than were announced.
    assert!(feed(&good[..good.len() - 8]).is_err());
    assert!(feed(&good[..3]).is_err());
    let mut longer = good.clone();
    longer.extend_from_slice(&[0; 8]);
    assert!(feed(&longer).is_err());
    let mut more = good.clone();
    more[0] = 6;
    assert!(feed(&more).is_err());
    // A bool that is neither 0 nor 1.
    let flag_schema = Schema::new(vec![Field::new(name("flag"), DataType::Bool)]);
    let mut flags = pack(
        &flag_schema,
        &[vec![Cell::Bool(true)], vec![Cell::Bool(false)]],
    );
    let at = flags.len() - 8;
    flags[at] = 2;
    let mut loader = ColumnsLoader::new(&flag_schema, 2).unwrap();
    assert!(
        loader
            .feed(&flags)
            .unwrap_err()
            .to_string()
            .contains("is not a bool")
    );
    // Text whose offsets cut a character in two.
    let text_schema = Schema::new(vec![Field::new(name("t"), DataType::Utf8)]);
    let mut text = pack(
        &text_schema,
        &[vec![Cell::Text("ä")], vec![Cell::Text("b")]],
    );
    // offsets are at 16..: [0, 2, 3]; make the first one 1, inside the two-byte "ä".
    text[16 + 4] = 1;
    let mut loader = ColumnsLoader::new(&text_schema, 2).unwrap();
    assert!(loader.feed(&text).is_err());
    // Offsets that do not span the text the header counts: the last one a byte short, and a first
    // one that is not 0.
    let mut short = pack(
        &text_schema,
        &[vec![Cell::Text("ä")], vec![Cell::Text("b")]],
    );
    short[16 + 8] = 2;
    let mut loader = ColumnsLoader::new(&text_schema, 2).unwrap();
    assert!(
        loader
            .feed(&short)
            .unwrap_err()
            .to_string()
            .contains("do not span")
    );
    let mut late = pack(
        &text_schema,
        &[vec![Cell::Text("ab")], vec![Cell::Text("c")]],
    );
    late[16] = 1;
    let mut loader = ColumnsLoader::new(&text_schema, 2).unwrap();
    assert!(
        loader
            .feed(&late)
            .unwrap_err()
            .to_string()
            .contains("do not span")
    );
    // Fewer rows than announced, more rows than announced.
    let mut loader = ColumnsLoader::new(&schema, rows.len() + 1).unwrap();
    loader.feed(&good).unwrap();
    assert!(
        loader
            .finish()
            .unwrap_err()
            .to_string()
            .contains("were announced")
    );
    let mut loader = ColumnsLoader::new(&schema, rows.len()).unwrap();
    loader.feed(&good).unwrap();
    assert!(loader.feed(&good).is_err());
}

#[test]
fn a_list_column_is_not_read_from_columns() {
    let schema = Schema::new(vec![Field::new(name("tags"), DataType::ListUtf8)]);
    assert!(ColumnsLoader::new(&schema, 1).is_err());
}

#[test]
fn no_rows_are_an_empty_table_of_the_schema() {
    let (schema, _) = everything();
    let table = ColumnsLoader::new(&schema, 0).unwrap().finish().unwrap();
    assert_eq!(table.num_rows(), 0);
    assert_eq!(table.schema().len(), schema.len(), "the columns survive");
}
