//! Validation: turns a [`Query`] into a [`ValidatedQuery`] whose field
//! references are resolved and whose filter literals are typed.
//!
//! Validation is the first line of defence on the server
//! (plan/spezifikation/07-server.md §Sicherheit): nothing downstream has to
//! re-check field existence, operator suitability or literal types.

use opengrid_json::Json as JsonValue;
use opengrid_types::{DataSourceId, DataType, Field, FieldName, Schema, Value as GridValue};

use crate::{Aggregate, CmpOp, FilterExpr, Query, QueryError, Sort, TreeSpec};

/// Server- and client-side guards applied during validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest accepted `limit`.
    pub max_limit: u64,
    /// Largest accepted filter nesting depth.
    pub max_depth: usize,
}

impl Limits {
    /// Explicit limits.
    pub fn new(max_limit: u64, max_depth: usize) -> Self {
        Self {
            max_limit,
            max_depth,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_limit: 10_000,
            max_depth: 8,
        }
    }
}

/// A query that passed [`validate`](Query::validate): every field exists, every
/// operator fits its type and every filter literal is a typed [`GridValue`].
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedQuery {
    pub source: DataSourceId,
    pub select: Vec<FieldName>,
    pub filter: Option<ValidatedFilter>,
    pub group: Vec<FieldName>,
    pub aggregate: Vec<Aggregate>,
    pub sort: Vec<Sort>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
    /// One level of a tree (E38), its `under` read as the key's type.
    pub tree: Option<ValidatedTree>,
    /// Columns the query produces, in order, with resolved types.
    pub output_schema: Schema,
}

/// The tree part of a validated query: the key and parent fields exist and
/// share a type, and `under` is a value of it — `None` asks for the roots.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedTree {
    pub key: FieldName,
    pub parent: FieldName,
    pub under: Option<GridValue>,
    /// The rows the tree consists of (see [`TreeSpec::scope`]).
    pub scope: Option<ValidatedFilter>,
    /// The subtree aggregates (rule T7), as the query gave them.
    pub aggregate: Vec<Aggregate>,
    /// Their aliases with the result types of S12, in query order.
    pub aggregate_schema: Schema,
    /// The whole tree, flat (T8), rather than one level.
    pub flat: bool,
}

/// A filter whose literals have been read against the field types.
#[derive(Clone, Debug, PartialEq)]
pub enum ValidatedFilter {
    And(Vec<ValidatedFilter>),
    Or(Vec<ValidatedFilter>),
    Not(Box<ValidatedFilter>),
    Cmp {
        field: FieldName,
        data_type: DataType,
        op: CmpOp,
        value: GridValue,
    },
    InList {
        field: FieldName,
        data_type: DataType,
        values: Vec<GridValue>,
    },
    IsNull {
        field: FieldName,
    },
    IsNotNull {
        field: FieldName,
    },
}

