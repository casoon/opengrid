//! A block of cells and its copy as text (issue #327).
//!
//! A range is the rectangle between an anchor and the focus; it is marked and
//! announced as a size, and `Ctrl`/`Cmd`+`C` puts it on the clipboard as a
//! spreadsheet reads it: tab-separated, a cell holding a tab, a line break or a
//! double quote in double quotes. The text a cell holds is its **value**, not
//! what the column's format shows, so it pastes back — here
//! ([`crate::paste::split_block`]) and into a spreadsheet — as it was.
//!
//! **The formula guard.** A spreadsheet runs a text cell starting with `=`, `+`,
//! `-`, `@`, a tab or a carriage return as a formula, and a copy is the same
//! hand-over as an export: such a text cell gets a leading `'`, as the export's
//! `protectFormulas` does. [`unguard`] takes it off again when the block is
//! pasted back into a text column, so the round trip keeps the data.

use opengrid_types::{Decimal, Value};

use crate::CellRef;

/// The rectangle between two cells, both inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRange {
    pub anchor: CellRef,
    pub focus: CellRef,
}

impl CellRange {
    pub fn new(anchor: CellRef, focus: CellRef) -> Self {
        Self { anchor, focus }
    }

    /// First and last row of the range.
    pub fn rows(&self) -> (u64, u64) {
        (
            self.anchor.row.min(self.focus.row),
            self.anchor.row.max(self.focus.row),
        )
    }

    /// First and last column of the range.
    pub fn cols(&self) -> (usize, usize) {
        (
            self.anchor.col.min(self.focus.col),
            self.anchor.col.max(self.focus.col),
        )
    }

    pub fn contains(&self, cell: CellRef) -> bool {
        let (first_row, last_row) = self.rows();
        let (first_col, last_col) = self.cols();
        (first_row..=last_row).contains(&cell.row) && (first_col..=last_col).contains(&cell.col)
    }

    /// How many rows and columns it spans.
    pub fn size(&self) -> (u64, u64) {
        let (first_row, last_row) = self.rows();
        let (first_col, last_col) = self.cols();
        (last_row - first_row + 1, (last_col - first_col) as u64 + 1)
    }

    /// A range of one cell is no range: it is the focus.
    pub fn is_single(&self) -> bool {
        self.anchor == self.focus
    }
}

fn is_formula(text: &str) -> bool {
    matches!(
        text.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    )
}

/// One cell as the clipboard holds it. `text_column` says it is a text cell —
/// the formula guard covers those only, as the export's does. `lone_column`
/// says the block has one column: with no tab in it a reader falls back to
/// commas ([`crate::paste::split_block`]), so a comma is quoted too.
pub fn cell_text(text: &str, text_column: bool, lone_column: bool) -> String {
    let guarded;
    let text = if text_column && is_formula(text) {
        guarded = format!("'{text}");
        guarded.as_str()
    } else {
        text
    };
    if text.contains(['\t', '\n', '\r', '"']) || (lone_column && text.contains(',')) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Rows of cells already in clipboard form, as one text: tabs between the
/// cells, a line break after each row.
pub fn block_text(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
    out
}

/// Takes the formula guard off a pasted text cell: `'=SUM(A1)` is the text
/// `=SUM(A1)` — what [`cell_text`] wrote for it. Any other text is unchanged.
pub fn unguard(text: &str) -> &str {
    match text.strip_prefix('\'') {
        Some(rest) if is_formula(rest) => rest,
        _ => text,
    }
}

/// What the footer says about a range's values (issue #525): how many cells
/// hold a value, and the sum and the average of the numeric ones.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeSummary {
    /// The cells that hold a value — not NULL — whatever their type.
    pub count: u64,
    /// The sum of the numeric cells, exact where their types allow; `None`
    /// when the range holds no number.
    pub sum: Option<Value>,
    /// Their average, to two places (more where the column's scale has them).
    pub average: Option<Value>,
    /// The column every numeric cell sits in, when they share one: the sum
    /// and the average then read in that column's format.
    pub column: Option<usize>,
}

/// The running sum: whole numbers stay whole and decimals exact, until a
/// float or an overflow makes the sum a float.
#[derive(Clone, Copy)]
enum Sum {
    Int(i128),
    Dec(i128, u8),
    Float(f64),
}

impl Sum {
    fn as_f64(self) -> f64 {
        match self {
            Sum::Int(value) => value as f64,
            Sum::Dec(value, scale) => value as f64 / 10f64.powi(i32::from(scale)),
            Sum::Float(value) => value,
        }
    }

    /// The sum at `scale` places, `None` on overflow.
    fn at_scale(self, scale: u8) -> Option<i128> {
        let factor = 10i128.checked_pow(u32::from(scale))?;
        match self {
            Sum::Int(value) => value.checked_mul(factor),
            Sum::Dec(value, own) => {
                let up = 10i128.checked_pow(u32::from(scale.checked_sub(own)?))?;
                value.checked_mul(up)
            }
            Sum::Float(_) => None,
        }
    }

