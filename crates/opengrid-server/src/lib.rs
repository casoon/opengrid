//! `opengrid-server` — the data gateway, as a library (plan point 24, issue #45,
//! E36).
//!
//! Browser → HTTPS → this server → a data source. It accepts **only** the query
//! AST of `opengrid-query`, never SQL, and answers in the wire form of point 23
//! or the binary form of E35.
//!
//! **It knows no database.** The application builds it with [`Server::builder`]
//! and hands it its sources as [`Connector`]s — PostgreSQL, a file, anything.
//! Configuration is code: there is no configuration file and no binary here;
//! `examples/server` shows a program around it.
//!
//! Two promises the endpoint keeps, decided in point 22 (E15, E16):
//!
//! * **A bearer token** guards every request, compared in constant time. The
//!   token carries a context.
//! * **A mandatory row filter** per source is added to every query, with values
//!   from that context. It is enforced here, before a connector is asked, and no
//!   request can switch it off.

pub mod api;
mod export;
pub mod registry;
mod server;

pub use api::{AppState, router};
pub use opengrid_connector::Connector;
pub use registry::{Registry, RegistryError, RowFilter, SourcePolicy};
pub use server::{Server, ServerBuilder};
