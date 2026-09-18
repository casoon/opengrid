//! `opengrid-grid` — the portable grid state machine and change detection.
//!
//! The crate holds what the grid shows (schema, sort, filter, focus, the virtual
//! window and the loaded page) and turns every transition into a minimal
//! [`Patch`] list — the data the renderer applies to the DOM
//! (plan/spezifikation/08-rendering.md §Change Detection). The rendering itself
//! arrives in point 16; this crate stays free of it.
//!
//! ```no_run
//! use opengrid_grid::{CellRef, GridState, Patch, Window};
//! # use opengrid_datasource::QueryResult;
//! # use opengrid_types::{DataType, Field, FieldName, Schema, Value};
//! # fn main() {
//! let schema = Schema::new(vec![Field::new(FieldName::new("country").unwrap(), DataType::Utf8)]);
//! let mut state = GridState::new(schema);
//! state.set_window(Window::new(0, 40));
//! let result = QueryResult::new(
//!     Schema::new(vec![Field::new(FieldName::new("country").unwrap(), DataType::Utf8)]),
//!     vec![vec![Value::Utf8("DE".to_owned())]],
//!     1,
//! );
//! let patches: Vec<Patch> = state.apply_result(result);
//! state.set_focus(Some(CellRef::new(0, 0)));
//! # let _ = patches;
//! # }
//! ```
//!
//! Design decisions:
//!
//! * **Portable, no DOM** (plan/spezifikation/11-crates.md §Portabilität): the
//!   crate depends on neither `web-sys` nor `js-sys`, so the state machine and its
//!   patch calculation are native unit tests and build for `wasm32`.
//! * **Arrow-free input** (decision E14): [`GridState::apply_result`] consumes the
//!   column-oriented [`QueryResult`](opengrid_datasource::QueryResult); Arrow never
//!   reaches the grid.
//! * **Minimal patches**: a no-op transition yields an empty list, a single cell
//!   change yields a single [`Patch::Cell`]. Patch order is deterministic (schema
//!   order), so snapshots stay stable.
//! * **Status is state** (point 41): loading, an empty result and a failed query
//!   are a [`GridStatus`] the state machine owns, not something the renderer
//!   invents, so the visible status line and its announcement agree by
//!   construction.
//! * **Logical coordinates**: [`CellRef`] rows count the whole result, not the
//!   loaded page, so focus and selection survive scrolling. The 1-based,
//!   header-counting `aria-rowindex` is derived by the renderer, not here.

mod patch;
mod state;
mod view;

pub use patch::Patch;
pub use state::{GridState, GridStatus};
pub use view::{CellRef, Window};
