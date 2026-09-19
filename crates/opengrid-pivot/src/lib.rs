//! `opengrid-pivot` — a pivot is a set of grouping sets plus a reshaping
//! (plan points 30 and 52, plan/spezifikation/06-pivot.md).
//!
//! # The one idea
//!
//! For row dimensions `r1..rn` and column dimensions `c1..cm`, the answer is
//! exactly what `n+1` ordinary grouped queries produce — `group = [r1..rk,
//! c1..cm]` for `k = n..0` — rearranged into a matrix. Three things follow, and
//! they shape everything here:
//!
//! 1. **Pivot semantics are query semantics.** Every cell is an aggregate over a
//!    group, and rules S1–S15 describe those completely. There is no second
//!    semantics to prove, and this crate re-implements none of the first.
//! 2. **No Arrow.** The engine sits on the [`DataSource`] trait, so it works over
//!    the local engine, PostgreSQL, REST and the hybrid path alike.
//! 3. **Subtotals are asked for, not rolled up.** The average of averages is not
//!    an average (S12), so every level is its own grouping over the raw rows.
//!
//! # What this crate never does
//!
//! It never compares two values to decide an order. Each grouping set is
//! requested **sorted** by its own keys, so the order comes from whoever owns
//! rules S3/S4 — the engine or the database. Assembling the matrix needs only
//! equality, and the one place that needs it says so ([`same_key`]).

mod assemble;
mod engine;
mod query;
mod result;

pub use engine::{ExecuteError, PivotEngine, assemble, execute};
pub use query::{PivotError, PivotLimits, PivotQuery, ValidatedPivotQuery, grouping_sets};
pub use result::{PivotColumn, PivotResult, pivot_to_json};

pub(crate) use assemble::{Key, same_key};