impl Query {
    /// Validates the query against a schema and the configured limits.
    pub fn validate(&self, schema: &Schema, limits: &Limits) -> Result<ValidatedQuery, QueryError> {
        if let Some(filter) = &self.filter {
            let depth = filter.depth();
            if depth > limits.max_depth {
                return Err(QueryError::FilterTooDeep {
                    depth,
                    max_depth: limits.max_depth,
                });
            }
        }

        for (i, field) in self.group.iter().enumerate() {
            if schema.field(field.as_str()).is_none() {
                return Err(QueryError::UnknownField {
                    path: format!("group[{i}]"),
                    field: field.to_string(),
                });
            }
        }

        // Resolve aggregates: result type and alias bookkeeping.
        let aliases = resolve_aggregates(&self.aggregate, schema, "aggregate")?;

        let output = self.output_fields(schema, &aliases)?;

        for (i, sort) in self.sort.iter().enumerate() {
            if !output.iter().any(|field| field.name == sort.field) {
                return Err(QueryError::SortUnknownColumn {
                    path: format!("sort[{i}].field"),
                    column: sort.field.to_string(),
                });
            }
        }

        if self.offset.is_some() && self.sort.is_empty() {
            return Err(QueryError::OffsetWithoutSort);
        }

        if let Some(limit) = self.limit
            && limit > limits.max_limit
        {
            return Err(QueryError::LimitTooLarge {
                limit,
                max_limit: limits.max_limit,
            });
        }

        let filter = match &self.filter {
            Some(filter) => Some(validate_filter(filter, schema, "filter")?),
            None => None,
        };

        let tree = match &self.tree {
            None => None,
            Some(tree) => {
                if !self.group.is_empty() || !self.aggregate.is_empty() {
                    return Err(QueryError::TreeWithGroup);
                }
                let key = field_type(schema, &tree.key, "tree.key")?;
                let parent = field_type(schema, &tree.parent, "tree.parent")?;
                if key != parent {
                    return Err(QueryError::TreeParentTypeMismatch { key, parent });
                }
                // T7: the subtree's aggregates, typed by the same rules as a
                // group's (S12), under their own path.
                let aggregate_schema = Schema::new(
                    resolve_aggregates(&tree.aggregate, schema, "tree.aggregate")?
                        .into_iter()
                        .map(|(alias, data_type, nullable)| {
                            if nullable {
                                Field::new(alias, data_type)
                            } else {
                                Field::required(alias, data_type)
                            }
                        })
                        .collect(),
                );
                // T8: the flat tree is the whole tree, with columns of its own.
                if tree.flat {
                    if tree.under.is_some() {
                        return Err(QueryError::TreeFlatWithUnder);
                    }
                    if !tree.aggregate.is_empty() {
                        return Err(QueryError::TreeFlatWithAggregate);
                    }
                    let added: &[&str] = if self.filter.is_some() {
                        &FLAT_COLUMNS
                    } else {
                        &FLAT_COLUMNS[..2]
                    };
                    if let Some(column) = added.iter().find(|name| schema.field(name).is_some()) {
                        return Err(QueryError::TreeFlatColumnTaken {
                            column: (*column).to_owned(),
                        });
                    }
                }
                Some(ValidatedTree {
                    key: tree.key.clone(),
                    parent: tree.parent.clone(),
                    aggregate: tree.aggregate.clone(),
                    aggregate_schema,
                    flat: tree.flat,
                    under: match &tree.under {
                        Some(under) => Some(coerce(under, key, "tree.under")?),
                        None => None,
                    },
                    scope: match &tree.scope {
                        Some(scope) => Some(validate_filter(scope, schema, "tree.scope")?),
                        None => None,
                    },
                })
            }
        };

        Ok(ValidatedQuery {
            source: self.source.clone(),
            select: self.select.clone(),
            filter,
            group: self.group.clone(),
            aggregate: self.aggregate.clone(),
            sort: self.sort.clone(),
            offset: self.offset,
            limit: self.limit,
            tree,
            output_schema: Schema::new(output),
        })
    }

    /// Builds the ordered output columns, enforcing the grouping rule.
    fn output_fields(
        &self,
        schema: &Schema,
        aliases: &[(FieldName, DataType, bool)],
    ) -> Result<Vec<Field>, QueryError> {
        let aggregate_query = !self.group.is_empty() || !self.aggregate.is_empty();
        let mut output: Vec<Field> = Vec::new();

        if aggregate_query {
            // Every selected field is either a group key or an aggregate alias.
            for (i, field) in self.select.iter().enumerate() {
                if self.group.contains(field) {
                    let source = schema.field(field.as_str()).expect("group checked");
                    output.push(Field {
                        name: field.clone(),
                        data_type: source.data_type,
                        nullable: source.nullable,
                        // An output column is never derived: the result carries
                        // its values, whatever produced them (point 54).
                        from: None,
                    });
                } else if let Some((alias, data_type, nullable)) =
                    aliases.iter().find(|(alias, _, _)| alias == field)
                {
                    output.push(Field {
                        name: alias.clone(),
                        data_type: *data_type,
                        nullable: *nullable,
                        from: None,
                    });
                } else {
                    return Err(QueryError::SelectNotGrouped {
                        path: format!("select[{i}]"),
                        field: field.to_string(),
                    });
                }
            }
            // Aggregates not named in `select` follow in declaration order.
            for (alias, data_type, nullable) in aliases {
                if !self.select.contains(alias) {
                    output.push(Field {
                        name: alias.clone(),
                        data_type: *data_type,
                        nullable: *nullable,
                        from: None,
                    });
                }
            }
        } else {
            for (i, field) in self.select.iter().enumerate() {
                let source =
                    schema
                        .field(field.as_str())
                        .ok_or_else(|| QueryError::UnknownField {
                            path: format!("select[{i}]"),
                            field: field.to_string(),
                        })?;
                output.push(Field {
                    name: field.clone(),
                    data_type: source.data_type,
                    nullable: source.nullable,
                    from: None,
                });
            }
        }

        if output.is_empty() {
            return Err(QueryError::EmptyProjection);
        }

        for j in 1..output.len() {
            if output[..j].iter().any(|field| field.name == output[j].name) {
                return Err(QueryError::DuplicateOutputColumn {
                    path: format!("select[{j}]"),
                    column: output[j].name.to_string(),
                });
            }
        }

        Ok(output)
    }
}

