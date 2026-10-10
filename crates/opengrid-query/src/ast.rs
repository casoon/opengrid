//! The query AST: the shared contract between the grid, the local engine and the
//! server (plan/spezifikation/02-query-modell.md).
//!
//! These types deserialize straight from the JSON contract. Filter literals are
//! kept as raw JSON here because their type is not known until validation against
//! a [`Schema`](opengrid_types::Schema) — the typed, guaranteed-valid counterpart
//! is [`ValidatedQuery`](crate::ValidatedQuery).
//!
//! Unknown fields are rejected everywhere, so a typo in a request never
//! silently changes its meaning. The JSON forms are read and written through
//! `opengrid-json`, the one codec of browser and server (issue #41).

use std::fmt;

use opengrid_json::{Error, Fields, FromJson, Json, ToJson, unknown_variant};
use opengrid_types::{DataSourceId, DataType, FieldName};

/// A full query as sent by a client.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub source: DataSourceId,
    pub select: Vec<FieldName>,
    pub filter: Option<FilterExpr>,
    pub group: Vec<FieldName>,
    pub aggregate: Vec<Aggregate>,
    pub sort: Vec<Sort>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
    /// One level of a tree (E38, rules T1–T10): the rows hang from each other
    /// through a parent field, and the query asks for the children of one node.
    pub tree: Option<TreeSpec>,
}

/// The tree part of a query (E38): which field is a node's key, which names
/// its parent, and whose children are asked for — the roots when `under` is
/// absent or NULL.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeSpec {
    pub key: FieldName,
    pub parent: FieldName,
    pub under: Option<Json>,
    /// Which rows the tree consists of (plan point 122): a row outside the
    /// scope does not exist — not as a match, not as context, not as a child —
    /// and a node whose parent is outside is an orphan. The server puts the
    /// mandatory row filter (E16) here, so no other tenant's row shows.
    pub scope: Option<FilterExpr>,
    /// Aggregates over each node's subtree (rule T7, issue #165): the node and
    /// all its descendants, matches only, from the raw rows. They come back
    /// beside the level's rows, never as columns of it.
    pub aggregate: Vec<Aggregate>,
    /// The whole tree instead of one level (rule T8, issue #166): every
    /// visible node, depth-first with siblings sorted, paged like any query —
    /// each row with its level, its path and whether it matches.
    pub flat: bool,
}

impl FromJson for TreeSpec {
    /// `{ "key"?, "parent", "under"?, "scope"?, "aggregate"?, "flat"? }`;
    /// `key` defaults to `id`.
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(
            json,
            "struct TreeSpec",
            &["key", "parent", "under", "scope", "aggregate", "flat"],
        )?;
        Ok(TreeSpec {
            key: match fields.read_optional("key")? {
                Some(key) => key,
                None => FieldName::new("id").expect("`id` is a field name"),
            },
            parent: fields.read("parent")?,
            under: fields
                .optional("under")
                .filter(|under| !under.is_null())
                .cloned(),
            scope: fields.read_optional("scope")?,
            aggregate: fields.read_or_default("aggregate")?,
            flat: fields.read_or_default("flat")?,
        })
    }
}

impl ToJson for TreeSpec {
    fn to_json(&self) -> Json {
        let mut json = opengrid_json::json!({
            "key": self.key,
            "parent": self.parent,
            "under": self.under.clone().unwrap_or(Json::Null),
        });
        if let (Some(scope), Json::Object(object)) = (&self.scope, &mut json) {
            object.insert("scope".to_owned(), scope.to_json());
        }
        // Written only when there are any: a reader from before T7 reads the rest.
        if let (false, Json::Object(object)) = (self.aggregate.is_empty(), &mut json) {
            object.insert("aggregate".to_owned(), self.aggregate.to_json());
        }
        if let (true, Json::Object(object)) = (self.flat, &mut json) {
            object.insert("flat".to_owned(), Json::Bool(true));
        }
        json
    }
}