    fn add(self, value: &Value) -> Sum {
        let added = match (self, value) {
            (Sum::Float(sum), _) => Some(Sum::Float(sum + number(value))),
            (Sum::Int(sum), Value::Int64(value)) => {
                sum.checked_add(i128::from(*value)).map(Sum::Int)
            }
            (_, Value::Float64(value)) => Some(Sum::Float(self.as_f64() + value)),
            (Sum::Dec(_, own), Value::Int64(value)) => self
                .at_scale(own)
                .and_then(|sum| {
                    sum.checked_add(i128::from(*value).checked_mul(10i128.pow(u32::from(own)))?)
                })
                .map(|sum| Sum::Dec(sum, own)),
            (_, Value::Decimal(decimal)) => {
                let scale = match self {
                    Sum::Dec(_, own) => own.max(decimal.scale()),
                    _ => decimal.scale(),
                };
                let theirs = Sum::Dec(decimal.value(), decimal.scale()).at_scale(scale);
                self.at_scale(scale)
                    .zip(theirs)
                    .and_then(|(sum, theirs)| sum.checked_add(theirs))
                    .map(|sum| Sum::Dec(sum, scale))
            }
            _ => Some(self),
        };
        added.unwrap_or_else(|| Sum::Float(self.as_f64() + number(value)))
    }
}

/// A numeric value as a float; 0 for anything else.
fn number(value: &Value) -> f64 {
    match value {
        Value::Int64(value) => *value as f64,
        Value::Float64(value) => *value,
        Value::Decimal(decimal) => Sum::Dec(decimal.value(), decimal.scale()).as_f64(),
        _ => 0.0,
    }
}

/// `numerator / denominator`, rounded half away from zero.
fn rounded_div(numerator: i128, denominator: i128) -> i128 {
    let quotient = numerator / denominator;
    let rest = numerator % denominator;
    if rest.abs() * 2 >= denominator.abs() {
        quotient + numerator.signum() * denominator.signum()
    } else {
        quotient
    }
}

