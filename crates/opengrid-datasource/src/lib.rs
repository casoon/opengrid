//! The `DataSource` contract: what the UI and the planner talk to.
//!
//! From point 09 on, no component above the engine knows about Arrow. The local
//! engine, the REST client and the PostgreSQL server are all reached through
//! [`DataSource`] (plan/spezifikation/03-datasource.md), and they all answer with
//! the same Arrow-free [`QueryResult`].
//!
//! * **The trait is async** (decision E5): `async fn` in the trait, no
//!   `async-trait` boxing. [`DataSource`] itself is the browser variant and
//!   carries no `Send` bound; `SendDataSource` is the server variant and is
//!   generated from the same definition by `trait-variant`.
//! * **The result is Arrow-free** (decision E14): `schema`, one `Vec<Value>` per
//!   output column and `total_count` — the column-oriented shape of the wire
//!   format (E6). Arrow stays inside `opengrid-arrow-engine`.
//! * **The crate is portable** (plan/spezifikation/11-crates.md §Portabilität):
//!   neither Arrow nor `web-sys`/`js-sys` are dependencies, so it builds for
//!   `wasm32-unknown-unknown`, natively and on the server.

mod capabilities;
mod error;
mod result;
mod source;

pub use capabilities::DataSourceCapabilities;
pub use error::DataSourceError;
pub use result::QueryResult;
pub use source::{DataSource, SendDataSource};

/// Where a query is executed.
///
/// Only `Local` is produced in MVP A; `Auto` is resolved by the planner from the
/// estimated rows and bytes, the browser's memory, the query's complexity and the
/// server's capabilities (plan/spezifikation/03-datasource.md, point 28).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionMode {
    /// In the client, over the in-memory engine (`LocalDataSource`).
    Local,
    /// On the server, over a remote data source.
    Remote,
    /// Split between both.
    Hybrid,
    /// Decided per query by the planner.
    Auto,
}
