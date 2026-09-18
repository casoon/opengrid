//! `opengrid-datasource-postgres` — the PostgreSQL side of opengrid
//! (plan/spezifikation/07-server.md §Compiler-Pipeline).
//!
//! Point 25 builds the compiler: a `ValidatedQuery` becomes **parameterized** SQL
//! that reproduces the semantics S1–S14 exactly, checked against snapshots rather
//! than against a database. Point 26 adds the connection, the execution and the
//! conformance run against a real PostgreSQL.
//!
//! The compiler is the product's core, not a thin wrapper (E12): the whole point
//! is that the same query means the same thing in the browser and in the
//! database, and that is decided here, in the places where PostgreSQL's defaults
//! differ from the specification.

mod compiler;

pub use compiler::{CompileError, CompiledQuery, PostgresCompiler, QueryCompiler};
