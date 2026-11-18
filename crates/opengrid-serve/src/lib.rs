//! `opengrid-serve` — the `opengrid-server` command (issue #486).
//!
//! From a connection string to a running grid, in three steps:
//!
//! 1. [`propose`]: read the database's tables and columns and write a
//!    configuration proposal — sources, fields, types. A column whose type has
//!    no mapping is named and left out, never guessed.
//! 2. The owner reads the file, edits it, and deletes `"proposal": true`.
//! 3. [`Config::into_builder`]: the file becomes the server's builder.
//!
//! **Nothing is released automatically.** A file that still says it is a
//! proposal does not start; only the columns the file names exist for a client;
//! the token, the allowed fields and the tenant's row filter are the server
//! builder's own. **Secrets never live in the file:** the connection string and
//! every token are read from environment variables the file *names*.

mod config;
mod pretty;
mod propose;

pub use config::{Config, ConfigError, Database, MIN_TOKEN_LENGTH};
pub use propose::{DEFAULT_CONNECTION_ENV, DEFAULT_TOKEN_ENV, proposal, read_proposal};
