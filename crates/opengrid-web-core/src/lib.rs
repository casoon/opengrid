//! `opengrid-web-core` — the DOM layer under the grid components (point 13).
//!
//! The renderer is deliberately small (plan/spezifikation/08-rendering.md
//! §Möglichst wenig Abstraktion): Rust → `wasm-bindgen` → `web-sys` → DOM, no
//! virtual DOM and no framework runtime. A change detection computes a list of
//! [`Patch`](patch::Patch)es, and the whole list is applied to the DOM once per
//! frame ([`patch::PatchBuffer`]) — not element by element, cell by cell
//! (risk R1, plan/spezifikation/13-risiken.md).
//!
//! # Portability
//!
//! Patch computation is pure data and stays native-testable; only the DOM glue
//! behind the [`Renderer`](renderer::Renderer) trait is browser-only and gated by
//! `#[cfg(target_arch = "wasm32")]`. `web-sys` and `js-sys` are pulled in for the
//! browser target only, so `cargo test -p opengrid-web-core` runs the batching
//! tests on the host (plan/spezifikation/11-crates.md §Portabilität).
//!
//! # Decisions the layer carries
//!
//! * **Open shadow root, all ARIA ids inside it** (E8/R6): [`element`] attaches
//!   the open root and mirrors the host `label` attribute to the inner element's
//!   `aria-label`; `aria-labelledby` and `<label for>` never cross the boundary.
//! * **Async data interface** (plan/spezifikation/04-local-engine.md §Worker
//!   "Zeitpunkt"): [`provider`] is Promise-based, so point 19 only swaps the
//!   implementation for a Worker-backed one.

pub mod element;
pub mod host;
pub mod patch;
pub mod provider;
pub mod renderer;