const QUERY_FIELDS: [&str; 9] = [
    "source",
    "select",
    "filter",
    "group",
    "aggregate",
    "sort",
    "offset",
    "limit",
    "tree",
];

impl FromJson for Query {
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct Query", &QUERY_FIELDS)?;
        Ok(Query {
            source: fields.read("source")?,
            select: fields.read_or_default("select")?,
            filter: fields.read_optional("filter")?,
            group: fields.read_or_default("group")?,
            aggregate: fields.read_or_default("aggregate")?,
            sort: fields.read_or_default("sort")?,
            offset: fields.read_optional("offset")?,
            limit: fields.read_optional("limit")?,
            tree: fields.read_optional("tree")?,
        })
    }
}

impl ToJson for Query {
    /// `tree` only when there is one: a server from before E38 reads the rest.
    fn to_json(&self) -> Json {
        let mut json = opengrid_json::json!({
            "source": self.source,
            "select": self.select,
            "filter": self.filter,
            "group": self.group,
            "aggregate": self.aggregate,
            "sort": self.sort,
            "offset": self.offset,
            "limit": self.limit,
        });
        if let (Some(tree), Json::Object(object)) = (&self.tree, &mut json) {
            object.insert("tree".to_owned(), tree.to_json());
        }
        json
    }
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
        value: Json,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

/// The keys a filter object may have.
const FILTER_FIELDS: [&str; 6] = ["and", "or", "not", "field", "op", "value"];

impl FromJson for FilterExpr {
    /// One filter object: `and`, `or` or `not` alone, or a `field` with an
    /// `op` and — unless the operator is a null check — a `value`. A key set to
    /// `null` counts as absent.
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct FilterRepr", &FILTER_FIELDS)?;
        let and = fields.read_optional::<Vec<FilterExpr>>("and")?;
        let or = fields.read_optional::<Vec<FilterExpr>>("or")?;
        let not = fields.read_optional::<Box<FilterExpr>>("not")?;
        let field = fields.read_optional::<FieldName>("field")?;
        let op = fields.read_optional::<String>("op")?;
        let value = fields.optional("value").cloned();

        // Decide the shape before moving anything out.
        let only_logical_keys = field.is_none() && op.is_none() && value.is_none();
        let logical_keys = [and.is_some(), or.is_some(), not.is_some()]
            .into_iter()
            .filter(|&set| set)
            .count();
        let fail = |message: String| Err(Error::new(message));
        if logical_keys > 1 {
            return fail("filter expression must use only one of: and, or, not".to_owned());
        }
        if let Some(and) = and {
            if !only_logical_keys {
                return fail("and must not carry field, op or value".to_owned());
            }
            return Ok(FilterExpr::And(and));
        }
        if let Some(or) = or {
            if !only_logical_keys {
                return fail("or must not carry field, op or value".to_owned());
            }
            return Ok(FilterExpr::Or(or));
        }
        if let Some(not) = not {
            if !only_logical_keys {
                return fail("not must not carry field, op or value".to_owned());
            }
            return Ok(FilterExpr::Not(not));
        }
        let Some(field) = field else {
            return fail("filter expression needs one of: and, or, not, field".to_owned());
        };
        let Some(op) = op else {
            return fail(format!("field {field:?} needs an \"op\""));
        };
        match op.as_str() {
            "is_null" => {
                if value.is_some() {
                    return fail("is_null takes no value".to_owned());
                }
                Ok(FilterExpr::IsNull { field })
            }
            "is_not_null" => {
                if value.is_some() {
                    return fail("is_not_null takes no value".to_owned());
                }
                Ok(FilterExpr::IsNotNull { field })
            }
            other => {
                let op = CmpOp::parse(other)
                    .ok_or_else(|| Error::new(format!("unknown operator {other:?}")))?;
                let Some(value) = value else {
                    return fail(format!("operator {} needs a \"value\"", op.as_str()));
                };
                Ok(FilterExpr::Cmp { field, op, value })
            }
        }
    }
}

