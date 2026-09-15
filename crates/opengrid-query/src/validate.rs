//! Validation: turns a [`Query`] into a [`ValidatedQuery`] whose field
//! references are resolved and whose filter literals are typed.
//!
//! Validation is the first line of defence on the server
//! (plan/spezifikation/07-server.md §Sicherheit): nothing downstream has to
//! re-check field existence, operator suitability or literal types.

use opengrid_types::{DataSourceId, DataType, Field, FieldName, Schema, Value as GridValue};
use serde_json::Value as JsonValue;

use crate::{Aggregate, CmpOp, FilterExpr, Query, QueryError, Sort};

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
    /// Columns the query produces, in order, with resolved types.
    pub output_schema: Schema,
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
        let mut aliases: Vec<(FieldName, DataType, bool)> = Vec::new();
        for (i, aggregate) in self.aggregate.iter().enumerate() {
            let input_type = match &aggregate.field {
                Some(field) => Some(schema.data_type(field.as_str()).ok_or_else(|| {
                    QueryError::UnknownField {
                        path: format!("aggregate[{i}].field"),
                        field: field.to_string(),
                    }
                })?),
                None => {
                    if aggregate.function.requires_field() {
                        return Err(QueryError::AggregateFieldRequired {
                            path: format!("aggregate[{i}].fn"),
                            function: aggregate.function,
                        });
                    }
                    None
                }
            };
            let result = match input_type {
                Some(input) => aggregate.function.result_type(input).ok_or(
                    QueryError::AggregateTypeMismatch {
                        path: format!("aggregate[{i}].fn"),
                        function: aggregate.function,
                        data_type: input,
                    },
                )?,
                None => DataType::Int64,
            };
            if schema.field(aggregate.alias.as_str()).is_some() {
                return Err(QueryError::AliasConflictsWithField {
                    path: format!("aggregate[{i}].as"),
                    alias: aggregate.alias.to_string(),
                });
            }
            if aliases
                .iter()
                .any(|(alias, _, _)| alias == &aggregate.alias)
            {
                return Err(QueryError::DuplicateAlias {
                    path: format!("aggregate[{i}].as"),
                    alias: aggregate.alias.to_string(),
                });
            }
            let nullable = !matches!(aggregate.function, crate::AggregateFn::Count);
            aliases.push((aggregate.alias.clone(), result, nullable));
        }

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

        Ok(ValidatedQuery {
            source: self.source.clone(),
            select: self.select.clone(),
            filter,
            group: self.group.clone(),
            aggregate: self.aggregate.clone(),
            sort: self.sort.clone(),
            offset: self.offset,
            limit: self.limit,
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
                    });
                } else if let Some((alias, data_type, nullable)) =
                    aliases.iter().find(|(alias, _, _)| alias == field)
                {
                    output.push(Field {
                        name: alias.clone(),
                        data_type: *data_type,
                        nullable: *nullable,
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
    GridValue::deserialize_typed(value.clone(), &data_type).map_err(|error| {
        QueryError::ValueTypeMismatch {
            path: path.to_owned(),
            data_type,
            message: error.to_string(),
        }
    })
}
