//! The edit history of one grid (issue #186).
//!
//! A step is what the reader did: one cell edit, one added row, one paste.
//! Undoing walks them back, redoing forward; a new edit ends the way forward.
//! The history is the grid's own — the page's saves are not in it — and it
//! ends with the source it was made on.

use opengrid_types::Value;

/// One cell's change.
#[derive(Clone, Debug, PartialEq)]
pub struct CellEdit {
    /// Where the cell was, and the record's key where there is one.
    pub row: u64,
    pub col: usize,
    pub key: Option<Value>,
    pub before: Value,
    pub after: Value,
}

impl CellEdit {
    /// Whether the page'"'"'s word for this record'"'"'s cell overrides the edit.
    fn is_stale(&self, key: &Value, col: usize, now: &Value) -> bool {
        self.col == col && self.key.as_ref() == Some(key) && self.after != *now
    }
}

/// One row the reader added, in schema order, under the key it was added with.
#[derive(Clone, Debug, PartialEq)]
pub struct AddedEntry {
    pub provisional: String,
    pub values: Vec<Value>,
}

/// A step of the history.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Cell(CellEdit),
    Added(AddedEntry),
    /// A paste is **one** step: its cells and its rows go back together.
    Paste {
        edits: Vec<CellEdit>,
        added: Vec<AddedEntry>,
    },
}

/// The most steps kept; the oldest go first.
const LIMIT: usize = 200;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    /// Records what the reader just did. What was undone before it is gone.
    pub fn record(&mut self, step: Step) {
        self.redo.clear();
        self.undo.push(step);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    /// The step to undo, taken off; [`done`](Self::done_undo) puts it on the other side.
    pub fn take_undo(&mut self) -> Option<Step> {
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<Step> {
        self.redo.pop()
    }

    /// A step was undone: it can be redone.
    pub fn done_undo(&mut self, step: Step) {
        self.redo.push(step);
    }

    /// A step was redone: it can be undone again.
    pub fn done_redo(&mut self, step: Step) {
        self.undo.push(step);
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// The page said what a record's cell holds now (`set_values`): a step that
    /// would put another value there is no longer the grid's to take back or
    /// give again, so it goes. A step whose `after` is the value the page just
    /// confirmed stays — nothing changed for it. A paste keeps its other cells.
    pub fn forget_cell(&mut self, key: &Value, col: usize, now: &Value) {
        for steps in [&mut self.undo, &mut self.redo] {
            steps.retain_mut(|step| match step {
                Step::Cell(edit) => !edit.is_stale(key, col, now),
                Step::Paste { edits, added } => {
                    edits.retain(|edit| !edit.is_stale(key, col, now));
                    !edits.is_empty() || !added.is_empty()
                }
                Step::Added(_) => true,
            });
        }
    }

    /// The shown columns changed (see `AddedRows::remap`): a cell edit follows
    /// its column to the new place, and one in a column that is no longer shown
    /// goes — nobody can see what undoing it would do.
    pub fn remap(&mut self, from: &[Option<usize>]) {
        let place = |col: usize| from.iter().position(|old| *old == Some(col));
        let pick = |values: &[Value]| -> Vec<Value> {
            from.iter()
                .map(|old| {
                    old.and_then(|old| values.get(old))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect()
        };
        let edit = |edit: &mut CellEdit| match place(edit.col) {
            Some(col) => {
                edit.col = col;
                true
            }
            None => false,
        };
        for steps in [&mut self.undo, &mut self.redo] {
            steps.retain_mut(|step| match step {
                Step::Cell(cell) => edit(cell),
                Step::Added(entry) => {
                    entry.values = pick(&entry.values);
                    true
                }
                Step::Paste { edits, added } => {
                    edits.retain_mut(edit);
                    for entry in added.iter_mut() {
                        entry.values = pick(&entry.values);
                    }
                    !edits.is_empty() || !added.is_empty()
                }
            });
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(row: u64, before: i64, after: i64) -> Step {
        Step::Cell(CellEdit {
            row,
            col: 0,
            key: None,
            before: Value::Int64(before),
            after: Value::Int64(after),
        })
    }

    /// Issue #225: the page'"'"'s `set_values` overrides an edit; the grid must not
    /// take the old value back over it. A confirmation of the same value, or
    /// another cell, leaves the history alone.
    #[test]
    fn a_value_the_page_sets_ends_the_steps_it_overrides() {
        let keyed = |key: i64, col: usize, before: i64, after: i64| CellEdit {
            row: 0,
            col,
            key: Some(Value::Int64(key)),
            before: Value::Int64(before),
            after: Value::Int64(after),
        };
        let mut history = History::default();
        history.record(Step::Cell(keyed(1, 0, 10, 20)));
        history.record(Step::Cell(keyed(2, 0, 10, 20)));
        history.record(Step::Cell(keyed(1, 1, 10, 20)));
        history.record(Step::Paste {
            edits: vec![keyed(1, 0, 30, 40), keyed(3, 0, 30, 40)],
            added: Vec::new(),
        });

        // The page says 40 for record 1, column 0. The edit that made it 20
        // is overridden and goes; the paste, which made it 40, stays whole.
        history.forget_cell(&Value::Int64(1), 0, &Value::Int64(40));
        let mut left = Vec::new();
        while let Some(step) = history.take_undo() {
            left.push(step);
        }
        assert_eq!(left.len(), 3);

        // A value that differs from the paste'"'"'s `after` takes that cell out of
        // the paste and keeps the other one.
        let mut history = History::default();
        history.record(Step::Paste {
            edits: vec![keyed(1, 0, 30, 40), keyed(3, 0, 30, 40)],
            added: Vec::new(),
        });
        history.forget_cell(&Value::Int64(1), 0, &Value::Int64(99));
        let Some(Step::Paste { edits, .. }) = history.take_undo() else {
            panic!("the paste stays, with one cell less");
        };
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].key, Some(Value::Int64(3)));
    }

    #[test]
    fn steps_walk_back_and_forward() {
        let mut history = History::default();
        history.record(edit(0, 1, 2));
        history.record(edit(1, 3, 4));

        let second = history.take_undo().unwrap();
        assert_eq!(second, edit(1, 3, 4));
        history.done_undo(second);
        let first = history.take_undo().unwrap();
        assert_eq!(first, edit(0, 1, 2));
        history.done_undo(first);
        assert!(!history.can_undo());

        let again = history.take_redo().unwrap();
        assert_eq!(again, edit(0, 1, 2));
        history.done_redo(again);
        assert!(history.can_undo() && history.can_redo());
    }

    #[test]
    fn a_new_edit_ends_the_way_forward() {
        let mut history = History::default();
        history.record(edit(0, 1, 2));
        let step = history.take_undo().unwrap();
        history.done_undo(step);
        assert!(history.can_redo());
        history.record(edit(5, 7, 8));
        assert!(!history.can_redo());
    }

    #[test]
    fn the_oldest_steps_go_first_and_clear_ends_it() {
        let mut history = History::default();
        for n in 0..(LIMIT as i64 + 5) {
            history.record(edit(0, n, n + 1));
        }
        let mut count = 0;
        while history.take_undo().is_some() {
            count += 1;
        }
        assert_eq!(count, LIMIT);
        history.record(edit(0, 1, 2));
        history.clear();
        assert!(!history.can_undo() && !history.can_redo());
    }
}
