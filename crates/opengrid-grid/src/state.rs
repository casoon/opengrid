//! The portable grid state machine.
//!
//! [`GridState`] holds what the grid shows — schema, sort, filter, focus, the
//! virtual window and the currently loaded page of values — and every transition
//! returns the [`Patch`] list that turns the previous view into the new one
//! (plan/spezifikation/08-rendering.md §Change Detection).
//!
//! Design points:
//!
//! * **No DOM knowledge.** A patch names a logical [`CellRef`], a [`Window`] or a
//!   column index. Attribute names, recycled row slots and `aria-rowindex` are the
//!   renderer's concern (point 16/17), so this crate builds for `wasm32` and for
//!   the host alike.
//! * **Minimal and deterministic.** A transition with a value equal to the current
//!   state returns an empty list; a transition that changes one cell returns only
//!   that cell's patch. Patch order follows schema order, so tests and snapshots
//!   are stable.
//! * **The page is the window's data.** [`GridState::apply_result`] places the
//!   result's rows at the current window offset, so a cell can be read by logical
//!   row across a scroll. Changing the window alone emits only a [`Patch::Window`];
//!   the new values arrive with the next result.
//! * **Arrow-free input.** The result is the column-oriented [`QueryResult`] of
//!   decision E14; the grid never sees Arrow.
//! * **The status is state.** Loading, empty and error are not renderer
//!   accidents but a [`GridStatus`] the state machine owns, so the visible
//!   status line and its announcement have a single source (plan point 41).

use opengrid_datasource::QueryResult;
use opengrid_query::{FilterExpr, Sort, SortDirection};
use opengrid_types::{Field, FieldName, Schema, Value};

use crate::{CellRef, Patch, Window};

/// What the grid is currently showing (plan point 41).
///
/// The four states are mutually exclusive and cover every moment of a query's
/// life: one is running ([`Loading`](Self::Loading)), it answered with rows
/// ([`Ready`](Self::Ready)), it answered with none ([`Empty`](Self::Empty)) or
/// it failed ([`Error`](Self::Error)). The renderer turns the status into the
/// visible status line and its `aria-live` announcement
/// (plan/spezifikation/09-accessibility.md §Statusmeldungen); the state machine
/// only owns which one holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GridStatus {
    /// Rows are loaded and shown.
    Ready,
    /// A query is running. Whatever is rendered may be stale — the default,
    /// because a fresh grid has not been answered yet.
    #[default]
    Loading,
    /// The query succeeded and matched no row.
    Empty,
    /// The last query failed. The message is the one shown to the user, so the
    /// caller phrases it before it gets here — never a raw debug string.
    Error(String),
}

/// The grid state and its change detection.
#[derive(Clone, Debug, PartialEq)]
pub struct GridState {
    schema: Schema,
    total_count: u64,
    sort: Vec<Sort>,
    filter: Option<FilterExpr>,
    focus: Option<CellRef>,
    window: Window,
    /// Logical row the first row of `page` belongs to.
    page_offset: u64,
    /// Column-oriented values of the loaded page, in schema order.
    page: Vec<Vec<Value>>,
    /// What the grid announces: loading, ready, empty or an error.
    status: GridStatus,
    /// Selected **logical** rows, ascending and without duplicates (point 35).
    ///
    /// Logical, not DOM slots: a slot is recycled while scrolling, so a
    /// selection kept per slot would move to whatever row landed there.
    selection: Vec<u64>,
    /// Where a range selection starts. Set by every plain toggle, used by
    /// `Shift`.
    anchor: Option<u64>,
    /// A sort or a filter has dropped a selection and the next result has not
    /// said so yet.
    selection_dropped: bool,
    /// Whether **this** result is the one that announces it. Exactly one does:
    /// `apply_result` moves the flag here, the next result overwrites it.
    announce_selection_cleared: bool,
    /// A one-off sentence the renderer appends to the status line.
    ///
    /// Column operations (point 36) change something only the sighted see, and
    /// the query that follows would otherwise overwrite the announcement with
    /// its own result line before anyone heard it.
    notice: Option<String>,
    /// The same sentence, kept for the **one** result that follows.
    pending_notice: Option<String>,
    /// The cell currently being edited (plan point 37).
    editing: Option<CellRef>,
    /// Cells the reader changed since the last result, so the renderer can mark
    /// them. **Optimistic, not saved**: the component edits, the page persists.
    changed: Vec<CellRef>,
}

