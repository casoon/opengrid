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
            return parse_number(text, data_type, notation).ok();
        }
        _ => text,
    };
    let json: Json = literal(text, data_type)?;
    Value::from_json_typed(&json, &data_type).ok()
}

/// Why a number was refused, in the words the reader is told (issue #535). The
/// reader is one function: the cell editor, the record form, the bulk panel and
/// a paste all go through [`parse_number`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberProblem {
    /// Not a number at all: letters, two decimal marks, a group mark in the
    /// wrong place.
    NotANumber,
    /// A number, but the column holds whole numbers.
    NotAnInteger,
    /// More decimal places than the column's scale; never rounded.
    Decimals { scale: u8 },
    /// A value that does not fit the column.
    TooLarge,
}

/// A number typed or pasted into a column of `data_type`, in the page's
/// notation — decimal mark, group marks only where they stand right, a leading
/// sign — or in the wire notation (`1234.5`), which is understood as well.
///
/// **Nothing is rounded:** a decimal with more places than its scale is refused
/// with [`NumberProblem::Decimals`], and so is a value past the precision. An
/// empty text is not a number; what empty means is the caller's to decide.
pub fn parse_number(
    text: &str,
    data_type: DataType,
    notation: &Notation,
) -> Result<Value, NumberProblem> {
    let text = text.trim();
    let read = |wire: &str| {
        let json = literal(wire, data_type).ok_or_else(|| shape_problem(wire, data_type))?;
        Value::from_json_typed(&json, &data_type).map_err(|_| range_problem(wire, data_type))
    };
    // The page's notation first, the wire notation if that is not it. Of two
    // refusals the more specific one is told.
    match normalize_number(text, notation) {
        Some(wire) => read(&wire).or_else(|problem| {
            read(text).map_err(|other| match problem {
                NumberProblem::NotANumber => other,
                specific => specific,
            })
        }),
        None => read(text),
    }
}

/// A number the way the page writes it: the wire text with the notation's
/// decimal mark and no group marks — what an editor opens with, so a German
/// reader meets `1,10` and not `1.1`.
pub fn number_text(wire: &str, notation: &Notation) -> String {
    wire.replacen('.', &notation.decimal.to_string(), 1)
}

/// Why `wire` has not the shape of a value of `data_type`.
fn shape_problem(wire: &str, data_type: DataType) -> NumberProblem {
    let digits = wire.strip_prefix(['-', '+']).unwrap_or(wire);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let plain = !whole.is_empty()
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.bytes().all(|b| b.is_ascii_digit());
    match data_type {
        DataType::Int64 if plain && digits.contains('.') => NumberProblem::NotAnInteger,
        DataType::Int64 if plain => NumberProblem::TooLarge,
        _ => NumberProblem::NotANumber,
    }
}

