//! Rows the reader added in the grid (issue #183).
//!
//! The source knows nothing of them: they are drawn **after the last row of
//! the result**, the last of them being the empty *draft* the reader types
//! into. A fired draft becomes an added row under a provisional key, marked
//! unsaved until the page says what became of it. None of this reaches a
//! query — the window asks the source for its own rows, and these come after.

use opengrid_types::{Field, Schema, Value};

/// What became of an added row, as the page said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    /// Fired, and the page has not answered.
    Unsaved,
    Saving,
    Saved,
    Error,
}

impl RowState {
    /// The wire token, which is also the `data-added` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            RowState::Unsaved => "unsaved",
            RowState::Saving => "saving",
            RowState::Saved => "saved",
            RowState::Error => "error",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "saving" => RowState::Saving,
            "saved" => RowState::Saved,
            "error" => RowState::Error,
            _ => return None,
        })
    }
}

/// One row the reader added.
#[derive(Clone, Debug, PartialEq)]
pub struct AddedRow {
    /// What the page names it by until it has a key of its own, and after it
    /// has one: the key the page gave.
    pub key: Value,
    /// The provisional key the row was added under — what `set_row_state` names.
    pub provisional: String,
    pub values: Vec<Value>,
    pub state: RowState,
}

/// What a display row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// A row of the result.
    Source,
    /// A row the reader added, by its place among them.
    Added(usize),
    /// The empty row the reader types into.
    Draft,
}

/// The added rows and the draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddedRows {
    enabled: bool,
    draft: Vec<Value>,
    rows: Vec<AddedRow>,
    /// Columns a fired draft lacked a value for, until the reader types one.
    missing: Vec<usize>,
    counter: u64,
}

impl AddedRows {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Turns the draft row on or off. Off drops the draft but not the rows
    /// already added: the page may not have answered for them yet.
    pub fn set_enabled(&mut self, enabled: bool, width: usize) {
        self.enabled = enabled;
        if !enabled {
            self.draft.clear();
            self.missing.clear();
        } else {
            self.fit(width);
        }
    }

    /// The draft is as wide as the schema.
    pub fn fit(&mut self, width: usize) {
        if self.enabled && self.draft.len() != width {
            self.draft = vec![Value::Null; width];
        }
    }

    /// Rows after the result: the added ones and the draft.
    pub fn len(&self) -> u64 {
        self.rows.len() as u64 + u64::from(self.enabled)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn rows(&self) -> &[AddedRow] {
        &self.rows
    }

    /// What display row `row` is, given the result's row count.
    pub fn kind(&self, row: u64, source: u64) -> RowKind {
        match row.checked_sub(source) {
            None => RowKind::Source,
            Some(place) if (place as usize) < self.rows.len() => RowKind::Added(place as usize),
            Some(_) if self.enabled => RowKind::Draft,
            Some(_) => RowKind::Source,
        }
    }

    pub fn value(&self, kind: RowKind, col: usize) -> Option<&Value> {
        match kind {
            RowKind::Source => None,
            RowKind::Added(place) => self.rows.get(place)?.values.get(col),
            RowKind::Draft => self.draft.get(col),
        }
    }

    /// Writes a draft cell; `true` when the value changed. Typing into a
    /// missing cell takes its mark off.
    pub fn set_draft(&mut self, col: usize, value: Value) -> bool {
        let Some(slot) = self.draft.get_mut(col) else {
            return false;
        };
        self.missing.retain(|missing| *missing != col);
        if *slot == value {
            return false;
        }
        *slot = value;
        true
    }

    pub fn draft(&self) -> &[Value] {
        &self.draft
    }

    /// The columns the draft lacks a value for after a refused add.
    pub fn missing(&self) -> &[usize] {
        &self.missing
    }

    /// The required columns the draft holds nothing in.
    pub fn lacking(&self, schema: &Schema) -> Vec<usize> {
        schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(col, field)| is_lacking(field, self.draft.get(*col)))
            .map(|(col, _)| col)
            .collect()
    }

    /// Remembers which columns a refused add lacked.
    pub fn mark_missing(&mut self, columns: Vec<usize>) {
        self.missing = columns;
    }

