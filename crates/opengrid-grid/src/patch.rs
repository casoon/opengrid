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