/// Why `wire`, of the right shape, is not a value of `data_type`.
fn range_problem(wire: &str, data_type: DataType) -> NumberProblem {
    let DataType::Decimal { scale, .. } = data_type else {
        return NumberProblem::NotANumber;
    };
    let fraction = wire.split_once('.').map_or("", |(_, fraction)| fraction);
    let lost = fraction
        .get(usize::from(scale)..)
        .is_some_and(|extra| extra.bytes().any(|b| b != b'0'));
    match (lost, scale) {
        (true, 0) => NumberProblem::NotAnInteger,
        (true, scale) => NumberProblem::Decimals { scale },
        (false, _) => NumberProblem::TooLarge,
    }
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

    const GERMAN: Notation = Notation {
        decimal: ',',
        group: Some('.'),
    };
    const ENGLISH: Notation = Notation {
        decimal: '.',
        group: Some(','),
    };
    const MONEY: DataType = DataType::Decimal {
        precision: 12,
        scale: 2,
    };

    fn money(coefficient: i128) -> Result<Value, NumberProblem> {
        Ok(Value::Decimal(Decimal::new(coefficient, 2)))
    }

    #[test]
    fn a_number_is_read_in_the_notation_of_the_page_and_in_the_wire_notation() {
        // German: the comma is the decimal mark, the point a group mark.
        for (text, expected) in [
            ("1,10", 110),
            ("1.10", 110),
            ("1.234,50", 123_450),
            ("-1.234,5", -123_450),
            ("+3", 300),
            ("  12,5 ", 1250),
            ("1.234", 123_400),
            ("1.234.567,89", 123_456_789),
        ] {
            assert_eq!(
                parse_number(text, MONEY, &GERMAN),
                money(expected),
                "{text}"
            );
        }
        // English: the other way round.
        for (text, expected) in [
            ("1.10", 110),
            ("1,234.50", 123_450),
            ("1,234", 123_400),
            ("-0.5", -50),
        ] {
            assert_eq!(
                parse_number(text, MONEY, &ENGLISH),
                money(expected),
                "{text}"
            );
        }
        // Without group marks any thousands are plain digits.
        assert_eq!(
            parse_number(
                "1234,5",
                MONEY,
                &Notation {
                    decimal: ',',
                    group: None
                }
            ),
            money(123_450)
        );
    }

    #[test]
    fn what_is_not_a_number_in_the_notation_is_refused() {
        for text in [
            "1,1,0",
            "abc",
            "1.23,4",
            "12.34.567,8",
            "-",
            "",
            "1,10 €",
            "--1",
        ] {
            assert_eq!(
                parse_number(text, MONEY, &GERMAN),
                Err(NumberProblem::NotANumber),
                "{text:?}"
            );
        }
        // On an English page a comma is a group mark, not a decimal mark.
        for text in ["1,10", "1,1,0", "1.2.3"] {
            assert_eq!(
                parse_number(text, MONEY, &ENGLISH),
                Err(NumberProblem::NotANumber),
                "{text:?}"
            );
        }
    }

    #[test]
    fn more_decimals_than_the_scale_are_refused_never_rounded() {
        assert_eq!(
            parse_number("1,234", MONEY, &GERMAN),
            Err(NumberProblem::Decimals { scale: 2 })
        );
        assert_eq!(
            parse_number("1.005", MONEY, &ENGLISH),
            Err(NumberProblem::Decimals { scale: 2 })
        );
        // Zeros past the scale lose nothing.
        assert_eq!(parse_number("1,230", MONEY, &GERMAN), money(123));
        // A decimal without places holds whole numbers only.
        let whole = DataType::Decimal {
            precision: 8,
            scale: 0,
        };
        assert_eq!(
            parse_number("1,5", whole, &GERMAN),
            Err(NumberProblem::NotAnInteger)
        );
    }

    #[test]
    fn a_value_past_the_precision_is_refused() {
        let small = DataType::Decimal {
            precision: 4,
            scale: 2,
        };
        assert_eq!(
            parse_number("123,45", small, &GERMAN),
            Err(NumberProblem::TooLarge)
        );
        // Zeros past the scale lose nothing: it is the size that is wrong.
        assert_eq!(
            parse_number("123,450", small, &GERMAN),
            Err(NumberProblem::TooLarge)
        );
        assert_eq!(
            parse_number("99999999999999999999", DataType::Int64, &GERMAN),
            Err(NumberProblem::TooLarge)
        );
    }

    #[test]
    fn integers_and_floats_read_the_same_way() {
        assert_eq!(
            parse_number("1.234", DataType::Int64, &GERMAN),
            Ok(Value::Int64(1234))
        );
        assert_eq!(
            parse_number("-7", DataType::Int64, &ENGLISH),
            Ok(Value::Int64(-7))
        );
        assert_eq!(
            parse_number("1,5", DataType::Int64, &GERMAN),
            Err(NumberProblem::NotAnInteger)
        );
        assert_eq!(
            parse_number("1.5", DataType::Int64, &ENGLISH),
            Err(NumberProblem::NotAnInteger)
        );
        assert_eq!(
            parse_number("1.234,5", DataType::Float64, &GERMAN),
            Ok(Value::Float64(1234.5))
        );
        assert_eq!(
            parse_number("1,1,0", DataType::Float64, &GERMAN),
            Err(NumberProblem::NotANumber)
        );
    }

    #[test]
    fn a_number_is_written_with_the_decimal_mark_of_the_page() {
        assert_eq!(number_text("1.10", &GERMAN), "1,10");
        assert_eq!(number_text("-1234.5", &GERMAN), "-1234,5");
        assert_eq!(number_text("1.10", &ENGLISH), "1.10");
        assert_eq!(number_text("42", &GERMAN), "42");
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
