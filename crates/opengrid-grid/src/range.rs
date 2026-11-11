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
}
