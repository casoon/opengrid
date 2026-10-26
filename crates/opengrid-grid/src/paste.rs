//! Reading a block of cells pasted from a spreadsheet (issue #184).
//!
//! A spreadsheet copies a block as tab-separated text, a cell holding a tab or
//! a line break in double quotes; a CSV file is the same with commas. The block
//! is split here, and each cell is read against the type of the column it
//! lands in — with the number notation of the page's language, because that is
//! how the person who copied it saw it. Nothing is guessed: a text that is not
//! a value of its column is `None`, and the caller says why.

use opengrid_json::Json;
use opengrid_types::{DataType, Value};

use crate::filter_row::literal;

/// How numbers are written where the grid is read: `1.234,5` in German,
/// `1,234.5` in English. The wire notation (`1234.5`) is always understood too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notation {
    pub decimal: char,
    pub group: Option<char>,
}

impl Default for Notation {
    fn default() -> Self {
        Self {
            decimal: '.',
            group: None,
        }
    }
}

/// Splits pasted text into rows of cells.
///
/// Tab is the delimiter when the text has one anywhere (what a spreadsheet
/// copies), a comma otherwise. A cell in double quotes may hold the delimiter
/// and line breaks; `""` inside is a quote. One trailing line break — every
/// spreadsheet ends the block with one — makes no row.
pub fn split_block(text: &str) -> Vec<Vec<String>> {
    let delimiter = if text.contains('\t') { '\t' } else { ',' };
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    // Whether anything of the current row has been seen: a final line break
    // does not start another.
    let mut started = false;
    while let Some(c) = chars.next() {
        started = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            c if c == delimiter => row.push(std::mem::take(&mut cell)),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                started = false;
            }
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                started = false;
            }
            c => cell.push(c),
        }
    }
    if started {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// A cell read against its column, or `None` when it is not a value of it.
///
/// An empty cell is NULL where the column may hold one — text keeps the empty
/// string, as it does when typed (S14) — and unreadable where it may not.
pub fn parse_cell(
    text: &str,
    data_type: DataType,
    nullable: bool,
    notation: &Notation,
) -> Option<Value> {
    if text.trim().is_empty() {
        return match data_type {
            DataType::Utf8 => Some(Value::Utf8(String::new())),
            _ if nullable => Some(Value::Null),
            _ => None,
        };
    }
    let text = match data_type {
        DataType::Bool => return bool_of(text).map(Value::Bool),
        DataType::Int64 | DataType::Float64 | DataType::Decimal { .. } => {
            // The page's notation first, the wire notation if that is not it.
            let read = |text: &str| {
                literal(text, data_type)
                    .and_then(|json| Value::from_json_typed(&json, &data_type).ok())
            };
            return normalize_number(text.trim(), notation)
                .and_then(|text| read(&text))
                .or_else(|| read(text));
        }
        _ => text,
    };
    let json: Json = literal(text, data_type)?;
    Value::from_json_typed(&json, &data_type).ok()
}

/// `true`/`false`/`1`/`0`, and the English `yes`/`no` a spreadsheet may show.
fn bool_of(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// A number in the page's notation as the wire notation: the group separator
/// out — only where it groups threes — and the decimal separator a point.
fn normalize_number(text: &str, notation: &Notation) -> Option<String> {
    let (sign, digits) = match text.strip_prefix(['-', '+']) {
        Some(rest) => (&text[..1], rest),
        None => ("", text),
    };
    let (whole, fraction) = match digits.split_once(notation.decimal) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let whole = match notation.group {
        Some(group) if whole.contains(group) => {
            let parts: Vec<&str> = whole.split(group).collect();
            let grouped = parts[0].len() <= 3
                && !parts[0].is_empty()
                && parts[1..].iter().all(|part| part.len() == 3);
            if !grouped {
                return None;
            }
            parts.concat()
        }
        _ => whole.to_owned(),
    };
    Some(match fraction {
        Some(fraction) => format!("{sign}{whole}.{fraction}"),
        None => format!("{sign}{whole}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::Decimal;

    #[test]
    fn a_spreadsheet_block_is_tabs_and_lines() {
        assert_eq!(
            split_block("a\tb\r\nc\td\r\n"),
            vec![vec!["a", "b"], vec!["c", "d"]]
        );
    }

    #[test]
    fn quotes_hold_tabs_line_breaks_and_quotes() {
        assert_eq!(
            split_block("\"x\ty\"\t\"line\nbreak\"\t\"say \"\"hi\"\"\"\n"),
            vec![vec!["x\ty", "line\nbreak", "say \"hi\""]]
        );
    }

    #[test]
    fn without_a_tab_commas_separate_and_one_cell_is_one_cell() {
        assert_eq!(
            split_block("1,2\n3,4"),
            vec![vec!["1", "2"], vec!["3", "4"]]
        );
        assert_eq!(split_block("just text"), vec![vec!["just text"]]);
        assert!(split_block("").is_empty());
    }

    #[test]
    fn empty_cells_and_rows_are_kept() {
        assert_eq!(
            split_block("a\t\tc\n\nd"),
            vec![vec!["a", "", "c"], vec![""], vec!["d"]]
        );
    }

    #[test]
    fn a_german_decimal_reads_as_a_decimal() {
        let german = Notation {
            decimal: ',',
            group: Some('.'),
        };
        let decimal = DataType::Decimal {
            precision: 12,
            scale: 2,
        };
        assert_eq!(
            parse_cell("1.234,50", decimal, true, &german),
            Some(Value::Decimal(Decimal::new(123_450, 2)))
        );
        assert_eq!(
            parse_cell("12,5", decimal, true, &german),
            Some(Value::Decimal(Decimal::new(1250, 2)))
        );
        // The wire notation is understood where it is not a grouping.
        assert_eq!(
            parse_cell("1.5", decimal, true, &german),
            Some(Value::Decimal(Decimal::new(150, 2)))
        );
        assert_eq!(
            parse_cell("1.234", DataType::Int64, true, &german),
            Some(Value::Int64(1234))
        );
        assert_eq!(parse_cell("abc", decimal, true, &german), None);
    }

    #[test]
    fn empty_is_null_where_allowed_and_unreadable_where_not() {
        let plain = Notation::default();
        assert_eq!(
            parse_cell("", DataType::Int64, true, &plain),
            Some(Value::Null)
        );
        assert_eq!(parse_cell("", DataType::Int64, false, &plain), None);
        assert_eq!(
            parse_cell("", DataType::Utf8, false, &plain),
            Some(Value::Utf8(String::new()))
        );
    }

    #[test]
    fn text_that_looks_like_a_formula_stays_text() {
        assert_eq!(
            parse_cell("=SUM(A1:A9)", DataType::Utf8, true, &Notation::default()),
            Some(Value::Utf8("=SUM(A1:A9)".to_owned()))
        );
        assert_eq!(
            parse_cell("=1+1", DataType::Int64, true, &Notation::default()),
            None
        );
    }

    #[test]
    fn booleans_and_dates() {
        let plain = Notation::default();
        assert_eq!(
            parse_cell("Yes", DataType::Bool, true, &plain),
            Some(Value::Bool(true))
        );
        assert_eq!(parse_cell("maybe", DataType::Bool, true, &plain), None);
        assert!(parse_cell("2026-10-26", DataType::Date, true, &plain).is_some());
        assert_eq!(parse_cell("26.10.2026", DataType::Date, true, &plain), None);
    }
}
