//! The opengrid engine: ingest (CSV/JSON → [`Table`](opengrid_columns::Table)),
//! the local query executor and the `DataSource` adapter over it.
//!
//! The data lives in `opengrid-columns` — typed columns of exactly the types of
//! the query model (decision E34, which replaced Arrow; see
//! plan/spezifikation/04-local-engine.md). It is an engine crate, so it pulls in
//! no browser dependency (`web-sys`/`js-sys`), see
//! plan/spezifikation/11-crates.md §Portabilität.

pub mod datasource;
pub mod execute;
pub mod hybrid;
pub mod ingest;

pub use opengrid_columns::Table;
