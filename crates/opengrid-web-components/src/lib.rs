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

pub mod grid;
pub mod pivot;
pub mod table;
pub mod texts;

#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod grid_element;
#[cfg(target_arch = "wasm32")]
mod pivot_element;
#[cfg(target_arch = "wasm32")]
pub use element::register;
