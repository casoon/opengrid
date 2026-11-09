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

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> FieldName {
        FieldName::new(text).unwrap()
    }

    #[test]
    fn a_state_token_reads_back_and_names_its_part() {
        for (token, state) in [
            ("saving", CellState::Saving),
            ("saved", CellState::Saved),
            ("error", CellState::Error),
            ("invalid", CellState::Invalid),
        ] {
            assert_eq!(CellState::parse(token), Some(state));
            assert_eq!(state.as_str(), token);
            // `::part()` takes no attribute selector, so the part carries the state.
            assert_eq!(state.part(), format!("cell cell-{token}"));
        }
        assert_eq!(CellState::parse("Saved"), None);
        assert_eq!(CellState::parse(""), None);
    }

    #[test]
    fn a_level_token_reads_back_and_nothing_else_does() {
        assert_eq!(Level::parse("warning"), Some(Level::Warning));
        assert_eq!(Level::parse("error"), Some(Level::Error));
        assert_eq!(Level::Warning.as_str(), "warning");
        assert_eq!(Level::Error.as_str(), "error");
        assert_eq!(Level::parse("info"), None);
        assert_eq!(Level::parse("Error"), None);
    }

    /// Everything is keyed by record and column: the same column of another
    /// record, or another column of the same record, is another cell.
    #[test]
    fn a_state_belongs_to_its_record_and_column_and_the_last_word_wins() {
        let mut saves = Saves::default();
        let key = Value::Int64(1);
        assert_eq!(saves.state(&key, &name("seats")), None);

        saves.set_state(key.clone(), name("seats"), CellState::Saving);
        assert_eq!(saves.state(&key, &name("seats")), Some(CellState::Saving));
        assert_eq!(saves.state(&key, &name("city")), None);
        assert_eq!(saves.state(&Value::Int64(2), &name("seats")), None);

        saves.set_state(key.clone(), name("seats"), CellState::Saved);
        assert_eq!(saves.state(&key, &name("seats")), Some(CellState::Saved));
        assert_eq!(
            saves.states.len(),
            1,
            "a second word replaces, it does not stack"
        );
    }

    #[test]
    fn a_message_is_replaced_and_forgotten_by_record_and_column() {
        let mut saves = Saves::default();
        let key = Value::Utf8("a".into());
        saves.set_message(key.clone(), name("seats"), Some("too many".into()));
        saves.set_message(key.clone(), name("city"), Some("unknown".into()));
        assert_eq!(saves.message(&key, &name("seats")), Some("too many"));

        saves.set_message(key.clone(), name("seats"), Some("too few".into()));
        assert_eq!(saves.message(&key, &name("seats")), Some("too few"));
        assert_eq!(saves.messages.len(), 2);

        saves.set_message(key.clone(), name("seats"), None);
        assert_eq!(saves.message(&key, &name("seats")), None);
        assert_eq!(saves.message(&key, &name("city")), Some("unknown"));
    }

    #[test]
    fn a_value_the_page_set_is_replaced_in_place() {
        let mut saves = Saves::default();
        let key = Value::Int64(7);
        saves.set_value(key.clone(), name("avg"), Value::Float64(1.5));
        saves.set_value(key.clone(), name("avg"), Value::Float64(2.5));
        saves.set_value(Value::Int64(8), name("avg"), Value::Float64(9.0));
        assert_eq!(saves.values.len(), 2);
        assert_eq!(saves.values[0].2, Value::Float64(2.5));
    }
}
