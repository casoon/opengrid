//! The pivot query, its limits, and the decomposition into grouping sets.

use opengrid_query::{
    Aggregate, Collation, FilterExpr, Limits, NullsOrder, Query, QueryError, Sort, SortDirection,
    ValidatedQuery,
};
use opengrid_types::{DataSourceId, FieldName, Schema};
use serde::{Deserialize, Serialize};

/// What to pivot: rows down the side, columns across the top, measures inside.
///
/// Deliberately close to `Query` — the filter is the very same [`FilterExpr`],
/// and a measure **is** an [`Aggregate`], not a parallel type. A pivot is a way
/// of arranging a query's answer, not a second query language.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PivotQuery {
    pub source: DataSourceId,
    /// Dimensions down the side, outermost first.
    #[serde(default)]
    pub rows: Vec<FieldName>,
    /// Dimensions across the top. V1 allows at most one (see [`PivotLimits`]).
    #[serde(default)]
    pub columns: Vec<FieldName>,
    /// The measures in each cell, in display order.
    pub values: Vec<Aggregate>,
    #[serde(default)]
    pub filter: Option<FilterExpr>,
}

/// The bounds a pivot must stay inside (plan point 30).
///
/// Exceeding one is an **error**, never a shortened answer: a pivot missing
/// columns shows totals that do not add up, and nobody can see that it is
/// missing anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PivotLimits {
    /// How many dimensions may go across the top. V1: one.
    pub max_column_dimensions: usize,
    /// How many generated leaf columns the answer may have.
    pub max_columns: usize,
    /// How many rows the answer may have, subtotals included.
    pub max_rows: usize,
}

impl Default for PivotLimits {
    fn default() -> Self {
        Self {
            max_column_dimensions: 1,
            max_columns: 256,
            max_rows: 2000,
        }
    }
}

/// A pivot query that is known to fit its schema, with its decomposition.
///
/// The grouping sets are part of the value on purpose: like `ExecutionPlan` in
/// the planner, the decision is inspectable before anything runs.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidatedPivotQuery {
    pub source: DataSourceId,
    pub rows: Vec<FieldName>,
    pub columns: Vec<FieldName>,
    pub values: Vec<Aggregate>,
    /// One query per level, **deepest first**: `[r1..rn, c]`, …, `[c]`.
    pub sets: Vec<ValidatedQuery>,
    pub limits: PivotLimits,
}

/// Why a pivot cannot be answered.
#[derive(Clone, Debug, PartialEq)]
pub enum PivotError {
    /// A dimension or measure does not fit the schema — the query validator's
    /// own diagnosis, for the grouping set it broke.
    Query(QueryError),
    /// A pivot without measures has nothing in its cells.
    NoMeasures,
    /// More dimensions across the top than V1 allows.
    TooManyColumnDimensions { found: usize, maximum: usize },
    /// The same dimension twice, or a dimension that is also a measure alias.
    DuplicateDimension { field: String },
    /// The answer would be wider than [`PivotLimits::max_columns`].
    TooManyColumns { found: usize, maximum: usize },
    /// The answer would be longer than [`PivotLimits::max_rows`].
    TooManyRows { found: usize, maximum: usize },
}

impl std::fmt::Display for PivotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PivotError::Query(error) => write!(f, "{error}"),
            PivotError::NoMeasures => f.write_str("a pivot needs at least one measure"),
            PivotError::TooManyColumnDimensions { found, maximum } => {
                write!(f, "{found} column dimensions, the maximum is {maximum}")
            }
            PivotError::DuplicateDimension { field } => {
                write!(f, "dimension {field:?} is named more than once")
            }
            PivotError::TooManyColumns { found, maximum } => write!(
                f,
                "the pivot would have {found} columns, the maximum is {maximum} — \
                 narrow the column dimension or the filter"
            ),
            PivotError::TooManyRows { found, maximum } => write!(
                f,
                "the pivot would have {found} rows, the maximum is {maximum} — \
                 narrow the filter"
            ),
        }
    }
}

impl std::error::Error for PivotError {}

impl From<QueryError> for PivotError {
    fn from(error: QueryError) -> Self {
        PivotError::Query(error)
    }
}

impl PivotQuery {
    /// Checks the pivot against `schema` and decomposes it.
    pub fn validate(
        &self,
        schema: &Schema,
        limits: &PivotLimits,
        query_limits: &Limits,
    ) -> Result<ValidatedPivotQuery, PivotError> {
        if self.values.is_empty() {
            return Err(PivotError::NoMeasures);
        }
        if self.columns.len() > limits.max_column_dimensions {
            return Err(PivotError::TooManyColumnDimensions {
                found: self.columns.len(),
                maximum: limits.max_column_dimensions,
            });
        }

        // A dimension twice would mean the same key at two depths; a dimension
        // that is also a measure alias would be two different columns with one
        // name. Both are caught here rather than in a confusing output schema.
        let mut seen: Vec<&FieldName> = Vec::new();
        for field in self.rows.iter().chain(&self.columns) {
            if seen.contains(&field) || self.values.iter().any(|value| value.alias == *field) {
                return Err(PivotError::DuplicateDimension {
                    field: field.to_string(),
                });
            }
            seen.push(field);
        }

        let sets = grouping_sets(self, schema, query_limits)?;
        Ok(ValidatedPivotQuery {
            source: self.source.clone(),
            rows: self.rows.clone(),
            columns: self.columns.clone(),
            values: self.values.clone(),
            sets,
            limits: *limits,
        })
    }
}

