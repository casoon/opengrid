//! The query AST: the shared contract between the grid, the local engine and the
//! server (plan/spezifikation/02-query-modell.md).
//!
//! These types deserialize straight from the JSON contract. Filter literals are
//! kept as raw JSON here because their type is not known until validation against
//! a [`Schema`](opengrid_types::Schema) — the typed, guaranteed-valid counterpart
//! is [`ValidatedQuery`](crate::ValidatedQuery).
//!
//! Unknown fields are rejected everywhere (`deny_unknown_fields`), so a typo in a
//! request never silently changes its meaning.

use std::fmt;

use opengrid_types::{DataSourceId, DataType, FieldName};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as JsonValue;

/// A full query as sent by a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub source: DataSourceId,
    #[serde(default)]
    pub select: Vec<FieldName>,
    #[serde(default)]
    pub filter: Option<FilterExpr>,
    #[serde(default)]
    pub group: Vec<FieldName>,
    #[serde(default)]
    pub aggregate: Vec<Aggregate>,
    #[serde(default)]
    pub sort: Vec<Sort>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

// ---------------------------------------------------------------------------
// Filter expression
// ---------------------------------------------------------------------------

/// A filter tree. `and`/`or`/`not` combine comparisons; each comparison names a
/// field and carries a raw JSON literal.
#[derive(Clone, Debug, PartialEq)]
pub enum FilterExpr {
    And(Vec<FilterExpr>),
    Or(Vec<FilterExpr>),
    Not(Box<FilterExpr>),
    Cmp {
        field: FieldName,
        op: CmpOp,
        value: JsonValue,
    },
    IsNull {
        field: FieldName,
    },
    IsNotNull {
        field: FieldName,
    },
}

impl FilterExpr {
    /// Nesting depth of the logical operators; a single comparison is depth 1.
    pub fn depth(&self) -> usize {
        match self {
            FilterExpr::And(items) | FilterExpr::Or(items) => {
                1 + items.iter().map(Self::depth).max().unwrap_or(0)
            }
            FilterExpr::Not(inner) => 1 + inner.depth(),
            FilterExpr::Cmp { .. } | FilterExpr::IsNull { .. } | FilterExpr::IsNotNull { .. } => 1,
        }
    }
}

/// Filter operators that carry a value (semantics-ready, plan/spezifikation
/// 02-query-modell.md). `is_null`/`is_not_null` are not operators here but their
/// own [`FilterExpr`] variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    In,
    Contains,
    StartsWith,
}

impl CmpOp {
    /// The wire name, identical to the JSON string.
    pub fn as_str(&self) -> &'static str {
        match self {
            CmpOp::Eq => "eq",
            CmpOp::Ne => "ne",
            CmpOp::Lt => "lt",
            CmpOp::Lte => "lte",
            CmpOp::Gt => "gt",
            CmpOp::Gte => "gte",
            CmpOp::In => "in",
            CmpOp::Contains => "contains",
            CmpOp::StartsWith => "starts_with",
        }
    }

    /// Parses a wire operator name; `is_null`/`is_not_null` are handled separately.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "eq" => CmpOp::Eq,
            "ne" => CmpOp::Ne,
            "lt" => CmpOp::Lt,
            "lte" => CmpOp::Lte,
            "gt" => CmpOp::Gt,
            "gte" => CmpOp::Gte,
            "in" => CmpOp::In,
            "contains" => CmpOp::Contains,
            "starts_with" => CmpOp::StartsWith,
            _ => return None,
        })
    }

    /// True for the two string operators (case-sensitive in V1, rule S5).
    pub fn is_string_op(&self) -> bool {
        matches!(self, CmpOp::Contains | CmpOp::StartsWith)
    }
}

/// Flat view of a single filter object used for deserialization.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterRepr {
    #[serde(default)]
    and: Option<Vec<FilterExpr>>,
    #[serde(default)]
    or: Option<Vec<FilterExpr>>,
    #[serde(default)]
    not: Option<Box<FilterExpr>>,
    #[serde(default)]
    field: Option<FieldName>,
    #[serde(default)]
    op: Option<String>,
    #[serde(default)]
    value: Option<JsonValue>,
}

impl FilterRepr {
    fn into_expr(self) -> Result<FilterExpr, String> {
        // Decide the shape before moving anything out of `self`.
        let only_logical_keys = self.field.is_none() && self.op.is_none() && self.value.is_none();
        let logical_keys = [self.and.is_some(), self.or.is_some(), self.not.is_some()]
            .into_iter()
            .filter(|&set| set)
            .count();
        if logical_keys > 1 {
            return Err("filter expression must use only one of: and, or, not".to_owned());
        }
        if let Some(and) = self.and {
            if !only_logical_keys {
                return Err("and must not carry field, op or value".to_owned());
            }
            return Ok(FilterExpr::And(and));
        }
        if let Some(or) = self.or {
            if !only_logical_keys {
                return Err("or must not carry field, op or value".to_owned());
            }
            return Ok(FilterExpr::Or(or));
        }
        if let Some(not) = self.not {
            if !only_logical_keys {
                return Err("not must not carry field, op or value".to_owned());
            }
            return Ok(FilterExpr::Not(not));
        }
        let Some(field) = self.field else {
            return Err("filter expression needs one of: and, or, not, field".to_owned());
        };
        let Some(op) = self.op else {
            return Err(format!("field {field:?} needs an \"op\""));
        };
        match op.as_str() {
            "is_null" => {
                if self.value.is_some() {
                    return Err("is_null takes no value".to_owned());
                }
                Ok(FilterExpr::IsNull { field })
            }
            "is_not_null" => {
                if self.value.is_some() {
                    return Err("is_not_null takes no value".to_owned());
                }
                Ok(FilterExpr::IsNotNull { field })
            }
            other => {
                let op =
                    CmpOp::parse(other).ok_or_else(|| format!("unknown operator {other:?}"))?;
                let Some(value) = self.value else {
                    return Err(format!("operator {} needs a \"value\"", op.as_str()));
                };
                Ok(FilterExpr::Cmp { field, op, value })
            }
        }
    }
}