impl GridState {
    /// A state with an empty window and no data.
    ///
    /// The schema is known before the first result arrives (the grid draws its
    /// header from it); [`apply_result`](Self::apply_result) may replace it.
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            total_count: 0,
            sort: Vec::new(),
            filter: None,
            focus: None,
            window: Window::default(),
            page_offset: 0,
            page: Vec::new(),
            status: GridStatus::Loading,
            selection: Vec::new(),
            anchor: None,
            selection_dropped: false,
            announce_selection_cleared: false,
            notice: None,
            pending_notice: None,
            editing: None,
            changed: Vec::new(),
        }
    }

    /// The output columns, in schema order.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Rows that matched the filter, before paging (`aria-rowcount` source).
    pub fn total_count(&self) -> u64 {
        self.total_count
    }

    /// What the grid is currently showing (plan point 41).
    pub fn status(&self) -> &GridStatus {
        &self.status
    }

    /// Replaces the status. Emits a [`Patch::Status`] when it changed, so a
    /// repeated `Loading` (one scroll query after another) announces once.
    pub fn set_status(&mut self, status: GridStatus) -> Vec<Patch> {
        if self.status == status {
            return Vec::new();
        }
        self.status = status.clone();
        vec![Patch::Status(status)]
    }

    /// The current sort keys, in order.
    pub fn sort(&self) -> &[Sort] {
        &self.sort
    }

    /// The active filter, if any.
    pub fn filter(&self) -> Option<&FilterExpr> {
        self.filter.as_ref()
    }

    /// The selected logical rows, ascending.
    pub fn selection(&self) -> &[u64] {
        &self.selection
    }

    /// Whether a logical row is selected.
    pub fn is_selected(&self, row: u64) -> bool {
        self.selection.binary_search(&row).is_ok()
    }

    /// Toggles one row and makes it the anchor for a later range.
    pub fn toggle_selection(&mut self, row: u64) -> Vec<Patch> {
        match self.selection.binary_search(&row) {
            Ok(index) => {
                self.selection.remove(index);
            }
            Err(index) => self.selection.insert(index, row),
        }
        // The anchor follows the last plain toggle even when it deselected:
        // `Shift` afterwards should extend from where the user just was.
        self.anchor = Some(row);
        vec![Patch::Selection(self.selection.clone())]
    }

    /// Selects the range from the anchor to `row`, keeping what is already
    /// selected.
    ///
    /// Without an anchor this is an ordinary toggle — `Shift` before anything
    /// else has no range to extend.
    pub fn extend_selection(&mut self, row: u64) -> Vec<Patch> {
        let Some(anchor) = self.anchor else {
            return self.toggle_selection(row);
        };
        let (low, high) = if anchor <= row {
            (anchor, row)
        } else {
            (row, anchor)
        };
        let before = self.selection.clone();
        for value in low..=high {
            if let Err(index) = self.selection.binary_search(&value) {
                self.selection.insert(index, value);
            }
        }
        if self.selection == before {
            return Vec::new();
        }
        vec![Patch::Selection(self.selection.clone())]
    }

    /// Selects every matching row, loaded or not.
    ///
    /// The selection is logical, so it can name rows the window has never
    /// shown — which is the only honest meaning of "select all" in a grid that
    /// holds one page at a time.
    pub fn select_all(&mut self) -> Vec<Patch> {
        let all: Vec<u64> = (0..self.total_count).collect();
        if self.selection == all {
            return Vec::new();
        }
        self.selection = all;
        vec![Patch::Selection(self.selection.clone())]
    }

    /// Drops the selection because the rows behind it changed.
    ///
    /// **A selection names positions, not records.** The grid identifies a row
    /// by its number in the result; it has no key column, and the query model
    /// gives it none. Sorting therefore does not reorder the selection — it
    /// leaves it pointing at whoever now sits in those positions, which is a
    /// different set of records and no longer what anybody picked. Filtering is
    /// the same. So every operation that changes which rows exist, or in what
    /// order, clears it.
    ///
    /// Silently would be a trap, so the caller announces it (point 41's status
    /// line).
    ///
    /// Public since point 66: a facet restricts the rows on top of the filter,
    /// and the state does not see it — the element calls this for the same
    /// reason `set_filter` does.
    pub fn invalidate_selection(&mut self) -> Vec<Patch> {
        let patches = self.clear_selection();
        if !patches.is_empty() {
            self.selection_dropped = true;
        }
        patches
    }

    /// Records that a selection was dropped **before** this state existed, so
    /// the next result says so like any other drop.
    ///
    /// Applying a view (point 59) builds a fresh state: the old selection goes
    /// with the old state, and without this the drop would be silent — the
    /// trap [`invalidate_selection`](Self::invalidate_selection) exists to avoid.
    pub fn note_selection_dropped(&mut self) {
        self.selection_dropped = true;
    }

    /// The cell being edited, if any (plan point 37).
    pub fn editing(&self) -> Option<CellRef> {
        self.editing
    }

    /// Whether a cell holds a value the reader changed and nobody saved yet.
    pub fn is_changed(&self, cell: CellRef) -> bool {
        self.changed.contains(&cell)
    }

    /// Opens the editor on a cell, if the cell holds a loaded value.
    ///
    /// A cell outside the loaded page has nothing to edit — there is no value
    /// to start from, and inventing one would be a lie.
    pub fn begin_edit(&mut self, cell: CellRef) -> Vec<Patch> {
        if self.editing == Some(cell) || self.cell(cell).is_none() {
            return Vec::new();
        }
        self.editing = Some(cell);
        vec![Patch::Editing(Some(cell))]
    }

    /// Closes the editor without changing anything.
    pub fn end_edit(&mut self) -> Vec<Patch> {
        if self.editing.take().is_none() {
            return Vec::new();
        }
        vec![Patch::Editing(None)]
    }

    /// Writes a value into a loaded cell and marks it as changed.
    ///
    /// **Optimistic and honest about it:** the grid shows what the reader typed
    /// and marks it; whether anyone stored it is the page's business (E23).
    pub fn set_cell(&mut self, cell: CellRef, value: Value) -> Vec<Patch> {
        let Some(row) = cell.row.checked_sub(self.page_offset) else {
            return Vec::new();
        };
        let Some(column) = self.page.get_mut(cell.col) else {
            return Vec::new();
        };
        let Some(slot) = column.get_mut(row as usize) else {
            return Vec::new();
        };
        if *slot == value {
            return Vec::new();
        }
        *slot = value.clone();
        if !self.changed.contains(&cell) {
            self.changed.push(cell);
        }
        vec![Patch::Cell { cell, value }]
    }

    /// The one-off sentence the status line should carry, if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Says something once — now, and again with the result that follows.
    ///
    /// Two moments, because the announcement happens before the query and the
    /// query's own result line would otherwise wipe it out before it was read.
    pub fn set_notice(&mut self, text: String) -> Vec<Patch> {
        self.notice = Some(text.clone());
        self.pending_notice = Some(text);
        vec![Patch::Status(self.status.clone())]
    }

    /// Whether the status line should say that the selection is gone.
    ///
    /// True for exactly one result — the one that followed the sort or the
    /// filter that dropped it.
    pub fn announce_selection_cleared(&self) -> bool {
        self.announce_selection_cleared
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) -> Vec<Patch> {
        if self.selection.is_empty() {
            return Vec::new();
        }
        self.selection.clear();
        self.anchor = None;
        vec![Patch::Selection(Vec::new())]
    }

    /// The focused logical cell, if any.
    pub fn focus(&self) -> Option<CellRef> {
        self.focus
    }

    /// The virtual window.
    pub fn window(&self) -> Window {
        self.window
    }

    /// Number of rows currently loaded into the page.
    pub fn loaded_rows(&self) -> usize {
        self.page.first().map_or(0, Vec::len)
    }

    /// The value of a logical cell, if its row is in the loaded page.
    pub fn cell(&self, cell: CellRef) -> Option<&Value> {
        self.value_at(cell.col, cell.row)
    }

    /// Loads a page of rows for the current window.
    ///
    /// The result must belong to the current [`Window`]: its rows are placed at
    /// `window.offset`. Emits a [`Patch::Columns`] when the schema changed, a
    /// [`Patch::RowCount`] when `total_count` changed, a [`Patch::Status`] when
    /// the status changed and a [`Patch::Cell`] for every cell that differs from
    /// the previously loaded value — so re-applying the same result produces
    /// nothing. A result is by definition a success, so it always leaves the
    /// status at [`GridStatus::Ready`] or, with no matching row,
    /// [`GridStatus::Empty`].
    pub fn apply_result(&mut self, result: QueryResult) -> Vec<Patch> {
        let mut patches = Vec::new();
        // Exactly one result carries the "selection cleared" notice: the one
        // that arrived because of the sort or filter that dropped it.
        self.announce_selection_cleared = std::mem::take(&mut self.selection_dropped);
        self.notice = self.pending_notice.take();
        // A fresh result is the source's word on every value, so the reader's
        // unsaved marks no longer mean anything (point 37).
        self.changed.clear();
        self.editing = None;

        if result.schema != self.schema {
            patches.push(Patch::Columns(result.schema.clone()));
        }
        if result.total_count != self.total_count {
            patches.push(Patch::RowCount(result.total_count));
        }
        let status = if result.total_count == 0 {
            GridStatus::Empty
        } else {
            GridStatus::Ready
        };
        if self.status != status {
            patches.push(Patch::Status(status.clone()));
        }

        let first = self.window.offset;
        for (col, column) in result.columns.iter().enumerate() {
            for (slot, value) in column.iter().enumerate() {
                let row = first.saturating_add(slot as u64);
                if self.value_at(col, row) != Some(value) {
                    patches.push(Patch::Cell {
                        cell: CellRef::new(row, col),
                        value: value.clone(),
                    });
                }
            }
        }

        self.schema = result.schema;
        self.total_count = result.total_count;
        self.page_offset = first;
        self.page = result.columns;
        self.status = status;
        patches
    }

    /// Takes the typed schema of the columns without a result (plan point 88).
    ///
    /// A probe — the query with `limit 0` — tells the types before the first
    /// rows arrive; a filter written as text has to meet them before it is sent.
    /// Only the schema changes: rows, count and status wait for the result.
    pub fn adopt_schema(&mut self, schema: Schema) -> Vec<Patch> {
        if schema == self.schema {
            return Vec::new();
        }
        self.schema = schema.clone();
        vec![Patch::Columns(schema)]
    }

    /// Replaces the sort keys. Emits one [`Patch::Sort`] per column whose
    /// indicator changed, in schema order.
    pub fn set_sort(&mut self, sort: Vec<Sort>) -> Vec<Patch> {
        if self.sort == sort {
            return Vec::new();
        }
        let mut patches = Vec::new();
        for (column, field) in self.schema.fields().iter().enumerate() {
            let before = sort_key(&self.sort, field);
            let after = sort_key(&sort, field);
            if before != after {
                patches.push(Patch::Sort {
                    column,
                    sort: after.cloned(),
                });
            }
        }
        self.sort = sort;
        // A selection names positions; a different order puts different records
        // in them (see `invalidate_selection`).
        patches.extend(self.invalidate_selection());
        patches
    }

    /// The single active sort key as `(field, direction)`, the direction being
    /// the query wire token `"asc"`/`"desc"`.
    ///
    /// The grid mode of point 16 sorted by at most one column; this accessor
    /// answers only the first key. Point 18 uses [`sort_keys`](Self::sort_keys)
    /// for the whole (multi-column) sort of the query.
    pub fn single_sort(&self) -> Option<(&str, &'static str)> {
        self.sort.first().map(|key| {
            let direction = match key.direction {
                SortDirection::Asc => "asc",
                SortDirection::Desc => "desc",
            };
            (key.field.as_str(), direction)
        })
    }

    /// All sort keys as `(field, direction)` wire tokens, in order.
    ///
    /// The query carries the whole list (plan point 18: multi-sort); the order is
    /// the user's — the first key is the primary sort. Owned because the caller
    /// builds the query while the state stays borrowed.
    pub fn sort_keys(&self) -> Vec<(String, &'static str)> {
        self.sort
            .iter()
            .map(|key| {
                let direction = match key.direction {
                    SortDirection::Asc => "asc",
                    SortDirection::Desc => "desc",
                };
                (key.field.as_str().to_owned(), direction)
            })
            .collect()
    }

    /// Toggles the single-column sort of `column`: none → ascending → descending
    /// → none, replacing any other key. An unknown column name is a no-op.
    ///
    /// This is the state half of the header-cell activation of point 16; the
    /// caller re-runs its query with the new [`single_sort`](Self::single_sort).
    pub fn toggle_sort(&mut self, column: &str) -> Vec<Patch> {
        if self.schema.index_of(column).is_none() {
            return Vec::new();
        }
        let Ok(field) = FieldName::new(column) else {
            return Vec::new();
        };
        let next = match self.sort.first() {
            Some(key) if key.field == field && key.direction == SortDirection::Asc => {
                Some(SortDirection::Desc)
            }
            Some(key) if key.field == field && key.direction == SortDirection::Desc => None,
            _ => Some(SortDirection::Asc),
        };
        let sort = next
            .map(|direction| {
                vec![Sort {
                    field,
                    direction,
                    nulls: Default::default(),
                    collation: Default::default(),
                }]
            })
            .unwrap_or_default();
        self.set_sort(sort)
    }

    /// Toggles `column` as an **additional** sort key, preserving the order of
    /// the existing keys (plan point 18).
    ///
    /// * A column not in the sort is appended ascending (the primary key stays
    ///   first).
    /// * An appended ascending key becomes descending.
    /// * A descending key is removed.
    ///
    /// An unknown column name is a no-op. The caller falls back to
    /// [`ensure_sorted`](Self::ensure_sorted) after a removal so a total order
    /// remains (rule S6).
    pub fn toggle_sort_multi(&mut self, column: &str) -> Vec<Patch> {
        if self.schema.index_of(column).is_none() {
            return Vec::new();
        }
        let Ok(field) = FieldName::new(column) else {
            return Vec::new();
        };
        let mut sort = self.sort.clone();
        match sort.iter().position(|key| key.field == field) {
            None => sort.push(Sort {
                field,
                direction: SortDirection::Asc,
                nulls: Default::default(),
                collation: Default::default(),
            }),
            Some(index) => {
                if sort[index].direction == SortDirection::Asc {
                    sort[index].direction = SortDirection::Desc;
                } else {
                    sort.remove(index);
                }
            }
        }
        self.set_sort(sort)
    }

    /// Ensures at least one sort key, defaulting to the first schema field
    /// ascending. Emits the sort patch when it had to set one.
    ///
    /// Grid mode pages with `offset`/`limit`, and rule S6 makes `offset` without
    /// a `sort` a validation error, so the grid keeps a sort at all times: when
    /// the user clears the sort, the grid falls back to this default instead of
    /// querying with an empty `sort`.
    pub fn ensure_sorted(&mut self) -> Vec<Patch> {
        if !self.sort.is_empty() {
            return Vec::new();
        }
        let Some(field) = self.schema.fields().first().map(|field| field.name.clone()) else {
            return Vec::new();
        };
        self.set_sort(vec![Sort {
            field,
            direction: SortDirection::Asc,
            nulls: Default::default(),
            collation: Default::default(),
        }])
    }

    /// Replaces the filter. Emits the new filter, or `None` to clear it.
    ///
    /// Drops the selection with it — see [`invalidate_selection`](Self::invalidate_selection).
    pub fn set_filter(&mut self, filter: Option<FilterExpr>) -> Vec<Patch> {
        if self.filter == filter {
            return Vec::new();
        }
        self.filter = filter.clone();
        let mut patches = vec![Patch::Filter(filter)];
        patches.extend(self.invalidate_selection());
        patches
    }

    /// Moves focus. Emits the old and new cell; either may be `None`.
    pub fn set_focus(&mut self, focus: Option<CellRef>) -> Vec<Patch> {
        if self.focus == focus {
            return Vec::new();
        }
        let from = self.focus;
        self.focus = focus;
        vec![Patch::Focus { from, to: focus }]
    }

    /// Moves or resizes the virtual window. The loaded page is left untouched;
    /// point 17 maps the change onto recycled rows, the next result fills them.
    pub fn set_window(&mut self, window: Window) -> Vec<Patch> {
        if self.window == window {
            return Vec::new();
        }
        self.window = window;
        vec![Patch::Window(window)]
    }

    fn value_at(&self, col: usize, row: u64) -> Option<&Value> {
        let slot = row.checked_sub(self.page_offset)?;
        let slot = usize::try_from(slot).ok()?;
        self.page.get(col)?.get(slot)
    }
}

