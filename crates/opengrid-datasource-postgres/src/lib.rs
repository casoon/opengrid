//! `opengrid-datasource-postgres` — the PostgreSQL side of opengrid
//! (plan/spezifikation/07-server.md §Compiler-Pipeline).
//!
//! Point 25 builds the compiler: a `ValidatedQuery` becomes **parameterized** SQL
//! that reproduces the semantics S1–S14 exactly, checked against snapshots rather
//! than against a database. Point 26 adds the connection, the execution and the
//! conformance run against a real PostgreSQL. Issue #2 adds the export: one
//! query read through a cursor, a piece at a time ([`PostgresExport`]).
//!
//! Issue #45 makes it a reference [`Connector`](opengrid_connector::Connector):
//! the server takes it like any other source, with the pivot pushed down to
//! `GROUPING SETS` and the export read through a cursor.
//!
//! The compiler (`opengrid-sql`, one for every SQL database, with a dialect per
//! database) is the product's core, not a thin wrapper (E12): the whole point
//! is that the same query means the same thing in the browser and in the
//! database, and that is decided here, in the places where PostgreSQL's defaults
//! differ from the specification.

mod connector;
mod export;
mod lease;
mod source;
mod tls;
mod tree;

pub use opengrid_sql::{CompileError, CompiledQuery, QueryCompiler};

/// The shared SQL compiler in its PostgreSQL dialect (issue #485).
pub type PostgresCompiler = opengrid_sql::Compiler<opengrid_sql::Postgres>;
pub use export::{ExportCanceller, PostgresExport};
pub use source::{PoolOptions, PostgresDataSource};
#[cfg(feature = "tls")]
pub use tls::rustls_connector;
