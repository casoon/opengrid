//! CSV after RFC 4180, as spreadsheets read it (E33).
//!
//! - **UTF-8 with a byte order mark** by default: without one, Excel reads
//!   UTF-8 as the system code page and every `ä` breaks.
//! - Lines end in CRLF; a field is quoted only when it has to be — it holds the
//!   delimiter, a quote or a line break — and an empty *string* is always
//!   quoted, `""`, so that it stays apart from NULL (S14), which is written as
//!   an unquoted empty field (or as [`CsvOptions::null`]). With one column and
//!   NULL written empty, a NULL row is an empty line, which most readers skip:
//!   spell NULL for such an export.
//! - **Formula injection** (OWASP): a text cell that starts with `=`, `+`, `-`,
//!   `@`, a tab or a carriage return is a formula to a spreadsheet — an export
//!   of user data must not become a way to run one. Such a cell gets a leading
//!   `'`. Only in text columns: a number column's `-5` is a number, not an
//!   attack, and prefixing it would change the data. With the guard on, a
//!   field that holds `,`, `;` or a tab is quoted whatever the delimiter: a
//!   spreadsheet that splits on the *other* list separator would otherwise cut
//!   `x;=cmd…` in two and run the second half.

use opengrid_datasource::QueryResult;
use opengrid_types::{DataType, Schema};

use crate::plain;

/// How a CSV is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvOptions {
    /// The field delimiter: `,` by default, `;` for a German Excel.
    pub delimiter: char,
    /// Start with a UTF-8 byte order mark. On by default (Excel).
    pub bom: bool,
    /// Prefix text cells that a spreadsheet would run as a formula. On by
    /// default; off for a receiver that needs the text exactly.
    pub protect_formulas: bool,
    /// What NULL is written as. Empty by default; `\N` makes the file read back
    /// into opengrid's own ingest unchanged.
    pub null: String,
}

impl CsvOptions {
    /// Whether these options write a readable file: the delimiter is not a
    /// quote or a line break, and the NULL spelling needs no quoting. With the
    /// formula guard on, the NULL spelling must not read as a formula either —
    /// it is written unguarded into every empty cell, so `=1+1` there would be
    /// the very thing the guard keeps out of the file. The writers trust
    /// options that passed this; check at the boundary.
    pub fn check(&self) -> Result<(), &'static str> {
        if matches!(self.delimiter, '"' | '\n' | '\r') {
            return Err("delimiter: one character, not a quote or a line break");
        }
        if self
            .null
            .chars()
            .any(|c| c == self.delimiter || matches!(c, '"' | '\n' | '\r'))
        {
            return Err("null: without the delimiter, a quote or a line break");
        }
        if self.protect_formulas && is_formula(&self.null) {
            return Err(
                "null: not starting with =, +, -, @ or a tab while the formula guard (protectFormulas) is on",
            );
        }
        Ok(())
    }
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ',',
            bom: true,
            protect_formulas: true,
            null: String::new(),
        }
    }
}

/// The header line — the byte order mark first, when asked for.
pub fn csv_header(schema: &Schema, options: &CsvOptions) -> String {
    let mut out = String::new();
    if options.bom {
        out.push('\u{FEFF}');
    }
    let names: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| quoted(field.name.as_str(), options))
        .collect();
    out.push_str(&names.join(&options.delimiter.to_string()));
    out.push_str("\r\n");
    out
}

/// The rows of one result, each line ending in CRLF; no header.
pub fn csv_rows(result: &QueryResult, options: &CsvOptions) -> String {
    let text_columns: Vec<bool> = result
        .schema
        .fields()
        .iter()
        .map(|field| field.data_type == DataType::Utf8)
        .collect();
    let delimiter = options.delimiter.to_string();
    let mut out = String::new();
    for row in 0..result.row_count() {
        let cells: Vec<String> = result
            .columns
            .iter()
            .enumerate()
            .map(|(col, column)| cell(&column[row], text_columns[col], options))
            .collect();
        out.push_str(&cells.join(&delimiter));
        out.push_str("\r\n");
    }
    out
}

/// A CSV written in pieces: the header with the first result, rows after.
#[derive(Debug)]
pub struct CsvWriter {
    options: CsvOptions,
    started: bool,
}

impl CsvWriter {
    pub fn new(options: CsvOptions) -> Self {
        Self {
            options,
            started: false,
        }
    }

    /// The next piece: the header before the first rows, then the rows. An
    /// empty result still writes the header, so an export of nothing says
    /// what it would have held.
    pub fn write(&mut self, result: &QueryResult) -> String {
        let mut out = String::new();
        if !self.started {
            self.started = true;
            out.push_str(&csv_header(&result.schema, &self.options));
        }
        out.push_str(&csv_rows(result, &self.options));
        out
    }
}