/// The `n+1` ordinary queries a pivot is made of, **deepest first**.
///
/// Level `k` groups by the first `k` row dimensions plus every column dimension.
/// The last one — `k = 0` — groups by the column dimensions alone and therefore
/// does double duty: it is the grand total row, and, because it is sorted, it is
/// also the catalogue of column values in S3/S4 order. That is why the pivot
/// engine never has to sort anything itself.
///
/// Every set is sorted by its own group keys, ascending, `NULLS LAST` (S3).
pub fn grouping_sets(
    pivot: &PivotQuery,
    schema: &Schema,
    limits: &Limits,
) -> Result<Vec<ValidatedQuery>, PivotError> {
    let mut sets = Vec::with_capacity(pivot.rows.len() + 1);
    for depth in (0..=pivot.rows.len()).rev() {
        let group: Vec<FieldName> = pivot.rows[..depth]
            .iter()
            .chain(&pivot.columns)
            .cloned()
            .collect();

        // Group keys first, then the measures: the output schema of every set
        // has the same shape, which is what the assembly reads.
        let select: Vec<FieldName> = group
            .iter()
            .cloned()
            .chain(pivot.values.iter().map(|value| value.alias.clone()))
            .collect();
        let sort: Vec<Sort> = group
            .iter()
            .map(|field| Sort {
                field: field.clone(),
                direction: SortDirection::Asc,
                nulls: NullsOrder::default(),
                collation: Collation::default(),
            })
            .collect();

        let query = Query {
            source: pivot.source.clone(),
            select,
            filter: pivot.filter.clone(),
            group,
            aggregate: pivot.values.clone(),
            sort,
            offset: None,
            limit: None,
        };
        sets.push(query.validate(schema, limits)?);
    }
    Ok(sets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::{DataType, Field};

    fn name(raw: &str) -> FieldName {
        FieldName::new(raw).expect("a test field name")
    }

    fn schema() -> Schema {
        Schema::new(vec![
            Field::required(name("id"), DataType::Int64),
            Field::new(name("country"), DataType::Utf8),
            Field::new(name("customer"), DataType::Utf8),
            Field::new(name("qty"), DataType::Int64),
        ])
    }

    fn pivot(json: &str) -> PivotQuery {
        serde_json::from_str(json).expect("a pivot")
    }

    fn validate(json: &str) -> Result<ValidatedPivotQuery, PivotError> {
        pivot(json).validate(&schema(), &PivotLimits::default(), &Limits::default())
    }

    const SIMPLE: &str = r#"{"source":"orders","rows":["country"],
        "values":[{"fn":"count","as":"n"}]}"#;

    /// P1: `n` row dimensions decompose into `n+1` levels, deepest first, each
    /// one an ordinary query.
    #[test]
    fn a_pivot_is_a_set_of_grouping_sets() {
        let plan = validate(
            r#"{"source":"orders","rows":["country","customer"],"columns":["qty"],
                "values":[{"fn":"count","as":"n"}]}"#,
        )
        .expect("valid");
        assert_eq!(plan.sets.len(), 3);
        assert_eq!(plan.sets[0].group.len(), 3);
        assert_eq!(
            plan.sets[2].group.len(),
            1,
            "the grand total keeps the columns"
        );
        // No paging anywhere: a level is the whole level (S6 would forbid the
        // offset anyway, but the point is that a pivot is not a page).
        assert!(plan.sets.iter().all(|set| set.limit.is_none()));
    }

    /// V1 allows one dimension across the top. The engine is generic; this is a
    /// limit, so lifting it later is a number, not a rewrite.
    #[test]
    fn more_than_one_column_dimension_is_refused() {
        let error = validate(
            r#"{"source":"orders","rows":["country"],"columns":["customer","qty"],
                "values":[{"fn":"count","as":"n"}]}"#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            PivotError::TooManyColumnDimensions {
                found: 2,
                maximum: 1
            }
        ));
    }

    /// A pivot without measures has nothing in its cells.
    #[test]
    fn a_pivot_needs_a_measure() {
        assert_eq!(
            validate(r#"{"source":"orders","rows":["country"],"values":[]}"#).unwrap_err(),
            PivotError::NoMeasures
        );
    }

    /// The same name twice would be two different columns with one name.
    #[test]
    fn a_dimension_is_named_once() {
        assert!(matches!(
            validate(
                r#"{"source":"orders","rows":["country","country"],
                    "values":[{"fn":"count","as":"n"}]}"#
            )
            .unwrap_err(),
            PivotError::DuplicateDimension { .. }
        ));
        assert!(
            matches!(
                validate(
                    r#"{"source":"orders","rows":["country"],
                        "values":[{"fn":"count","as":"country"}]}"#
                )
                .unwrap_err(),
                PivotError::DuplicateDimension { .. }
            ),
            "a dimension that is also a measure alias is the same collision"
        );
    }

    /// A dimension the schema does not have fails with the query validator's own
    /// diagnosis — there is no second set of error messages.
    #[test]
    fn an_unknown_dimension_reports_the_validators_reason() {
        let error =
            validate(r#"{"source":"orders","rows":["nope"],"values":[{"fn":"count","as":"n"}]}"#)
                .unwrap_err();
        assert!(
            error.to_string().contains("nope"),
            "the diagnosis names it: {error}"
        );
    }

    /// The pivot query round-trips through its JSON form.
    #[test]
    fn a_pivot_reads_and_writes_itself() {
        let pivot = pivot(SIMPLE);
        let json = serde_json::to_string(&pivot).expect("JSON");
        assert_eq!(serde_json::from_str::<PivotQuery>(&json).unwrap(), pivot);
    }
}
