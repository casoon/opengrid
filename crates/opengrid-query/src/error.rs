use std::error::Error;
use std::fmt;

use opengrid_types::DataType;

use crate::{AggregateFn, CmpOp};

/// Every way a query can be rejected by [`validate`](crate::Query::validate).
///
/// Errors carry a JSON path (`filter.and[1].value`, `sort[0].field`, …) so a
/// client can point at the offending part of the request. `code()` returns the
/// stable variant name used by the fixtures and, later, by the server's error
/// payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// The field does not exist in the schema (or, for a sort key, not in the output).
    UnknownField { path: String, field: String },
    /// The same output column is produced twice.
    DuplicateOutputColumn { path: String, column: String },
    /// The query would produce no columns at all.
    EmptyProjection,
    /// The operator does not apply to the field's type (e.g. `contains` on a number).
    OperatorNotSupported {
        path: String,
        op: CmpOp,
        data_type: DataType,
    },
    /// The JSON literal cannot be read as the field's type.
    ValueTypeMismatch {
        path: String,
        data_type: DataType,
        message: String,
    },
    /// `in` received something other than a JSON array.
    ListExpected { path: String },
    /// `in` list contains a NULL (rule S2).
    NullInList { path: String },
    /// Filter nesting exceeds `Limits::max_depth`.
    FilterTooDeep { depth: usize, max_depth: usize },
    /// In an aggregate query a selected field is neither grouped nor an aggregate alias.
    SelectNotGrouped { path: String, field: String },
    /// `sum`/`avg`/`min`/`max` need a field.
    AggregateFieldRequired { path: String, function: AggregateFn },
    /// The aggregate does not apply to the field's type.
    AggregateTypeMismatch {
        path: String,
        function: AggregateFn,
        data_type: DataType,
    },
    /// The alias collides with a column name of the schema.
    AliasConflictsWithField { path: String, alias: String },
    /// The same alias is used twice.
    DuplicateAlias { path: String, alias: String },
    /// The sort key does not name an output column.
    SortUnknownColumn { path: String, column: String },
    /// `offset` without a stable order (rule S6).
    OffsetWithoutSort,
    /// `limit` exceeds `Limits::max_limit`.
    LimitTooLarge { limit: u64, max_limit: u64 },
    /// A tree query that also groups or aggregates at the top (rule T10) —
    /// a subtree's aggregates go in `tree.aggregate` (T7).
    TreeWithGroup,
    /// The parent field's type is not the key's: a parent could never be found.
    TreeParentTypeMismatch { key: DataType, parent: DataType },
    /// `tree.flat` answers the whole tree, so it names no node to look under (T8).
    TreeFlatWithUnder,
    /// `tree.flat` with subtree aggregates: not part of the flat export yet (#166).
    TreeFlatWithAggregate,
    /// A source field has the name of a column the flat tree adds (T8):
    /// renaming either silently would leave an export nobody can read right.
    TreeFlatColumnTaken { column: String },
}