/// The sort key of a column, if the sort list names it.
fn sort_key<'a>(sort: &'a [Sort], field: &Field) -> Option<&'a Sort> {
    sort.iter().find(|key| key.field == field.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probe's schema is taken without touching rows or status (point 88).
    #[test]
    fn a_probe_schema_is_adopted_alone() {
        use opengrid_types::{DataType, Field, FieldName};
        let text = Schema::new(vec![Field::new(
            FieldName::new("qty").unwrap(),
            DataType::Utf8,
        )]);
        let typed = Schema::new(vec![Field::new(
            FieldName::new("qty").unwrap(),
            DataType::Int64,
        )]);
        let mut state = GridState::new(text);
        let status = state.status().clone();
        assert_eq!(
            state.adopt_schema(typed.clone()),
            vec![Patch::Columns(typed.clone())]
        );
        assert_eq!(state.schema(), &typed);
        assert_eq!(state.status(), &status);
        assert_eq!(state.total_count(), 0);
        assert!(
            state.adopt_schema(typed).is_empty(),
            "the same schema is no change"
        );
    }
    use opengrid_query::{Collation, NullsOrder, SortDirection};
    use opengrid_types::{DataType, FieldName};

    pub(super) fn schema() -> Schema {
        Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("amount").unwrap(), DataType::Int64),
        ])
    }

    /// A three-column result (`id`, `country`, `amount`) in schema order.
    pub(super) fn result(rows: &[(i64, &str, i64)], total_count: u64) -> QueryResult {
        let ids = rows.iter().map(|(id, _, _)| Value::Int64(*id)).collect();
        let countries = rows
            .iter()
            .map(|(_, country, _)| Value::Utf8((*country).to_owned()))
            .collect();
        let amounts = rows
            .iter()
            .map(|(_, _, amount)| Value::Int64(*amount))
            .collect();
        QueryResult::new(schema(), vec![ids, countries, amounts], total_count)
    }

    fn sort(field: &str, direction: SortDirection) -> Sort {
        Sort {
            field: FieldName::new(field).unwrap(),
            direction,
            nulls: NullsOrder::default(),
            collation: Collation::default(),
        }
    }

    #[test]
    fn identical_result_produces_no_patches() {
        let mut state = GridState::new(schema());
        let first = state.apply_result(result(&[(1, "DE", 10)], 1));
        assert!(!first.is_empty());

        assert_eq!(state.apply_result(result(&[(1, "DE", 10)], 1)), Vec::new());
    }

    #[test]
    fn single_cell_change_yields_only_that_patch() {
        let mut state = GridState::new(schema());
        state.set_window(Window::new(0, 3));
        state.apply_result(result(&[(1, "DE", 10), (2, "FR", 20), (3, "US", 30)], 3));

        let patches = state.apply_result(result(&[(1, "DE", 10), (2, "FR", 99), (3, "US", 30)], 3));

        assert_eq!(
            patches,
            vec![Patch::Cell {
                cell: CellRef::new(1, 2),
                value: Value::Int64(99),
            }]
        );
    }

    #[test]
    fn total_count_change_yields_only_row_count() {
        let mut state = GridState::new(schema());
        state.apply_result(result(&[(1, "DE", 10)], 1));

        assert_eq!(
            state.apply_result(result(&[(1, "DE", 10)], 7)),
            vec![Patch::RowCount(7)]
        );
    }

    #[test]
    fn column_change_yields_only_columns() {
        let mut state = GridState::new(schema());
        state.apply_result(result(&[(1, "DE", 10)], 1));

        let narrowed = Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
        ]);
        let narrowed_result = QueryResult::new(
            narrowed.clone(),
            vec![vec![Value::Int64(1)], vec![Value::Utf8("DE".to_owned())]],
            1,
        );

        assert_eq!(
            state.apply_result(narrowed_result),
            vec![Patch::Columns(narrowed)]
        );
    }

    #[test]
    fn result_after_a_window_move_fills_the_new_rows() {
        let mut state = GridState::new(schema());
        state.set_window(Window::new(0, 2));
        state.apply_result(result(&[(1, "DE", 10), (2, "FR", 20)], 4));

        assert_eq!(
            state.set_window(Window::new(2, 2)),
            vec![Patch::Window(Window::new(2, 2))]
        );
        let patches = state.apply_result(result(&[(3, "US", 30), (4, "ES", 40)], 4));

        // Rows 2 and 3 were not loaded before, so all six cells are new.
        assert_eq!(patches.len(), 6);
        assert!(patches.contains(&Patch::Cell {
            cell: CellRef::new(2, 0),
            value: Value::Int64(3),
        }));
        assert!(patches.contains(&Patch::Cell {
            cell: CellRef::new(3, 2),
            value: Value::Int64(40),
        }));
    }

    #[test]
    fn focus_transition_is_minimal() {
        let mut state = GridState::new(schema());
        let cell = CellRef::new(2, 1);
        let other = CellRef::new(5, 0);

        assert_eq!(
            state.set_focus(Some(cell)),
            vec![Patch::Focus {
                from: None,
                to: Some(cell),
            }]
        );
        assert_eq!(state.set_focus(Some(cell)), Vec::new());
        assert_eq!(
            state.set_focus(Some(other)),
            vec![Patch::Focus {
                from: Some(cell),
                to: Some(other),
            }]
        );
        assert_eq!(
            state.set_focus(None),
            vec![Patch::Focus {
                from: Some(other),
                to: None,
            }]
        );
        assert_eq!(state.set_focus(None), Vec::new());
    }

    #[test]
    fn sort_transition_reports_changed_columns() {
        let mut state = GridState::new(schema());
        let amount_desc = sort("amount", SortDirection::Desc);

        assert_eq!(
            state.set_sort(vec![amount_desc.clone()]),
            vec![Patch::Sort {
                column: 2,
                sort: Some(amount_desc.clone()),
            }]
        );
        assert_eq!(state.set_sort(vec![amount_desc.clone()]), Vec::new());

        let amount_asc = sort("amount", SortDirection::Asc);
        assert_eq!(
            state.set_sort(vec![amount_asc.clone()]),
            vec![Patch::Sort {
                column: 2,
                sort: Some(amount_asc),
            }]
        );
        assert_eq!(
            state.set_sort(Vec::new()),
            vec![Patch::Sort {
                column: 2,
                sort: None,
            }]
        );
    }

    #[test]
    fn single_sort_reads_the_first_key_as_a_wire_token() {
        let mut state = GridState::new(schema());
        assert_eq!(state.single_sort(), None);

        state.set_sort(vec![sort("country", SortDirection::Desc)]);
        assert_eq!(state.single_sort(), Some(("country", "desc")));

        state.set_sort(vec![sort("id", SortDirection::Asc)]);
        assert_eq!(state.single_sort(), Some(("id", "asc")));
    }

    #[test]
    fn toggle_sort_cycles_none_asc_desc_none() {
        let mut state = GridState::new(schema());

        state.toggle_sort("country");
        assert_eq!(state.single_sort(), Some(("country", "asc")));
        state.toggle_sort("country");
        assert_eq!(state.single_sort(), Some(("country", "desc")));
        state.toggle_sort("country");
        assert_eq!(state.single_sort(), None);

        // A different column starts a fresh ascending sort.
        state.toggle_sort("country");
        state.toggle_sort("amount");
        assert_eq!(state.single_sort(), Some(("amount", "asc")));

        // An unknown column is a no-op.
        assert_eq!(state.toggle_sort("missing"), Vec::new());
        assert_eq!(state.single_sort(), Some(("amount", "asc")));
    }

    /// Multi-sort appends additional keys, keeps their order and toggles a key
    /// asc → desc → removed.
    #[test]
    fn multi_sort_appends_and_preserves_order() {
        let mut state = GridState::new(schema());

        state.toggle_sort_multi("country");
        state.toggle_sort_multi("amount");
        assert_eq!(
            state.sort_keys(),
            [("country".to_owned(), "asc"), ("amount".to_owned(), "asc"),]
        );

        // The first key keeps its position while the second toggles.
        state.toggle_sort_multi("amount");
        assert_eq!(
            state.sort_keys(),
            [("country".to_owned(), "asc"), ("amount".to_owned(), "desc"),]
        );

        // The next activation removes it, leaving the primary key.
        state.toggle_sort_multi("amount");
        assert_eq!(state.sort_keys(), [("country".to_owned(), "asc")]);

        // An unknown column is a no-op.
        let before = state.sort_keys();
        assert_eq!(state.toggle_sort_multi("missing"), Vec::new());
        assert_eq!(state.sort_keys(), before);
    }

    /// The grid keeps a sort at all times (S6): clearing falls back to the first
    /// schema field ascending.
    #[test]
    fn ensure_sorted_defaults_to_the_first_field() {
        let mut state = GridState::new(schema());
        assert_eq!(state.single_sort(), None);

        assert_eq!(
            state.ensure_sorted(),
            vec![Patch::Sort {
                column: 0,
                sort: Some(sort("id", SortDirection::Asc)),
            }]
        );
        assert_eq!(state.single_sort(), Some(("id", "asc")));
        assert_eq!(state.ensure_sorted(), Vec::new());
    }

    #[test]
    fn sort_reports_every_changed_column_in_schema_order() {
        let mut state = GridState::new(schema());

        let patches = state.set_sort(vec![
            sort("country", SortDirection::Asc),
            sort("id", SortDirection::Desc),
        ]);

        assert_eq!(
            patches,
            vec![
                Patch::Sort {
                    column: 0,
                    sort: Some(sort("id", SortDirection::Desc)),
                },
                Patch::Sort {
                    column: 1,
                    sort: Some(sort("country", SortDirection::Asc)),
                },
            ]
        );
    }

    #[test]
    fn filter_transition_reports_the_new_filter() {
        let mut state = GridState::new(schema());
        let filter = FilterExpr::IsNull {
            field: FieldName::new("country").unwrap(),
        };

        assert_eq!(
            state.set_filter(Some(filter.clone())),
            vec![Patch::Filter(Some(filter.clone()))]
        );
        assert_eq!(state.set_filter(Some(filter)), Vec::new());
        assert_eq!(state.set_filter(None), vec![Patch::Filter(None)]);
        assert_eq!(state.set_filter(None), Vec::new());
    }

    #[test]
    fn window_transition_reports_the_window() {
        let mut state = GridState::new(schema());
        let window = Window::new(40, 20);

        assert_eq!(state.set_window(window), vec![Patch::Window(window)]);
        assert_eq!(state.set_window(window), Vec::new());
    }

    /// A fresh grid has not been answered yet, and the first result flips the
    /// status to `Ready` — once, not on every later result.
    #[test]
    fn the_first_result_reports_ready_once() {
        let mut state = GridState::new(schema());
        assert_eq!(state.status(), &GridStatus::Loading);

        let patches = state.apply_result(result(&[(1, "DE", 10)], 1));
        assert!(patches.contains(&Patch::Status(GridStatus::Ready)));
        assert_eq!(state.status(), &GridStatus::Ready);

        // A second, different result is still `Ready`: no status patch.
        let patches = state.apply_result(result(&[(1, "DE", 10)], 7));
        assert_eq!(patches, vec![Patch::RowCount(7)]);
    }

    /// A result with no matching row is `Empty`, not a silent zero-row `Ready`.
    #[test]
    fn a_result_without_rows_reports_empty() {
        let mut state = GridState::new(schema());
        state.apply_result(result(&[(1, "DE", 10)], 1));

        let patches = state.apply_result(result(&[], 0));
        assert!(patches.contains(&Patch::Status(GridStatus::Empty)));
        assert_eq!(state.status(), &GridStatus::Empty);

        // And back to `Ready` with the next non-empty result.
        let patches = state.apply_result(result(&[(1, "DE", 10)], 1));
        assert!(patches.contains(&Patch::Status(GridStatus::Ready)));
    }

    /// The error status is a transition like any other: minimal, and cleared by
    /// the next successful result.
    #[test]
    fn the_error_status_is_a_minimal_transition() {
        let mut state = GridState::new(schema());
        state.apply_result(result(&[(1, "DE", 10)], 1));

        let failed = GridStatus::Error("Die Daten konnten nicht geladen werden".to_owned());
        assert_eq!(
            state.set_status(failed.clone()),
            vec![Patch::Status(failed.clone())]
        );
        assert_eq!(state.set_status(failed), Vec::new());

        let patches = state.apply_result(result(&[(1, "DE", 10)], 1));
        assert_eq!(patches, vec![Patch::Status(GridStatus::Ready)]);
    }

    #[test]
    fn cells_are_read_by_logical_row() {
        let mut state = GridState::new(schema());
        state.set_window(Window::new(10, 2));
        state.apply_result(result(&[(10, "DE", 10), (11, "FR", 20)], 100));

        assert_eq!(state.total_count(), 100);
        assert_eq!(state.loaded_rows(), 2);
        assert_eq!(
            state.cell(CellRef::new(10, 1)),
            Some(&Value::Utf8("DE".to_owned()))
        );
        assert_eq!(state.cell(CellRef::new(11, 2)), Some(&Value::Int64(20)));
        // Rows outside the loaded page, and columns outside the schema.
        assert_eq!(state.cell(CellRef::new(9, 1)), None);
        assert_eq!(state.cell(CellRef::new(12, 0)), None);
        assert_eq!(state.cell(CellRef::new(10, 9)), None);
    }
}

