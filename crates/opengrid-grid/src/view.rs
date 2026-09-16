//! Logical coordinates of the grid viewport.
//!
//! These are deliberately **logical**, not DOM coordinates: a [`CellRef`] names a
//! row of the whole result (not of the loaded page) so focus survives scrolling,
//! and a [`Window`] names a slice of logical rows. Mapping them onto recycled DOM
//! rows and the 1-based, header-counting `aria-rowindex` is the renderer's job
//! (points 16/17, plan/spezifikation/09-accessibility.md §Virtualisierung).

/// A logical cell.
///
/// `row` counts the whole result before paging, `col` indexes the schema. The
/// coordinate stays stable while the window moves, which is what keeps focus and
/// selection valid across scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellRef {
    /// Logical row in the whole result (0-based, before `offset`/`limit`).
    pub row: u64,
    /// Column index into the schema.
    pub col: usize,
}

impl CellRef {
    /// A cell at `row`, `col`.
    pub const fn new(row: u64, col: usize) -> Self {
        Self { row, col }
    }
}

/// The virtual window: the slice of logical rows the grid renders.
///
/// Point 15 only tracks it as state; recycling DOM rows from it is point 17. The
/// window is not clamped to `total_count` — while a page is being reloaded the
/// two can disagree, and the state must not invent rows that do not exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Window {
    /// First logical row of the window.
    pub offset: u64,
    /// Number of DOM row slots.
    pub count: u64,
}

impl Window {
    /// A window starting at `offset` with `count` slots.
    pub const fn new(offset: u64, count: u64) -> Self {
        Self { offset, count }
    }

    /// The first logical row past the window (exclusive).
    pub const fn end(&self) -> u64 {
        self.offset.saturating_add(self.count)
    }

    /// True when `row` lies inside the window.
    pub const fn contains(&self, row: u64) -> bool {
        row >= self.offset && row < self.end()
    }
}