impl ToJson for FilterExpr {
    fn to_json(&self) -> Json {
        match self {
            FilterExpr::And(items) => opengrid_json::json!({ "and": items }),
            FilterExpr::Or(items) => opengrid_json::json!({ "or": items }),
            FilterExpr::Not(inner) => opengrid_json::json!({ "not": inner }),
            FilterExpr::Cmp { field, op, value } => {
                opengrid_json::json!({ "field": field, "op": op.as_str(), "value": value })
            }
            FilterExpr::IsNull { field } => {
                opengrid_json::json!({ "field": field, "op": "is_null" })
            }
            FilterExpr::IsNotNull { field } => {
                opengrid_json::json!({ "field": field, "op": "is_not_null" })
            }
        }
    }
}

/// A unit enum written as its snake-case name.
macro_rules! named {
    ($name:ident { $($variant:ident = $text:literal),* $(,)? }) => {
        impl $name {
            fn name(&self) -> &'static str {
                match self {
                    $($name::$variant => $text,)*
                }
            }
        }

        impl FromJson for $name {
            fn from_json(json: &Json) -> Result<Self, Error> {
                let text = String::from_json(json)?;
                match text.as_str() {
                    $($text => Ok($name::$variant),)*
                    other => Err(unknown_variant(other, &[$($text),*])),
                }
            }
        }

        impl ToJson for $name {
            fn to_json(&self) -> Json {
                Json::from(self.name())
            }
        }
    };
}

named!(CmpOp {
    Eq = "eq",
    Ne = "ne",
    Lt = "lt",
    Lte = "lte",
    Gt = "gt",
    Gte = "gte",
    In = "in",
    Contains = "contains",
    StartsWith = "starts_with",
});
named!(SortDirection { Asc = "asc", Desc = "desc" });
named!(NullsOrder { First = "first", Last = "last" });
named!(Collation { Binary = "binary" });
named!(AggregateFn {
    Sum = "sum",
    Avg = "avg",
    Count = "count",
    Min = "min",
    Max = "max",
});

// ---------------------------------------------------------------------------
// Sort
// ---------------------------------------------------------------------------

/// Sort direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// Where NULLs land. Always explicit in compiled SQL, default `last` regardless
/// of direction (rule S3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NullsOrder {
    First,
    #[default]
    Last,
}

/// String collation. V1 knows only `binary` (rule S4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Collation {
    #[default]
    Binary,
}

/// One sort key on an output column.
#[derive(Clone, Debug, PartialEq)]
pub struct Sort {
    pub field: FieldName,
    pub direction: SortDirection,
    pub nulls: NullsOrder,
    pub collation: Collation,
}

impl FromJson for Sort {
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(
            json,
            "struct Sort",
            &["field", "direction", "nulls", "collation"],
        )?;
        Ok(Sort {
            field: fields.read("field")?,
            direction: fields.read_or_default("direction")?,
            nulls: fields.read_or_default("nulls")?,
            collation: fields.read_or_default("collation")?,
        })
    }
}

impl ToJson for Sort {
    fn to_json(&self) -> Json {
        opengrid_json::json!({
            "field": self.field,
            "direction": self.direction,
            "nulls": self.nulls,
            "collation": self.collation,
        })
    }
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Aggregate functions in V1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct Aggregate {
    /// Missing for `count(*)`.
    pub field: Option<FieldName>,
    pub function: AggregateFn,
    pub alias: FieldName,
}

impl FromJson for Aggregate {
    /// `{ "field", "fn", "as" }`; no `field` is `count(*)`.
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct Aggregate", &["field", "fn", "as"])?;
        Ok(Aggregate {
            field: fields.read_optional("field")?,
            function: fields.read("fn")?,
            alias: fields.read("as")?,
        })
    }
}

impl ToJson for Aggregate {
    fn to_json(&self) -> Json {
        opengrid_json::json!({ "field": self.field, "fn": self.function, "as": self.alias })
    }
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