#[cfg(test)]
mod selection_tests {
    use super::tests::{result, schema};
    use super::*;
    use opengrid_types::FieldName;

    fn grid() -> GridState {
        let mut state = GridState::new(schema());
        state.apply_result(result(&[(1, "DE", 10)], 100));
        state
    }

    /// A selection is a set of logical rows, and toggling it is symmetric.
    #[test]
    fn a_row_toggles_in_and_out() {
        let mut state = grid();
        assert_eq!(state.toggle_selection(3), vec![Patch::Selection(vec![3])]);
        assert!(state.is_selected(3));
        assert_eq!(state.toggle_selection(3), vec![Patch::Selection(vec![])]);
        assert!(!state.is_selected(3));
    }

    /// The rows come out ascending whatever order they went in — the renderer
    /// and the page both read this list, and neither should have to sort it.
    #[test]
    fn the_selection_is_ordered() {
        let mut state = grid();
        state.toggle_selection(7);
        state.toggle_selection(2);
        state.toggle_selection(5);
        assert_eq!(state.selection(), [2, 5, 7]);
    }

    /// `Shift` extends from the last plain toggle, in either direction, and
    /// keeps what was already there.
    #[test]
    fn a_range_extends_from_the_anchor() {
        let mut state = grid();
        state.toggle_selection(4);
        state.extend_selection(7);
        assert_eq!(state.selection(), [4, 5, 6, 7]);

        // Backwards from the same anchor, and the earlier rows stay.
        state.toggle_selection(20);
        state.extend_selection(18);
        assert_eq!(state.selection(), [4, 5, 6, 7, 18, 19, 20]);
    }