fn validate_filter(
    expr: &FilterExpr,
    schema: &Schema,
    path: &str,
) -> Result<ValidatedFilter, QueryError> {
    match expr {
        FilterExpr::And(items) => Ok(ValidatedFilter::And(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| validate_filter(item, schema, &format!("{path}.and[{i}]")))
                .collect::<Result<_, _>>()?,
        )),
        FilterExpr::Or(items) => Ok(ValidatedFilter::Or(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| validate_filter(item, schema, &format!("{path}.or[{i}]")))
                .collect::<Result<_, _>>()?,
        )),
        FilterExpr::Not(inner) => Ok(ValidatedFilter::Not(Box::new(validate_filter(
            inner,
            schema,
            &format!("{path}.not"),
        )?))),
        FilterExpr::IsNull { field } => {
            field_type(schema, field, &format!("{path}.field"))?;
            Ok(ValidatedFilter::IsNull {
                field: field.clone(),
            })
        }
        FilterExpr::IsNotNull { field } => {
            field_type(schema, field, &format!("{path}.field"))?;
            Ok(ValidatedFilter::IsNotNull {
                field: field.clone(),
            })
        }
        FilterExpr::Cmp { field, op, value } => {
            let data_type = field_type(schema, field, &format!("{path}.field"))?;
            match op {
                CmpOp::In => {
                    let JsonValue::Array(items) = value else {
                        return Err(QueryError::ListExpected {
                            path: format!("{path}.value"),
                        });
                    };
                    let values = items
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let item_path = format!("{path}.value[{i}]");
                            if item.is_null() {
                                return Err(QueryError::NullInList { path: item_path });
                            }
                            coerce(item, data_type, &item_path)
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(ValidatedFilter::InList {
                        field: field.clone(),
                        data_type,
                        values,
                    })
                }
                CmpOp::Contains | CmpOp::StartsWith => {
                    if data_type != DataType::Utf8 {
                        return Err(QueryError::OperatorNotSupported {
                            path: format!("{path}.op"),
                            op: *op,
                            data_type,
                        });
                    }
                    let value = coerce(value, data_type, &format!("{path}.value"))?;
                    Ok(ValidatedFilter::Cmp {
                        field: field.clone(),
                        data_type,
                        op: *op,
                        value,
                    })
                }
                _ => {
                    let value = coerce(value, data_type, &format!("{path}.value"))?;
                    Ok(ValidatedFilter::Cmp {
                        field: field.clone(),
                        data_type,
                        op: *op,
                        value,
                    })
                }
            }
        }
    }
}

/// The columns a flat tree adds after the query's own (T8): its level, its
/// path of keys, and — with a filter — whether the row matches.
pub const FLAT_COLUMNS: [&str; 3] = ["level", "path", "match"];

