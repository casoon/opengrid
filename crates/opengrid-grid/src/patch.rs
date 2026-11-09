//! The output of change detection: a pure-data patch list.
//!
//! A [`Patch`] describes **what changed**, never how to write it to the DOM. The
//! renderer of point 16 maps each variant onto attributes and text inside the
//! shadow root; because there are no DOM types here, the whole state machine runs
//! natively (plan/spezifikation/11-crates.md §Portabilität).
//!
//! Patches are grouped by the three concerns the rendering specification names
//! (plan/spezifikation/08-rendering.md §Change Detection): **cells** ([`Patch::Cell`]),
//! **attributes** ([`Patch::Columns`], [`Patch::Window`], [`Patch::Sort`],
//! [`Patch::Focus`], [`Patch::Filter`]) and **rows** ([`Patch::RowCount`],
//! [`Patch::Status`]).
//!
//! The list is minimal in the sense the state machine promises: a transition that
//! does not change the state yields no patch, and a transition that changes one
//! cell yields exactly that one cell patch.

use opengrid_query::{FilterExpr, Sort};
use opengrid_types::{Schema, Value};

use crate::{CellRef, GridStatus, Window};

/// One atomic change produced by a [`GridState`](crate::GridState) transition.
#[derive(Clone, Debug, PartialEq)]
pub enum Patch {
    /// The number of matching rows changed (before paging). Source for
    /// `aria-rowcount`, which additionally counts the header row.
    RowCount(u64),
    /// The visible columns changed — count, order, names or types. Rebuilds the
    /// header row and `aria-colcount`.
    Columns(Schema),
    /// A logical cell holds a different value now.
    Cell {
        /// The logical cell that changed.
        cell: CellRef,
        /// The value it now holds.
        value: Value,
    },
    /// The virtual window moved or resized. Point 17 turns this into recycled
    /// rows and updated `aria-rowindex` attributes.
    Window(Window),
    /// The sort indicator of one column changed; `None` clears it.
    Sort {
        /// Column index into the schema.
        column: usize,
        /// The column's new sort key, if any.
        sort: Option<Sort>,
    },
    /// Focus moved; either side may be `None`.
    Focus {
        /// The cell that lost focus, if any.
        from: Option<CellRef>,
        /// The cell that gained focus, if any.
        to: Option<CellRef>,
    },
    /// The active filter changed; `None` means no filter.
    Filter(Option<FilterExpr>),
    /// The grid's status changed (plan point 41). The renderer writes it into
    /// the visible, `aria-live` status line.
    Status(GridStatus),
    /// The editor opened on a cell or closed (plan point 37). `None` closes it.
    Editing(Option<CellRef>),
    /// The selection changed (plan point 35). Carries the rows that are
    /// selected **now**, ascending, as logical row numbers.
    ///
    /// The whole selection rather than a delta: it is what the renderer needs
    /// to set `aria-selected` on the rows it happens to be showing, and what
    /// the page gets in the event. A delta would make both of them keep their
    /// own copy of the truth.
    Selection(Vec<u64>),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patch is plain data: the renderer compares and clones it, and a
    /// transition that changed nothing yields an equal (empty) list.
    #[test]
    fn patches_compare_by_what_they_say() {
        let cell = |row, value: i64| Patch::Cell {
            cell: CellRef::new(row, 0),
            value: Value::Int64(value),
        };
        assert_eq!(cell(1, 5), cell(1, 5));
        assert_ne!(cell(1, 5), cell(1, 6));
        assert_ne!(cell(1, 5), cell(2, 5));
        assert_ne!(Patch::RowCount(3), Patch::RowCount(4));
        assert_ne!(Patch::RowCount(3), Patch::Selection(vec![3]));
    }

    #[test]
    fn a_selection_patch_carries_the_whole_selection_not_a_delta() {
        let before = Patch::Selection(vec![1, 2]);
        let after = Patch::Selection(vec![1, 2, 3]);
        assert_ne!(before, after);
        assert_eq!(after.clone(), after);
    }

    #[test]
    fn focus_and_editing_may_have_no_cell() {
        let focus = Patch::Focus {
            from: None,
            to: Some(CellRef::new(0, 0)),
        };
        assert_ne!(
            focus,
            Patch::Focus {
                from: Some(CellRef::new(0, 0)),
                to: None,
            }
        );
        assert_ne!(
            Patch::Editing(None),
            Patch::Editing(Some(CellRef::new(0, 0)))
        );
        assert_ne!(Patch::Filter(None), Patch::Window(Window::new(0, 1)));
    }
}