    /// Fires the draft: it becomes an added row, unsaved, under a provisional
    /// key, and a fresh draft follows. The caller has checked [`lacking`](Self::lacking).
    pub fn fire(&mut self) -> Option<&AddedRow> {
        if !self.enabled {
            return None;
        }
        self.counter += 1;
        let provisional = format!("new-{}", self.counter);
        let width = self.draft.len();
        let values = std::mem::replace(&mut self.draft, vec![Value::Null; width]);
        self.missing.clear();
        self.rows.push(AddedRow {
            key: Value::Utf8(provisional.clone()),
            provisional,
            values,
            state: RowState::Unsaved,
        });
        self.rows.last()
    }

    /// Adds a row with these values, as a fired draft would be — a paste of
    /// more rows than the grid has (issue #184). `values` is in schema order.
    pub fn push(&mut self, mut values: Vec<Value>) -> Option<&AddedRow> {
        if !self.enabled {
            return None;
        }
        values.resize(self.draft.len(), Value::Null);
        self.counter += 1;
        let provisional = format!("new-{}", self.counter);
        self.rows.push(AddedRow {
            key: Value::Utf8(provisional.clone()),
            provisional,
            values,
            state: RowState::Unsaved,
        });
        self.rows.last()
    }

    /// What the page said became of an added row. Answers its place among
    /// the added rows.
    pub fn set_state(
        &mut self,
        provisional: &str,
        state: RowState,
        key: Option<Value>,
    ) -> Option<usize> {
        let place = self
            .rows
            .iter()
            .position(|row| row.provisional == provisional)?;
        let row = &mut self.rows[place];
        row.state = state;
        if let (RowState::Saved, Some(key)) = (state, key) {
            row.key = key;
        }
        Some(place)
    }

    /// The saved rows are the source's now; a fresh result has them.
    pub fn drop_saved(&mut self) {
        self.rows.retain(|row| row.state != RowState::Saved);
    }

    /// How many added rows wait for the page.
    pub fn unsaved(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row.state, RowState::Unsaved | RowState::Saving))
            .count()
    }
}

/// A required field without a value: NULL, or text with nothing in it.
fn is_lacking(field: &Field, value: Option<&Value>) -> bool {
    !field.nullable
        && match value {
            None | Some(Value::Null) => true,
            Some(Value::Utf8(text)) => text.is_empty(),
            Some(_) => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{DataType, FieldName};

    fn schema() -> Schema {
        Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("note").unwrap(), DataType::Utf8),
        ])
    }

    fn rows() -> AddedRows {
        let mut added = AddedRows::default();
        added.set_enabled(true, 2);
        added
    }

    #[test]
    fn the_draft_is_the_last_row_and_a_fired_one_is_added_before_it() {
        let mut added = rows();
        assert_eq!(added.len(), 1);
        assert_eq!(added.kind(5, 5), RowKind::Draft);
        assert_eq!(added.kind(4, 5), RowKind::Source);
        added.set_draft(0, Value::Int64(7));
        let fired = added.fire().unwrap().clone();
        assert_eq!(fired.values[0], Value::Int64(7));
        assert_eq!(fired.state, RowState::Unsaved);
        assert_eq!(added.len(), 2);
        assert_eq!(added.kind(5, 5), RowKind::Added(0));
        assert_eq!(added.kind(6, 5), RowKind::Draft);
        assert_eq!(added.draft(), [Value::Null, Value::Null]);
    }

    #[test]
    fn a_required_field_without_a_value_is_lacking() {
        let mut added = rows();
        assert_eq!(added.lacking(&schema()), vec![0]);
        added.set_draft(0, Value::Int64(1));
        assert!(added.lacking(&schema()).is_empty());
    }

    #[test]
    fn typing_into_a_missing_cell_takes_its_mark_off() {
        let mut added = rows();
        added.mark_missing(vec![0]);
        added.set_draft(0, Value::Int64(1));
        assert!(added.missing().is_empty());
    }

    #[test]
    fn the_page_answers_by_the_provisional_key() {
        let mut added = rows();
        added.set_draft(0, Value::Int64(1));
        let provisional = added.fire().unwrap().provisional.clone();
        assert_eq!(
            added.set_state(&provisional, RowState::Saved, Some(Value::Int64(42))),
            Some(0)
        );
        assert_eq!(added.rows()[0].key, Value::Int64(42));
        assert_eq!(added.unsaved(), 0);
        added.drop_saved();
        assert!(added.rows().is_empty());
        assert_eq!(added.set_state("nobody", RowState::Error, None), None);
    }
}
