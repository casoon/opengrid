//! Column storage for the opengrid engine: typed columns, NULL bitmaps, and the
//! kernels the executor is built from — gather ([`Column::take`]) and sort
//! ([`sort::order`]) — plus the binary form a result travels in ([`wire`],
//! decision E35).
//!
//! Decision E34 (plan/spezifikation/14-entscheidungen.md): the engine used to
//! keep its data in Arrow. Arrow's kernels branch over every Arrow type — lists,
//! structs, dictionaries — and nearly all of the engine module was that
//! generality. This crate knows exactly the seven types of the query model
//! ([`DataType`]) and nothing else; a new type comes through the query model,
//! never through this crate.
//!
//! What the crate does **not** know is query semantics: three-valued filter
//! logic, the S7 comparison of floats, grouping and aggregation live in the
//! engine. The one ordering this crate defines is the sort order of
//! [`sort::order`], and its rules are written there.
//!
//! Like every engine crate it pulls in no browser dependency (`web-sys`,
//! `js-sys`), see plan/spezifikation/11-crates.md §Portabilität.

mod bitmap;
mod column;
pub mod sort;
mod table;
pub mod wire;

pub use bitmap::Bitmap;
pub use column::{Column, ColumnBuilder, Values};
pub use table::Table;