pub(crate) fn cell(
    value: &opengrid_types::Value,
    text_column: bool,
    options: &CsvOptions,
) -> String {
    match plain(value) {
        Some(text) => text_cell(text, text_column, options),
        None => options.null.clone(),
    }
}

/// A cell that holds text: a value's, or a label a pivot writes in place of
/// one. `guard`: whether the formula guard applies — a text column's value, or
/// any label.
pub(crate) fn text_cell(text: String, guard: bool, options: &CsvOptions) -> String {
    if text.is_empty() {
        return "\"\"".to_owned();
    }
    let text = if guard && options.protect_formulas && is_formula(&text) {
        format!("'{text}")
    } else {
        text
    };
    // A real value that reads like the NULL spelling is quoted, so a reader
    // that tells quoted from unquoted keeps them apart.
    if !options.null.is_empty() && text == options.null {
        return format!("\"{text}\"");
    }
    quoted(&text, options)
}

fn is_formula(text: &str) -> bool {
    matches!(
        text.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    )
}

fn quoted(text: &str, options: &CsvOptions) -> String {
    let needs = text.contains(options.delimiter)
        || text.contains('"')
        || text.contains('\n')
        || text.contains('\r')
        || (options.protect_formulas && text.contains([',', ';', '\t']));
    if needs {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{Field, FieldName, Value};

    fn field(name: &str, data_type: DataType) -> Field {
        Field::new(FieldName::new(name).unwrap(), data_type)
    }

    fn one_row(fields: Vec<Field>, values: Vec<Value>) -> QueryResult {
        let columns = values.into_iter().map(|value| vec![value]).collect();
        QueryResult::new(Schema::new(fields), columns, 1)
    }

    fn plain_options() -> CsvOptions {
        CsvOptions {
            bom: false,
            ..CsvOptions::default()
        }
    }

    #[test]
    fn every_type_is_written_in_the_wire_notation() {
        let result = one_row(
            vec![
                field("b", DataType::Bool),
                field("i", DataType::Int64),
                field("f", DataType::Float64),
                field("n", DataType::Float64),
                field(
                    "d",
                    DataType::Decimal {
                        precision: 14,
                        scale: 2,
                    },
                ),
                field("day", DataType::Date),
                field("at", DataType::Timestamp),
                field("t", DataType::Utf8),
            ],
            vec![
                Value::Bool(true),
                Value::Int64(-5),
                Value::Float64(360.001),
                Value::Float64(f64::NAN),
                Value::from_wire_str(
                    "123456789012.34",
                    &DataType::Decimal {
                        precision: 14,
                        scale: 2,
                    },
                )
                .unwrap(),
                Value::from_wire_str("2026-09-26", &DataType::Date).unwrap(),
                Value::from_wire_str("2026-09-26T08:15:00.000001Z", &DataType::Timestamp).unwrap(),
                Value::Utf8("Alpha".to_owned()),
            ],
        );
        assert_eq!(
            csv_rows(&result, &plain_options()),
            "true,-5,360.001,NaN,123456789012.34,2026-09-26,2026-09-26T08:15:00.000001Z,Alpha\r\n"
        );
    }

    #[test]
    fn null_and_the_empty_string_stay_apart() {
        let fields = vec![field("a", DataType::Utf8), field("b", DataType::Utf8)];
        let result = one_row(fields, vec![Value::Null, Value::Utf8(String::new())]);
        assert_eq!(csv_rows(&result, &plain_options()), ",\"\"\r\n");
        let spelled = CsvOptions {
            null: "\\N".to_owned(),
            ..plain_options()
        };
        assert_eq!(csv_rows(&result, &spelled), "\\N,\"\"\r\n");
        // A text that reads like the NULL spelling is quoted.
        let looks = one_row(
            vec![field("a", DataType::Utf8)],
            vec![Value::Utf8("\\N".to_owned())],
        );
        assert_eq!(csv_rows(&looks, &spelled), "\"\\N\"\r\n");
    }

    #[test]
    fn a_field_is_quoted_only_when_it_has_to_be() {
        let result = one_row(
            vec![
                field("a", DataType::Utf8),
                field("b", DataType::Utf8),
                field("c", DataType::Utf8),
                field("d", DataType::Utf8),
            ],
            vec![
                Value::Utf8("plain".to_owned()),
                Value::Utf8("a,b".to_owned()),
                Value::Utf8("say \"hi\"".to_owned()),
                Value::Utf8("one\ntwo".to_owned()),
            ],
        );
        assert_eq!(
            csv_rows(&result, &plain_options()),
            "plain,\"a,b\",\"say \"\"hi\"\"\",\"one\ntwo\"\r\n"
        );
        let semicolon = CsvOptions {
            delimiter: ';',
            ..plain_options()
        };
        let raw_semicolon = CsvOptions {
            protect_formulas: false,
            ..semicolon.clone()
        };
        assert_eq!(
            csv_rows(&result, &raw_semicolon),
            "plain;a,b;\"say \"\"hi\"\"\";\"one\ntwo\"\r\n"
        );
    }

    #[test]
    fn the_guard_quotes_the_other_list_separator_too() {
        // Unquoted, a spreadsheet splitting on `;` would read a second cell
        // `=cmd…` out of the first — and on `,` the same the other way round.
        let result = one_row(
            vec![field("a", DataType::Utf8), field("b", DataType::Utf8)],
            vec![
                Value::Utf8("x;=cmd|' /C calc'!A0".to_owned()),
                Value::Utf8("a\t=1+1".to_owned()),
            ],
        );
        assert_eq!(
            csv_rows(&result, &plain_options()),
            "\"x;=cmd|' /C calc'!A0\",\"a\t=1+1\"\r\n"
        );
        let semicolon = CsvOptions {
            delimiter: ';',
            ..plain_options()
        };
        let comma = one_row(
            vec![field("a", DataType::Utf8)],
            vec![Value::Utf8("a,=1+1".to_owned())],
        );
        assert_eq!(csv_rows(&comma, &semicolon), "\"a,=1+1\"\r\n");
    }

    #[test]
    fn options_that_would_break_the_file_are_refused() {
        assert!(CsvOptions::default().check().is_ok());
        for delimiter in ['"', '\n', '\r'] {
            let options = CsvOptions {
                delimiter,
                ..CsvOptions::default()
            };
            assert!(options.check().is_err(), "{delimiter:?}");
        }
        for null in [",", "a\"b", "\n", "\r"] {
            let options = CsvOptions {
                null: null.to_owned(),
                ..CsvOptions::default()
            };
            assert!(options.check().is_err(), "{null:?}");
        }
        let semicolon_null = CsvOptions {
            delimiter: ';',
            null: "a,b".to_owned(),
            ..CsvOptions::default()
        };
        assert!(semicolon_null.check().is_ok());
    }

    /// NULL is written unguarded into every empty cell, so under the guard
    /// its spelling must not be a formula — and without the guard it may be.
    #[test]
    fn a_null_spelled_as_a_formula_is_refused_under_the_guard() {
        for null in ["=1+1", "+1", "-", "@SUM(A1)", "\tx"] {
            let guarded = CsvOptions {
                null: null.to_owned(),
                ..CsvOptions::default()
            };
            let message = guarded.check().expect_err(null);
            assert!(message.starts_with("null:"), "{message}");
            assert!(message.contains("protectFormulas"), "{message}");
            let raw = CsvOptions {
                protect_formulas: false,
                ..guarded
            };
            assert!(raw.check().is_ok(), "{null:?} without the guard");
        }
        // Not a formula: `\N`, `NULL`, an empty spelling, a `-` inside.
        for null in ["\\N", "NULL", "", "n-a"] {
            let options = CsvOptions {
                null: null.to_owned(),
                ..CsvOptions::default()
            };
            assert!(options.check().is_ok(), "{null:?}");
        }
    }

    #[test]
    fn a_formula_in_a_text_cell_is_defused_and_a_number_is_left_alone() {
        let fields = vec![
            field("t", DataType::Utf8),
            field("u", DataType::Utf8),
            field("v", DataType::Utf8),
            field("w", DataType::Utf8),
            field("n", DataType::Int64),
        ];
        let result = one_row(
            fields,
            vec![
                Value::Utf8("=HYPERLINK(\"http://x\")".to_owned()),
                Value::Utf8("+1".to_owned()),
                Value::Utf8("@SUM(A1)".to_owned()),
                Value::Utf8("-5".to_owned()),
                Value::Int64(-5),
            ],
        );
        assert_eq!(
            csv_rows(&result, &plain_options()),
            "\"'=HYPERLINK(\"\"http://x\"\")\",'+1,'@SUM(A1),'-5,-5\r\n"
        );
        let raw = CsvOptions {
            protect_formulas: false,
            ..plain_options()
        };
        assert_eq!(
            csv_rows(&result, &raw),
            "\"=HYPERLINK(\"\"http://x\"\")\",+1,@SUM(A1),-5,-5\r\n"
        );
    }

    #[test]
    fn the_header_comes_once_with_the_mark_and_an_empty_export_still_has_it() {
        let schema = Schema::new(vec![
            field("id", DataType::Int64),
            field("note", DataType::Utf8),
        ]);
        let mut writer = CsvWriter::new(CsvOptions::default());
        let empty = QueryResult::new(schema.clone(), vec![Vec::new(), Vec::new()], 0);
        assert_eq!(writer.write(&empty), "\u{FEFF}id,note\r\n");
        let rows = QueryResult::new(
            schema,
            vec![vec![Value::Int64(1)], vec![Value::Utf8("é".to_owned())]],
            1,
        );
        assert_eq!(writer.write(&rows), "1,é\r\n");
    }
}