    /// `Shift` before anything else has no range to extend, so it is a toggle.
    #[test]
    fn a_range_without_an_anchor_is_a_toggle() {
        let mut state = grid();
        assert_eq!(state.extend_selection(9), vec![Patch::Selection(vec![9])]);
    }

    /// "Select all" means every matching row, not every loaded one — the grid
    /// holds one page, the selection is logical.
    #[test]
    fn select_all_reaches_rows_the_window_never_showed() {
        let mut state = grid();
        assert_eq!(state.loaded_rows(), 1);
        state.select_all();
        assert_eq!(state.selection().len(), 100);
        assert!(state.is_selected(99), "far outside the loaded page");
    }

    /// **The constraint that decides the behaviour.** A selection names
    /// positions, and sorting puts different records in them.
    #[test]
    fn sorting_and_filtering_drop_the_selection() {
        let mut state = grid();
        state.toggle_selection(3);
        let patches = state.toggle_sort("id");
        assert!(
            patches.contains(&Patch::Selection(Vec::new())),
            "sorting must say that the selection is gone: {patches:?}"
        );
        assert!(state.selection().is_empty());

        state.toggle_selection(3);
        let patches = state.set_filter(Some(FilterExpr::IsNull {
            field: FieldName::new("name").unwrap(),
        }));
        assert!(patches.contains(&Patch::Selection(Vec::new())));
        assert!(state.selection().is_empty());
    }