/// Sums up the cells of a range, each given with its column (issue #525).
///
/// `Int64`, `Float64` and `Decimal` are numbers; every other value counts
/// but does not add. A whole-number sum stays whole and a decimal one exact;
/// the average has two places, or the decimal column's scale where it is more.
pub fn summarize<'a>(cells: impl IntoIterator<Item = (usize, &'a Value)>) -> RangeSummary {
    let mut count = 0;
    let mut numbers: i128 = 0;
    let mut sum = Sum::Int(0);
    // `None`: no number yet; `Some(None)`: numbers in more than one column.
    let mut column: Option<Option<usize>> = None;
    for (col, value) in cells {
        if matches!(value, Value::Null) {
            continue;
        }
        count += 1;
        if matches!(
            value,
            Value::Int64(_) | Value::Float64(_) | Value::Decimal(_)
        ) {
            numbers += 1;
            sum = sum.add(value);
            column = match column {
                None => Some(Some(col)),
                Some(Some(shared)) if shared == col => Some(Some(col)),
                _ => Some(None),
            };
        }
    }
    if numbers == 0 {
        return RangeSummary {
            count,
            sum: None,
            average: None,
            column: None,
        };
    }
    let (total, average) = match sum {
        Sum::Int(value) => (
            i64::try_from(value).map_or(Value::Float64(value as f64), Value::Int64),
            value
                .checked_mul(100)
                .map_or(Value::Float64(value as f64 / numbers as f64), |hundred| {
                    Value::Decimal(Decimal::new(rounded_div(hundred, numbers), 2))
                }),
        ),
        Sum::Dec(value, scale) => {
            let places = scale.max(2);
            let average = sum
                .at_scale(places)
                .map_or(Value::Float64(sum.as_f64() / numbers as f64), |scaled| {
                    Value::Decimal(Decimal::new(rounded_div(scaled, numbers), places))
                });
            (Value::Decimal(Decimal::new(value, scale)), average)
        }
        Sum::Float(value) => (
            Value::Float64(value),
            Value::Float64((value / numbers as f64 * 100.0).round() / 100.0),
        ),
    };
    RangeSummary {
        count,
        sum: Some(total),
        average: Some(average),
        column: column.flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paste::split_block;

    fn range(a: (u64, usize), b: (u64, usize)) -> CellRange {
        CellRange::new(CellRef::new(a.0, a.1), CellRef::new(b.0, b.1))
    }

    #[test]
    fn the_rectangle_runs_in_either_direction() {
        let up_left = range((4, 3), (2, 1));
        assert_eq!(up_left.rows(), (2, 4));
        assert_eq!(up_left.cols(), (1, 3));
        assert_eq!(up_left.size(), (3, 3));
        assert!(up_left.contains(CellRef::new(3, 2)));
        assert!(!up_left.contains(CellRef::new(5, 2)));
        assert!(!up_left.contains(CellRef::new(3, 4)));
        assert!(range((1, 1), (1, 1)).is_single());
        assert!(!range((1, 1), (1, 2)).is_single());
    }

    #[test]
    fn a_cell_with_a_tab_a_break_or_a_quote_is_quoted() {
        assert_eq!(cell_text("plain", true, false), "plain");
        assert_eq!(cell_text("a\tb", true, false), "\"a\tb\"");
        assert_eq!(cell_text("a\nb", true, false), "\"a\nb\"");
        assert_eq!(cell_text("say \"hi\"", true, false), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn a_text_cell_that_would_run_as_a_formula_is_guarded_and_a_number_is_not() {
        assert_eq!(cell_text("=1+1", true, false), "'=1+1");
        assert_eq!(cell_text("@sum", true, false), "'@sum");
        assert_eq!(cell_text("-5", false, false), "-5");
        assert_eq!(cell_text("-5", true, false), "'-5");
    }

    #[test]
    fn what_is_copied_is_pasted_back_unchanged() {
        let cells = [
            "plain",
            "a\tb",
            "two\nlines",
            "say \"hi\"",
            "\"starts with a quote",
            "=HYPERLINK(\"x\")",
            "'real quote",
            "",
        ];
        let rows: Vec<Vec<String>> = cells
            .chunks(2)
            .map(|pair| {
                pair.iter()
                    .map(|text| cell_text(text, true, false))
                    .collect()
            })
            .collect();
        let back = split_block(&block_text(&rows));
        let flat: Vec<String> = back
            .iter()
            .flatten()
            .map(|text| unguard(text).to_owned())
            .collect();
        assert_eq!(flat, cells);
    }

    #[test]
    fn one_column_survives_a_comma() {
        let rows: Vec<Vec<String>> = ["a,b", "c"]
            .iter()
            .map(|text| vec![cell_text(text, true, true)])
            .collect();
        assert_eq!(
            split_block(&block_text(&rows)),
            vec![vec!["a,b"], vec!["c"]]
        );
    }

    #[test]
    fn only_a_guard_is_taken_off() {
        assert_eq!(unguard("'=SUM(A1)"), "=SUM(A1)");
        assert_eq!(unguard("'quoted"), "'quoted");
        assert_eq!(unguard("=SUM(A1)"), "=SUM(A1)");
    }
    fn int(value: i64) -> Value {
        Value::Int64(value)
    }

    #[test]
    fn a_summary_counts_values_and_sums_the_numbers() {
        let cells = [
            (0, Value::Utf8("Room 1".to_owned())),
            (1, int(4)),
            (0, Value::Null),
            (1, int(3)),
        ];
        let summary = summarize(cells.iter().map(|(col, value)| (*col, value)));
        assert_eq!(summary.count, 3, "NULL is no value; a text is one");
        assert_eq!(summary.sum, Some(int(7)), "whole numbers stay whole");
        assert_eq!(summary.average, Some(Value::Decimal(Decimal::new(350, 2))));
        assert_eq!(summary.column, Some(1), "every number sits in column 1");
    }

    #[test]
    fn a_summary_without_numbers_has_no_sum() {
        let cells = [(0, Value::Utf8("a".to_owned())), (0, Value::Bool(true))];
        let summary = summarize(cells.iter().map(|(col, value)| (*col, value)));
        assert_eq!(summary.count, 2);
        assert_eq!(
            (summary.sum, summary.average, summary.column),
            (None, None, None)
        );
    }

    #[test]
    fn decimals_sum_exactly_at_the_widest_scale() {
        let cells = [
            (2, Value::Decimal(Decimal::new(1050, 2))),
            (2, Value::Decimal(Decimal::new(1, 3))),
            (3, int(2)),
        ];
        let summary = summarize(cells.iter().map(|(col, value)| (*col, value)));
        assert_eq!(
            summary.sum,
            Some(Value::Decimal(Decimal::new(12501, 3))),
            "10.5 + 0.001 + 2"
        );
        assert_eq!(
            summary.average,
            Some(Value::Decimal(Decimal::new(4167, 3))),
            "12.501 / 3, rounded"
        );
        assert_eq!(summary.column, None, "numbers in two columns read plain");
    }

    #[test]
    fn a_float_makes_the_sum_a_float_and_the_average_two_places() {
        let cells = [(0, int(1)), (0, Value::Float64(0.5)), (0, int(1))];
        let summary = summarize(cells.iter().map(|(col, value)| (*col, value)));
        assert_eq!(summary.sum, Some(Value::Float64(2.5)));
        assert_eq!(summary.average, Some(Value::Float64(0.83)));
    }

    #[test]
    fn an_average_rounds_half_away_from_zero() {
        let cells = [
            (0, int(-1)),
            (0, int(-2)),
            (0, int(0)),
            (0, int(0)),
            (0, int(0)),
            (0, int(0)),
            (0, int(0)),
            (0, int(0)),
        ];
        // -3 / 8 = -0.375 → -0.38
        let summary = summarize(cells.iter().map(|(col, value)| (*col, value)));
        assert_eq!(summary.average, Some(Value::Decimal(Decimal::new(-38, 2))));
    }

    #[test]
    fn an_overflowing_sum_becomes_a_float() {
        let cells = [(0, int(i64::MAX)), (0, int(i64::MAX))];
        let summary = summarize(cells.iter().map(|(col, value)| (*col, value)));
        assert_eq!(summary.sum, Some(Value::Float64(i64::MAX as f64 * 2.0)));
    }
}