impl<'de> Deserialize<'de> for FilterExpr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        FilterRepr::deserialize(deserializer)?
            .into_expr()
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for FilterExpr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Logical<'a> {
            and: &'a Vec<FilterExpr>,
        }
        #[derive(Serialize)]
        struct LogicalOr<'a> {
            or: &'a Vec<FilterExpr>,
        }
        #[derive(Serialize)]
        struct LogicalNot<'a> {
            not: &'a FilterExpr,
        }
        #[derive(Serialize)]
        struct Cmp<'a> {
            field: &'a FieldName,
            op: CmpOp,
            value: &'a JsonValue,
        }
        #[derive(Serialize)]
        struct NullCheck<'a> {
            field: &'a FieldName,
            op: &'static str,
        }

        match self {
            FilterExpr::And(items) => Logical { and: items }.serialize(serializer),
            FilterExpr::Or(items) => LogicalOr { or: items }.serialize(serializer),
            FilterExpr::Not(inner) => LogicalNot { not: inner }.serialize(serializer),
            FilterExpr::Cmp { field, op, value } => Cmp {
                field,
                op: *op,
                value,
            }
            .serialize(serializer),
            FilterExpr::IsNull { field } => NullCheck {
                field,
                op: "is_null",
            }
            .serialize(serializer),
            FilterExpr::IsNotNull { field } => NullCheck {
                field,
                op: "is_not_null",
            }
            .serialize(serializer),
        }
    }
}

// ---------------------------------------------------------------------------
// Sort
// ---------------------------------------------------------------------------

/// Sort direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// Where NULLs land. Always explicit in compiled SQL, default `last` regardless
/// of direction (rule S3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullsOrder {
    First,
    #[default]
    Last,
}

/// String collation. V1 knows only `binary` (rule S4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collation {
    #[default]
    Binary,
}

/// One sort key on an output column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sort {
    pub field: FieldName,
    #[serde(default)]
    pub direction: SortDirection,
    #[serde(default)]
    pub nulls: NullsOrder,
    #[serde(default)]
    pub collation: Collation,
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Aggregate functions in V1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateFn {
    Sum,
    Avg,
    Count,
    Min,
    Max,
}

impl AggregateFn {
    /// The wire name.
    pub fn as_str(&self) -> &'static str {
        match self {
            AggregateFn::Sum => "sum",
            AggregateFn::Avg => "avg",
            AggregateFn::Count => "count",
            AggregateFn::Min => "min",
            AggregateFn::Max => "max",
        }
    }

    /// Only `count` may run without a field (`count(*)`).
    pub fn requires_field(&self) -> bool {
        !matches!(self, AggregateFn::Count)
    }

    /// Result type for an input type, per the aggregate table of
    /// plan/spezifikation/02-query-modell.md (rule S12). `None` means the
    /// aggregate does not apply to that input type.
    pub fn result_type(&self, input: DataType) -> Option<DataType> {
        match self {
            AggregateFn::Count => Some(DataType::Int64),
            AggregateFn::Sum => match input {
                DataType::Int64 => Some(DataType::Int64),
                DataType::Float64 => Some(DataType::Float64),
                DataType::Decimal { scale, .. } => {
                    DataType::decimal(DataType::MAX_DECIMAL_PRECISION, scale).ok()
                }
                _ => None,
            },
            AggregateFn::Avg => match input {
                DataType::Int64 | DataType::Float64 | DataType::Decimal { .. } => {
                    Some(DataType::Float64)
                }
                _ => None,
            },
            AggregateFn::Min | AggregateFn::Max => Some(input),
        }
    }
}

/// One aggregate over a field, published under an alias.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aggregate {
    /// Missing for `count(*)`.
    #[serde(default)]
    pub field: Option<FieldName>,
    #[serde(rename = "fn")]
    pub function: AggregateFn,
    #[serde(rename = "as")]
    pub alias: FieldName,
}

impl fmt::Display for FilterExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterExpr::And(_) => f.write_str("and(...)"),
            FilterExpr::Or(_) => f.write_str("or(...)"),
            FilterExpr::Not(_) => f.write_str("not(...)"),
            FilterExpr::Cmp { field, op, .. } => write!(f, "{} {}", field, op.as_str()),
            FilterExpr::IsNull { field } => write!(f, "{field} is_null"),
            FilterExpr::IsNotNull { field } => write!(f, "{field} is_not_null"),
        }
    }
}
