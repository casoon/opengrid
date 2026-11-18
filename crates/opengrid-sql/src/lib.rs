//! `opengrid-sql` — one SQL compiler for opengrid's fixed query vocabulary
//! (fields, comparisons, `and`/`or`, sort, window, group, pivot, tree), over a
//! [`ValidatedQuery`](opengrid_query::ValidatedQuery).
//!
//! What the databases share is written once, in [`Compiler`]; what differs is a
//! [`Dialect`]: quoting, placeholders, NULL order, collation, how decimals and
//! dates are stored, the aggregate casts, and what the database cannot do
//! (`GROUPING SETS`, `WITH RECURSIVE`). A new database is a dialect and a driver.
//! [`Postgres`] and [`Sqlite`] are the two there are.
//!
//! Two properties matter more than anything else, and both are structural rather
//! than a matter of care:
//!
//! * **No value ever reaches the SQL text.** Every literal becomes a placeholder
//!   and travels in [`CompiledQuery::params`]. There is exactly one method that
//!   can put a value anywhere (`Sql::param`) and it can only produce a
//!   placeholder.
//! * **Identifiers come from the schema, never from the request.** A column name
//!   that reached the compiler was checked against the schema by validation; a
//!   table name comes from the configuration. [`quote_ident`] rejects the one
//!   character that could break out.
//!
//! No query builder is used: the vocabulary is small and fixed, and the hard
//! part is the semantics (rules S1–S14), which a builder does not solve.

mod compiler;
mod dialect;
mod postgres;
mod sqlite;
mod tree;

pub use compiler::{CompileError, CompiledQuery, Compiler, GROUPING_PREFIX, QueryCompiler};
pub use dialect::{Dialect, Sql, quote_ident};
pub use postgres::Postgres;
pub use sqlite::{COLLATION, FOLD, Sqlite};
pub use tree::{TREE_CHILDREN, TREE_MATCHED};