impl QueryError {
    /// Stable machine-readable code (the variant name), used in tests and on the wire.
    pub fn code(&self) -> &'static str {
        match self {
            QueryError::UnknownField { .. } => "UnknownField",
            QueryError::DuplicateOutputColumn { .. } => "DuplicateOutputColumn",
            QueryError::EmptyProjection => "EmptyProjection",
            QueryError::OperatorNotSupported { .. } => "OperatorNotSupported",
            QueryError::ValueTypeMismatch { .. } => "ValueTypeMismatch",
            QueryError::ListExpected { .. } => "ListExpected",
            QueryError::NullInList { .. } => "NullInList",
            QueryError::FilterTooDeep { .. } => "FilterTooDeep",
            QueryError::SelectNotGrouped { .. } => "SelectNotGrouped",
            QueryError::AggregateFieldRequired { .. } => "AggregateFieldRequired",
            QueryError::AggregateTypeMismatch { .. } => "AggregateTypeMismatch",
            QueryError::AliasConflictsWithField { .. } => "AliasConflictsWithField",
            QueryError::DuplicateAlias { .. } => "DuplicateAlias",
            QueryError::SortUnknownColumn { .. } => "SortUnknownColumn",
            QueryError::OffsetWithoutSort => "OffsetWithoutSort",
            QueryError::LimitTooLarge { .. } => "LimitTooLarge",
            QueryError::TreeWithGroup => "TreeWithGroup",
            QueryError::TreeParentTypeMismatch { .. } => "TreeParentTypeMismatch",
            QueryError::TreeFlatWithUnder => "TreeFlatWithUnder",
            QueryError::TreeFlatWithAggregate => "TreeFlatWithAggregate",
            QueryError::TreeFlatColumnTaken { .. } => "TreeFlatColumnTaken",
        }
    }

    /// The JSON path this error points at, when the error has one.
    pub fn path(&self) -> Option<&str> {
        match self {
            QueryError::UnknownField { path, .. }
            | QueryError::DuplicateOutputColumn { path, .. }
            | QueryError::OperatorNotSupported { path, .. }
            | QueryError::ValueTypeMismatch { path, .. }
            | QueryError::ListExpected { path }
            | QueryError::NullInList { path }
            | QueryError::SelectNotGrouped { path, .. }
            | QueryError::AggregateFieldRequired { path, .. }
            | QueryError::AggregateTypeMismatch { path, .. }
            | QueryError::AliasConflictsWithField { path, .. }
            | QueryError::DuplicateAlias { path, .. }
            | QueryError::SortUnknownColumn { path, .. } => Some(path),
            QueryError::EmptyProjection
            | QueryError::FilterTooDeep { .. }
            | QueryError::OffsetWithoutSort
            | QueryError::LimitTooLarge { .. } => None,
            QueryError::TreeWithGroup => Some("tree"),
            QueryError::TreeParentTypeMismatch { .. } => Some("tree.parent"),
            QueryError::TreeFlatWithUnder => Some("tree.under"),
            QueryError::TreeFlatWithAggregate => Some("tree.aggregate"),
            QueryError::TreeFlatColumnTaken { .. } => Some("tree.flat"),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::UnknownField { path, field } => {
                write!(f, "{path}: unknown field {field:?}")
            }
            QueryError::DuplicateOutputColumn { path, column } => {
                write!(f, "{path}: duplicate output column {column:?}")
            }
            QueryError::EmptyProjection => f.write_str("query produces no columns"),
            QueryError::OperatorNotSupported {
                path,
                op,
                data_type,
            } => write!(
                f,
                "{path}: operator {} is not supported for {data_type}",
                op.as_str()
            ),
            QueryError::ValueTypeMismatch {
                path,
                data_type,
                message,
            } => write!(f, "{path}: value is not a valid {data_type}: {message}"),
            QueryError::ListExpected { path } => {
                write!(f, "{path}: operator in expects a JSON array")
            }
            QueryError::NullInList { path } => {
                write!(f, "{path}: NULL is not allowed in an in list")
            }
            QueryError::FilterTooDeep { depth, max_depth } => {
                write!(f, "filter depth {depth} exceeds the maximum of {max_depth}")
            }
            QueryError::SelectNotGrouped { path, field } => write!(
                f,
                "{path}: field {field:?} is neither grouped nor an aggregate alias"
            ),
            QueryError::AggregateFieldRequired { path, function } => {
                write!(f, "{path}: {} needs a field", function.as_str())
            }
            QueryError::AggregateTypeMismatch {
                path,
                function,
                data_type,
            } => write!(
                f,
                "{path}: {} is not supported for {data_type}",
                function.as_str()
            ),
            QueryError::AliasConflictsWithField { path, alias } => {
                write!(f, "{path}: alias {alias:?} collides with a column name")
            }
            QueryError::DuplicateAlias { path, alias } => {
                write!(f, "{path}: duplicate alias {alias:?}")
            }
            QueryError::SortUnknownColumn { path, column } => {
                write!(f, "{path}: {column:?} is not an output column")
            }
            QueryError::OffsetWithoutSort => f.write_str("offset requires a sort (rule S6)"),
            QueryError::LimitTooLarge { limit, max_limit } => {
                write!(f, "limit {limit} exceeds the maximum of {max_limit}")
            }
            QueryError::TreeWithGroup => {
                f.write_str(
                    "tree: a tree is not grouped (rule T10); its aggregates go in tree.aggregate (rule T7)",
                )
            }
            QueryError::TreeParentTypeMismatch { key, parent } => write!(
                f,
                "tree.parent: the parent field is {parent:?} and the key {key:?} — a parent \
                 is found by its key, so both need the same type"
            ),
            QueryError::TreeFlatWithUnder => f.write_str(
                "tree.under: a flat tree is the whole tree, so it names no node to look under (rule T8)",
            ),
            QueryError::TreeFlatWithAggregate => f.write_str(
                "tree.aggregate: a flat tree does not carry subtree aggregates yet (rule T8, issue #166)",
            ),
            QueryError::TreeFlatColumnTaken { column } => write!(
                f,
                "tree.flat: the source has a field named {column:?}, the name of a column the flat \
                 tree adds (rule T8) — rename the field to export the tree flat"
            ),
        }
    }
}

impl Error for QueryError {}
