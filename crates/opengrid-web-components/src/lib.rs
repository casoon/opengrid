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

pub mod grid;
pub mod table;

#[cfg(target_arch = "wasm32")]
mod element;
#[cfg(target_arch = "wasm32")]
mod grid_element;
#[cfg(target_arch = "wasm32")]
pub use element::register;
