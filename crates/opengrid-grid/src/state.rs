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

use opengrid_datasource::QueryResult;
use opengrid_query::{FilterExpr, Sort, SortDirection};
use opengrid_types::{Field, FieldName, Schema, Value};

use crate::{CellRef, Patch, Window};

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

    /// The current sort keys, in order.
    pub fn sort(&self) -> &[Sort] {
        &self.sort
    }

    /// The active filter, if any.
    pub fn filter(&self) -> Option<&FilterExpr> {
        self.filter.as_ref()
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
    /// [`Patch::RowCount`] when `total_count` changed and a [`Patch::Cell`] for
    /// every cell that differs from the previously loaded value — so re-applying
    /// the same result produces nothing.
    pub fn apply_result(&mut self, result: QueryResult) -> Vec<Patch> {
        let mut patches = Vec::new();

        if result.schema != self.schema {
            patches.push(Patch::Columns(result.schema.clone()));
        }
        if result.total_count != self.total_count {
            patches.push(Patch::RowCount(result.total_count));
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
        patches
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
    pub fn set_filter(&mut self, filter: Option<FilterExpr>) -> Vec<Patch> {
        if self.filter == filter {
            return Vec::new();
        }
        self.filter = filter.clone();
        vec![Patch::Filter(filter)]
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
    use opengrid_query::{Collation, NullsOrder, SortDirection};
    use opengrid_types::{DataType, FieldName};

    fn schema() -> Schema {
        Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("amount").unwrap(), DataType::Int64),
        ])
    }

    /// A three-column result (`id`, `country`, `amount`) in schema order.
    fn result(rows: &[(i64, &str, i64)], total_count: u64) -> QueryResult {
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