/// Resolves a list of aggregates against `schema`: the result type of each
/// (S12), whether it can be NULL, and its alias — unique, and no field's name.
/// `path` is where the list sits in the query (`aggregate`, `tree.aggregate`).
fn resolve_aggregates(
    aggregates: &[Aggregate],
    schema: &Schema,
    path: &str,
) -> Result<Vec<(FieldName, DataType, bool)>, QueryError> {
    let mut aliases: Vec<(FieldName, DataType, bool)> = Vec::new();
    for (i, aggregate) in aggregates.iter().enumerate() {
        let input_type =
            match &aggregate.field {
                Some(field) => Some(schema.data_type(field.as_str()).ok_or_else(|| {
                    QueryError::UnknownField {
                        path: format!("{path}[{i}].field"),
                        field: field.to_string(),
                    }
                })?),
                None => {
                    if aggregate.function.requires_field() {
                        return Err(QueryError::AggregateFieldRequired {
                            path: format!("{path}[{i}].fn"),
                            function: aggregate.function,
                        });
                    }
                    None
                }
            };
        let result =
            match input_type {
                Some(input) => aggregate.function.result_type(input).ok_or(
                    QueryError::AggregateTypeMismatch {
                        path: format!("{path}[{i}].fn"),
                        function: aggregate.function,
                        data_type: input,
                    },
                )?,
                None => DataType::Int64,
            };
        if schema.field(aggregate.alias.as_str()).is_some() {
            return Err(QueryError::AliasConflictsWithField {
                path: format!("{path}[{i}].as"),
                alias: aggregate.alias.to_string(),
            });
        }
        if aliases
            .iter()
            .any(|(alias, _, _)| alias == &aggregate.alias)
        {
            return Err(QueryError::DuplicateAlias {
                path: format!("{path}[{i}].as"),
                alias: aggregate.alias.to_string(),
            });
        }
        let nullable = !matches!(aggregate.function, crate::AggregateFn::Count);
        aliases.push((aggregate.alias.clone(), result, nullable));
    }
    Ok(aliases)
}

fn field_type(schema: &Schema, field: &FieldName, path: &str) -> Result<DataType, QueryError> {
    schema
        .data_type(field.as_str())
        .ok_or_else(|| QueryError::UnknownField {
            path: path.to_owned(),
            field: field.to_string(),
        })
}

/// Reads a raw JSON literal as the given type.
fn coerce(value: &JsonValue, data_type: DataType, path: &str) -> Result<GridValue, QueryError> {
    GridValue::from_json_typed(value, &data_type).map_err(|error| QueryError::ValueTypeMismatch {
        path: path.to_owned(),
        data_type,
        message: error.to_string(),
    })
}

/// A validated query, written back as the JSON contract it came from.
///
/// Validation adds knowledge (the type of every literal, the output schema); it
/// throws nothing away. So the way back is total — which is what makes a plan
/// transportable: the planner splits a [`ValidatedQuery`] in two, and one half
/// has to travel to a server as ordinary query JSON (plan point 28).
impl From<&ValidatedQuery> for Query {
    fn from(query: &ValidatedQuery) -> Self {
        Query {
            source: query.source.clone(),
            select: query.select.clone(),
            filter: query.filter.as_ref().map(FilterExpr::from),
            group: query.group.clone(),
            aggregate: query.aggregate.clone(),
            sort: query.sort.clone(),
            offset: query.offset,
            limit: query.limit,
            tree: query.tree.as_ref().map(|tree| TreeSpec {
                key: tree.key.clone(),
                parent: tree.parent.clone(),
                under: tree.under.as_ref().map(opengrid_json::ToJson::to_json),
                scope: tree.scope.as_ref().map(FilterExpr::from),
                aggregate: tree.aggregate.clone(),
                flat: tree.flat,
            }),
        }
    }
}

impl From<&ValidatedFilter> for FilterExpr {
    fn from(filter: &ValidatedFilter) -> Self {
        /// The literal as the contract writes it — the same notation the wire
        /// format uses, since both go through `Value`'s own serialization.
        fn literal(value: &GridValue) -> JsonValue {
            opengrid_json::ToJson::to_json(value)
        }

        match filter {
            ValidatedFilter::And(parts) => FilterExpr::And(parts.iter().map(Self::from).collect()),
            ValidatedFilter::Or(parts) => FilterExpr::Or(parts.iter().map(Self::from).collect()),
            ValidatedFilter::Not(inner) => FilterExpr::Not(Box::new(Self::from(inner.as_ref()))),
            ValidatedFilter::Cmp {
                field, op, value, ..
            } => FilterExpr::Cmp {
                field: field.clone(),
                op: *op,
                value: literal(value),
            },
            ValidatedFilter::InList { field, values, .. } => FilterExpr::Cmp {
                field: field.clone(),
                op: CmpOp::In,
                value: JsonValue::Array(values.iter().map(literal).collect()),
            },
            ValidatedFilter::IsNull { field } => FilterExpr::IsNull {
                field: field.clone(),
            },
            ValidatedFilter::IsNotNull { field } => FilterExpr::IsNotNull {
                field: field.clone(),
            },
        }
    }
}
