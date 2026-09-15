//! The opengrid query contract: AST, JSON shape and validation.
//!
//! ```no_run
//! use opengrid_query::{Query, Limits};
//! use opengrid_types::{DataType, Field, FieldName, Schema};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = Schema::new(vec![Field::required(
//!     FieldName::new("customer")?,
//!     DataType::Utf8,
//! )]);
//! let query: Query = serde_json::from_str(r#"{"source":"orders","select":["customer"]}"#)?;
//! let _validated = query.validate(&schema, &Limits::default())?;
//! # Ok(())
//! # }
//! ```
//!
//! See plan/spezifikation/02-query-modell.md for the JSON contract and the
//! semantics rules S1–S14.

mod ast;
mod error;
mod validate;

pub use ast::{
    Aggregate, AggregateFn, CmpOp, Collation, FilterExpr, NullsOrder, Query, Sort, SortDirection,
};
pub use error::QueryError;
pub use validate::{Limits, ValidatedFilter, ValidatedQuery};
