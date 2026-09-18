//! Arrow integration for opengrid: ingest (CSV/JSON → `RecordBatch`), the local
//! query executor and the `DataSource` adapter over it.
//!
//! Arrow is the internal data model (plan/spezifikation/04-local-engine.md
//! §Arrow als internes Datenmodell). Per decision E3 the crate uses arrow-rs
//! **part crates** — never the `arrow` meta-crate. It is an engine crate, so it
//! pulls in no browser dependency (`web-sys`/`js-sys`), see
//! plan/spezifikation/11-crates.md §Portabilität.

pub mod datasource;
pub mod execute;
pub mod hybrid;
pub mod ingest;