    /// Scrolling is not a change of the rows, so it leaves the selection alone.
    #[test]
    fn scrolling_keeps_the_selection() {
        let mut state = grid();
        state.toggle_selection(3);
        state.set_window(Window::new(40, 20));
        state.apply_result(result(&[(41, "FR", 20)], 100));
        assert_eq!(
            state.selection(),
            [3],
            "the row is off-screen, not unselected"
        );
    }

    /// Nothing to change means no patch — the same rule the rest of the state
    /// follows, so a repeated key does not announce twice.
    #[test]
    fn an_unchanged_selection_is_silent() {
        let mut state = grid();
        assert!(state.clear_selection().is_empty());
        state.select_all();
        assert!(state.select_all().is_empty());
    }
}

#[cfg(test)]
mod edit_tests {
    use super::tests::{result, schema};
    use super::*;

    fn grid() -> GridState {
        let mut state = GridState::new(schema());
        state.apply_result(result(&[(1, "DE", 10), (2, "FR", 20)], 2));
        state
    }

    /// The editor opens on a loaded cell and closes again.
    #[test]
    fn the_editor_opens_and_closes() {
        let mut state = grid();
        let cell = CellRef::new(0, 1);
        assert_eq!(state.begin_edit(cell), vec![Patch::Editing(Some(cell))]);
        assert_eq!(state.editing(), Some(cell));
        assert!(state.begin_edit(cell).is_empty(), "already open");
        assert_eq!(state.end_edit(), vec![Patch::Editing(None)]);
        assert!(state.end_edit().is_empty());
    }

