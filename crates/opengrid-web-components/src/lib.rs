//! `opengrid-web-components` — the custom elements `<opengrid-table>` and
//! `<opengrid-grid>` (points 13, 14, 16, 17).
//!
//! Point 13 registers the first element, `<opengrid-table>`, and renders the
//! empty native-`<table>` skeleton into an open shadow root (E8). Point 14 fills
//! it with columns and rows over the provider seam, sorting through the query
//! AST. Point 16 adds `<opengrid-grid>`: a `<table role="grid">` driven by the
//! portable [`opengrid_grid::GridState`], with roving tabindex and the WAI-ARIA
//! grid keyboard matrix. Point 17 virtualizes it: a fixed pool of recycled DOM
//! rows over a scrollable `<tbody>` sizer, range fetching over the provider and
//! focus pinning. On the host only the patch computation compiles and is
//! unit-tested; the DOM glue and the registration are `wasm32`-only, so
//! `cargo test` stays green without a browser and `just wasm-test` is the gate
//! that runs it (plan/spezifikation/11-crates.md §Portabilität).
//!
//! Point 32 adds `<opengrid-pivot>`: a **native** `<table>` with a two-level
//! column header, row headers and marked subtotals — Table Mode, not a virtual
//! grid, because an accessible virtual pivot is the highest risk in the project
//! (06-pivot.md, R5) and the limits of point 30 are what make rendering the whole
//! thing safe. It lives here rather than in its own `opengrid-pivot-grid` crate:
//! that split exists for bundle size, which point 40 decides after measuring,
//! and doing it now would duplicate the registration, the provider seam and the
//! text registry for no measured gain.
//!
//! Point 48 gives both elements one language: everything they write themselves
//! is English and lives in [`texts`], and a page overrides any of it — with the
//! language it is in — through the exported `set_texts`.

// **On the host this crate is only its tests.** Every entry point — the
// registered elements, the exported functions, the DOM glue — is `wasm32`-only,
// so the plain library target that `just check` also builds reaches none of the
// code below. Point 39 made that visible by closing the module surface; the
// alternative was to leave the modules `pub` and call unreachable code an API,
// which is the promise this point exists to stop making.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

// Nothing here is public API: the surface of this crate is the **DOM** — the
// elements, their attributes, their events and their parts — plus the four
// exported functions below. A module left `pub` would be a promise nobody meant
// to make (plan point 39).
#[cfg(test)]
mod api;
#[cfg(feature = "grid")]
pub(crate) mod column_menu;
#[cfg(feature = "grid")]
pub(crate) mod columns;
#[cfg(feature = "grid")]
pub(crate) mod facets;
#[cfg(feature = "grid")]
pub(crate) mod formats;
#[cfg(feature = "grid")]
pub(crate) mod grid;
#[cfg(feature = "grid")]
pub(crate) mod grouping;
#[cfg(feature = "pivot")]
pub(crate) mod pivot;
#[cfg(feature = "grid")]
pub(crate) mod presentation;
#[cfg(feature = "grid")]
pub(crate) mod search;
pub(crate) mod shared;
pub(crate) mod table;
pub(crate) mod texts;
#[cfg(feature = "grid")]
pub(crate) mod view;

#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(all(target_arch = "wasm32", feature = "grid"))]
mod grid_element;
#[cfg(all(target_arch = "wasm32", feature = "pivot"))]
mod pivot_element;

/// The event names, readable on the host so the API freeze can check them.
///
/// They are defined once, here, and used by the element — a second spelling in
/// the browser code is exactly the kind of drift point 39 exists to prevent.
pub(crate) mod grid_element_events {
    /// Fired when the selection changed (plan point 35).
    pub const SELECTION_EVENT: &str = "opengrid-selection-change";
    /// Fired when a cell was edited (plan point 37).
    pub const CELL_EVENT: &str = "opengrid-cell-change";
    /// Fired when the view changed — sort, filters, columns or density
    /// (plan point 59). Scrolling and selecting are not view changes.
    pub const VIEW_EVENT: &str = "opengrid-view-change";
}
#[cfg(target_arch = "wasm32")]
pub use element::register;
