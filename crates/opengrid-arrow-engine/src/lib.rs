//! Arrow integration for opengrid: ingest (CSV/JSON → `RecordBatch`) and, from
//! point 07 on, the local query executor.
//!
//! Arrow is the internal data model (plan/spezifikation/04-local-engine.md
//! §Arrow als internes Datenmodell). Per decision E3 the crate uses arrow-rs
//! **part crates** — never the `arrow` meta-crate. It is an engine crate, so it
//! pulls in no browser dependency (`web-sys`/`js-sys`), see
//! plan/spezifikation/11-crates.md §Portabilität.

pub mod ingest;
