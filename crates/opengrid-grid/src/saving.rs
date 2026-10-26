//! What a page that saves edits tells the grid (issue #153).
//!
//! The grid edits, the page persists (point 37). A page that saves every cell
//! on its own has two things to say back: what became of an edit — saving,
//! saved, failed — and values it computes itself, a running average say,
//! that are neither the source's nor the reader's.
//!
//! **Both name the record, not the position.** A row number holds only while
//! nothing sorts, filters or reloads; the page saves while the reader goes on
//! working. So everything here is keyed by the value of the `row-key` field
//! and finds its row in whatever page is loaded at the time — which is also
//! why it outlives a result: a sort does not change what became of an edit.

use opengrid_types::{FieldName, Value};

/// What became of an edit the page saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    Saving,
    Saved,
    Error,
    /// A finding the page computed elsewhere (issue #188): the value is not
    /// wrong for the grid, the page says it is wrong for the record.
    Invalid,
}

impl CellState {
    /// The wire token, which is also the `data-state` value.
    pub fn as_str(&self) -> &'static str {
        match self {
            CellState::Saving => "saving",
            CellState::Saved => "saved",
            CellState::Error => "error",
            CellState::Invalid => "invalid",
        }
    }

    /// The cell's `part` attribute in this state: `cell` and a name of its
    /// own, so a page reaches it with `::part(cell-error)` — `::part()` takes
    /// no attribute selector, so `data-state` alone is out of its reach.
    pub fn part(&self) -> &'static str {
        match self {
            CellState::Saving => "cell cell-saving",
            CellState::Saved => "cell cell-saved",
            CellState::Error => "cell cell-error",
            CellState::Invalid => "cell cell-invalid",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "saving" => CellState::Saving,
            "saved" => CellState::Saved,
            "error" => CellState::Error,
            "invalid" => CellState::Invalid,
            _ => return None,
        })
    }
}

/// How serious a finding about a value is (issue #188).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Warning,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Warning => "warning",
            Level::Error => "error",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "warning" => Level::Warning,
            "error" => Level::Error,
            _ => return None,
        })
    }
}

/// What a validator said about a value: how serious, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    pub message: String,
}

/// The page's word on cells, by record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saves {
    /// The field that names a record; nothing here works without it.
    pub(crate) key: Option<FieldName>,
    pub(crate) states: Vec<(Value, FieldName, CellState)>,
    /// Values the page set, shown in place of the source's.
    pub(crate) values: Vec<(Value, FieldName, Value)>,
    /// What the page said about a cell it called invalid (issue #188).
    pub(crate) messages: Vec<(Value, FieldName, String)>,
}

impl Saves {
    pub(crate) fn state(&self, key: &Value, column: &FieldName) -> Option<CellState> {
        self.states
            .iter()
            .find(|(k, c, _)| k == key && c == column)
            .map(|(_, _, state)| *state)
    }

    pub(crate) fn set_state(&mut self, key: Value, column: FieldName, state: CellState) {
        match self
            .states
            .iter_mut()
            .find(|(k, c, _)| *k == key && *c == column)
        {
            Some(entry) => entry.2 = state,
            None => self.states.push((key, column, state)),
        }
    }

    pub(crate) fn message(&self, key: &Value, column: &FieldName) -> Option<&str> {
        self.messages
            .iter()
            .find(|(k, c, _)| k == key && c == column)
            .map(|(_, _, message)| message.as_str())
    }

    /// Remembers the sentence of an `invalid` cell; `None` forgets it.
    pub(crate) fn set_message(&mut self, key: Value, column: FieldName, message: Option<String>) {
        self.messages
            .retain(|(k, c, _)| !(*k == key && *c == column));
        if let Some(message) = message {
            self.messages.push((key, column, message));
        }
    }

    pub(crate) fn set_value(&mut self, key: Value, column: FieldName, value: Value) {
        match self
            .values
            .iter_mut()
            .find(|(k, c, _)| *k == key && *c == column)
        {
            Some(entry) => entry.2 = value,
            None => self.values.push((key, column, value)),
        }
    }
}
