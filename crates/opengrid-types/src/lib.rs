//! Shared type system for opengrid: schemas, data types, values and identifiers.
//!
//! This crate is the contract every other opengrid component builds on (query AST,
//! local engine, server, SQL compilers). It deliberately carries **no Arrow
//! dependency**: the Arrow mapping lives behind the optional `arrow` feature. It
//! also pulls in no browser dependency (see plan/spezifikation/11-crates.md
//! §Portabilität).
//!
//! Type system and JSON representation follow plan/spezifikation/02-query-modell.md
//! §Typsystem: `Bool`, `Int64`, `Float64`, `Decimal(p, s)`, `Utf8`, `Date`,
//! `Timestamp` (UTC, microseconds). Decimals travel as JSON strings, dates as
//! `YYYY-MM-DD` and timestamps as ISO-8601 with a trailing `Z`.

mod data_type;
mod error;
mod identifier;
mod schema;
mod value;

#[cfg(feature = "arrow")]
mod arrow;

pub use data_type::DataType;
pub use error::{InvalidIdentifier, ValueError};
pub use identifier::{DataSourceId, FieldName, is_valid_identifier};
pub use schema::{Field, Schema};
pub use value::{Date, Decimal, Timestamp, Value};