    /// A cell outside the loaded page has no value to start from.
    #[test]
    fn a_cell_without_a_value_cannot_be_edited() {
        let mut state = grid();
        assert!(state.begin_edit(CellRef::new(500, 0)).is_empty());
        assert_eq!(state.editing(), None);
    }

    /// A committed value shows at once and is marked as unsaved.
    #[test]
    fn a_changed_cell_shows_and_is_marked() {
        let mut state = grid();
        let cell = CellRef::new(1, 1);
        let value = Value::Utf8("NL".to_owned());
        assert_eq!(
            state.set_cell(cell, value.clone()),
            vec![Patch::Cell {
                cell,
                value: value.clone()
            }]
        );
        assert_eq!(state.cell(cell), Some(&value));
        assert!(state.is_changed(cell));
        assert!(!state.is_changed(CellRef::new(0, 1)));
        // The same value again is not a change.
        assert!(state.set_cell(cell, value).is_empty());
    }

    /// A fresh result is the source's word: the marks and the editor go.
    #[test]
    fn a_new_result_clears_the_marks() {
        let mut state = grid();
        let cell = CellRef::new(0, 1);
        state.set_cell(cell, Value::Utf8("NL".to_owned()));
        state.begin_edit(cell);

        state.apply_result(result(&[(1, "DE", 10), (2, "FR", 20)], 2));
        assert!(!state.is_changed(cell), "the source has spoken");
        assert_eq!(state.editing(), None);
    }

    /// A selection dropped before this state existed (a view applied, point 59)
    /// is said by the next result — once — like one dropped by a sort.
    #[test]
    fn a_selection_dropped_elsewhere_is_said_once() {
        let mut state = grid();
        state.apply_result(result(&[(1, "DE", 10)], 1));
        assert!(!state.announce_selection_cleared(), "nothing was dropped");

        state.note_selection_dropped();
        state.apply_result(result(&[(1, "DE", 10)], 1));
        assert!(state.announce_selection_cleared());

        state.apply_result(result(&[(1, "DE", 10)], 1));
        assert!(
            !state.announce_selection_cleared(),
            "said once, not on every result"
        );
    }
}
