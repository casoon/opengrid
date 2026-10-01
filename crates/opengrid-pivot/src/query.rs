//! The pivot query, its limits, and the decomposition into grouping sets.

use opengrid_json::{Error, Fields, FromJson, Json, ToJson};
use opengrid_query::{
    Aggregate, Collation, FilterExpr, Limits, NullsOrder, Query, QueryError, Sort, SortDirection,
    ValidatedQuery,
};
use opengrid_types::{DataSourceId, FieldName, Schema};

/// What to pivot: rows down the side, columns across the top, measures inside.
///
/// Deliberately close to `Query` — the filter is the very same [`FilterExpr`],
/// and a measure **is** an [`Aggregate`], not a parallel type. A pivot is a way
/// of arranging a query's answer, not a second query language.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotQuery {
    pub source: DataSourceId,
    /// Dimensions down the side, outermost first.
    pub rows: Vec<FieldName>,
    /// Dimensions across the top. V1 allows at most two (see [`PivotLimits`]).
    pub columns: Vec<FieldName>,
    /// The measures in each cell, in display order.
    pub values: Vec<Aggregate>,
    pub filter: Option<FilterExpr>,
    /// How the rows of a level are ordered among their siblings (rule P9).
    /// A level without an entry is ascending by its own values.
    pub sort: Vec<PivotSort>,
}

/// The order of one level's rows among their siblings (rule P9).
///
/// `field` names the row dimension — the level. Without `by` the level is
/// ordered by its own values; `by` names a measure alias, and the level is
/// ordered by that measure **over the whole row**: across every column value,
/// computed from the raw rows (P5).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotSort {
    pub field: FieldName,
    pub by: Option<FieldName>,
    pub direction: SortDirection,
}

impl FromJson for PivotSort {
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct PivotSort", &["field", "by", "direction"])?;
        Ok(PivotSort {
            field: fields.read("field")?,
            by: fields.read_optional("by")?,
            direction: fields.read_or_default("direction")?,
        })
    }
}

impl ToJson for PivotSort {
    fn to_json(&self) -> Json {
        match &self.by {
            Some(by) => opengrid_json::json!({
                "field": self.field,
                "by": by,
                "direction": self.direction,
            }),
            None => opengrid_json::json!({
                "field": self.field,
                "direction": self.direction,
            }),
        }
    }
}

impl FromJson for PivotQuery {
    /// `{ "source", "rows", "columns", "values", "filter", "sort" }`; only
    /// `source` and `values` are required.
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(
            json,
            "struct PivotQuery",
            &["source", "rows", "columns", "values", "filter", "sort"],
        )?;
        Ok(PivotQuery {
            source: fields.read("source")?,
            rows: fields.read_or_default("rows")?,
            columns: fields.read_or_default("columns")?,
            values: fields.read("values")?,
            filter: fields.read_optional("filter")?,
            sort: fields.read_or_default("sort")?,
        })
    }
}

impl ToJson for PivotQuery {
    /// `sort` only when there is one: a server from before rule P9 reads a
    /// pivot without it as it always did.
    fn to_json(&self) -> Json {
        let mut json = opengrid_json::json!({
            "source": self.source,
            "rows": self.rows,
            "columns": self.columns,
            "values": self.values,
            "filter": self.filter,
        });
        if !self.sort.is_empty()
            && let Json::Object(object) = &mut json
        {
            object.insert("sort".to_owned(), self.sort.to_json());
        }
        json
    }
}

/// The bounds a pivot must stay inside (plan point 30).
///
/// Exceeding one is an **error**, never a shortened answer: a pivot missing
/// columns shows totals that do not add up, and nobody can see that it is
/// missing anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PivotLimits {
    /// How many dimensions may go across the top. V1: two (E20, revised by
    /// issue #120 — the header stays two levels of values).
    pub max_column_dimensions: usize,
    /// How many generated leaf columns the answer may have.
    pub max_columns: usize,
    /// How many rows the answer may have, subtotals included.
    pub max_rows: usize,
    /// How many cells — rows times generated columns — the answer may have
    /// (E39, issue #122). A browser draws a native table in time proportional
    /// to its cells, whatever their shape; this is the budget that keeps a
    /// pivot drawn in about a second.
    pub max_cells: usize,
}

impl Default for PivotLimits {
    fn default() -> Self {
        Self {
            max_column_dimensions: 2,
            max_columns: 256,
            max_rows: 10_000,
            max_cells: 131_072,
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
    /// The levels ordered by a measure while there is a column dimension:
    /// `(depth, query)`, the query grouped by `r1..r_depth` alone and sorted
    /// by the measure — the row's value over every column (rule P9). Their
    /// answers follow the sets' in [`crate::assemble`]'s input.
    pub orders: Vec<(usize, ValidatedQuery)>,
    /// Rule P9 as asked; empty is every level ascending by its values.
    pub sort: Vec<PivotSort>,
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
    /// The answer would have more cells than [`PivotLimits::max_cells`].
    TooManyCells {
        rows: usize,
        columns: usize,
        maximum: usize,
    },
    /// A sort that names no row dimension, a level twice, or a measure the
    /// pivot does not have.
    Sort { reason: String },
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
            PivotError::TooManyCells {
                rows,
                columns,
                maximum,
            } => write!(
                f,
                "the pivot would have {rows} rows by {columns} columns, {} cells; the \
                 maximum is {maximum} — narrow the filter, or choose fewer dimensions",
                rows * columns
            ),
            PivotError::Sort { reason } => write!(f, "sort: {reason}"),
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

        for (index, sort) in self.sort.iter().enumerate() {
            let problem = if !self.rows.contains(&sort.field) {
                Some(format!("{} is not a row dimension", sort.field))
            } else if self.sort[..index]
                .iter()
                .any(|earlier| earlier.field == sort.field)
            {
                Some(format!("{} is sorted twice", sort.field))
            } else {
                sort.by
                    .as_ref()
                    .filter(|by| !self.values.iter().any(|value| value.alias == **by))
                    .map(|by| format!("{by} is not a measure of this pivot"))
            };
            if let Some(reason) = problem {
                return Err(PivotError::Sort { reason });
            }
        }

        let sets = grouping_sets(self, schema, query_limits)?;
        let orders = order_sets(self, schema, query_limits)?;
        Ok(ValidatedPivotQuery {
            source: self.source.clone(),
            rows: self.rows.clone(),
            columns: self.columns.clone(),
            values: self.values.clone(),
            sets,
            orders,
            sort: self.sort.clone(),
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
/// Every set is sorted by its own group keys, ascending, `NULLS LAST` (S3) —
/// except where rule P9 orders a level: its own dimension descending, or its
/// measure first when there is no column dimension. The order of the siblings
/// of a level is the order in which its set names them, so this is all the
/// sorting a pivot needs.
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
        let level = depth.checked_sub(1).and_then(|last| sort_of(pivot, last));
        let mut sort: Vec<Sort> = Vec::new();
        if let Some(PivotSort {
            by: Some(by),
            direction,
            ..
        }) = level
            && pivot.columns.is_empty()
        {
            sort.push(key(by, *direction));
        }
        sort.extend(group.iter().enumerate().map(|(index, field)| {
            let direction = match level {
                Some(PivotSort {
                    by: None,
                    direction,
                    ..
                }) if index + 1 == depth => *direction,
                _ => SortDirection::Asc,
            };
            key(field, direction)
        }));

        let query = Query {
            source: pivot.source.clone(),
            select,
            filter: pivot.filter.clone(),
            group,
            aggregate: pivot.values.clone(),
            sort,
            offset: None,
            limit: None,
            tree: None,
        };
        sets.push(query.validate(schema, limits)?);
    }
    Ok(sets)
}

/// The entry of rule P9 for the level at `index` (0 is the outermost).
fn sort_of(pivot: &PivotQuery, index: usize) -> Option<&PivotSort> {
    let field = pivot.rows.get(index)?;
    pivot.sort.iter().find(|sort| sort.field == *field)
}

fn key(field: &FieldName, direction: SortDirection) -> Sort {
    Sort {
        field: field.clone(),
        direction,
        nulls: NullsOrder::default(),
        collation: Collation::default(),
    }
}

/// The extra queries of rule P9: a level ordered by a measure while there is
/// a column dimension. Its set holds one row per column value, so the order
/// of the row **as a whole** needs the level grouped by its row dimensions
/// alone — sorted by the measure, ties by the dimensions ascending.
fn order_sets(
    pivot: &PivotQuery,
    schema: &Schema,
    limits: &Limits,
) -> Result<Vec<(usize, ValidatedQuery)>, PivotError> {
    if pivot.columns.is_empty() {
        return Ok(Vec::new());
    }
    let mut orders = Vec::new();
    for depth in 1..=pivot.rows.len() {
        let Some(PivotSort {
            by: Some(by),
            direction,
            ..
        }) = sort_of(pivot, depth - 1)
        else {
            continue;
        };
        let group: Vec<FieldName> = pivot.rows[..depth].to_vec();
        let query = Query {
            source: pivot.source.clone(),
            select: group
                .iter()
                .cloned()
                .chain(pivot.values.iter().map(|value| value.alias.clone()))
                .collect(),
            filter: pivot.filter.clone(),
            group: group.clone(),
            aggregate: pivot.values.clone(),
            sort: std::iter::once(key(by, *direction))
                .chain(group.iter().map(|field| key(field, SortDirection::Asc)))
                .collect(),
            offset: None,
            limit: None,
            tree: None,
        };
        orders.push((depth, query.validate(schema, limits)?));
    }
    Ok(orders)
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
        opengrid_json::from_str(json).expect("a pivot")
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

    /// V1 allows two dimensions across the top (#120). The engine is generic;
    /// this is a limit, so lifting it was a number, not a rewrite.
    #[test]
    fn more_than_two_column_dimensions_are_refused() {
        assert!(
            validate(
                r#"{"source":"orders","rows":["id"],"columns":["customer","qty"],
                    "values":[{"fn":"count","as":"n"}]}"#,
            )
            .is_ok(),
            "two are fine"
        );
        let error = validate(
            r#"{"source":"orders","columns":["country","customer","qty"],
                "values":[{"fn":"count","as":"n"}]}"#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            PivotError::TooManyColumnDimensions {
                found: 3,
                maximum: 2
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

    /// Rule P9 names a row dimension once, and a measure the pivot has.
    #[test]
    fn a_sort_names_a_level_and_a_measure_of_this_pivot() {
        let sorted = |sort: &str| {
            validate(&format!(
                r#"{{"source":"orders","rows":["country","customer"],
                    "values":[{{"fn":"count","as":"n"}}],"sort":{sort}}}"#
            ))
        };
        assert!(sorted(r#"[{"field":"country","by":"n","direction":"desc"}]"#).is_ok());
        assert!(
            sorted(r#"[{"field":"customer"}]"#).is_ok(),
            "ascending by default"
        );
        for (sort, says) in [
            (r#"[{"field":"qty"}]"#, "not a row dimension"),
            (
                r#"[{"field":"country"},{"field":"country"}]"#,
                "sorted twice",
            ),
            (r#"[{"field":"country","by":"total"}]"#, "not a measure"),
        ] {
            let error = sorted(sort).unwrap_err();
            assert!(
                matches!(&error, PivotError::Sort { reason } if reason.contains(says)),
                "{sort}: {error}"
            );
        }
    }

    /// A measure sort with a column dimension asks the level once more, by
    /// its row dimensions alone; without one the level's own set is enough.
    #[test]
    fn a_measure_sort_across_columns_costs_one_query() {
        let with_columns = validate(
            r#"{"source":"orders","rows":["country","customer"],"columns":["qty"],
                "values":[{"fn":"count","as":"n"}],
                "sort":[{"field":"customer","by":"n","direction":"desc"}]}"#,
        )
        .expect("valid");
        assert_eq!(with_columns.orders.len(), 1);
        assert_eq!(with_columns.orders[0].0, 2);
        assert_eq!(with_columns.orders[0].1.group.len(), 2);
        let without = validate(
            r#"{"source":"orders","rows":["country"],"values":[{"fn":"count","as":"n"}],
                "sort":[{"field":"country","by":"n","direction":"desc"}]}"#,
        )
        .expect("valid");
        assert!(without.orders.is_empty());
    }

    /// A pivot without `sort` writes none, so a server from before P9 reads it.
    #[test]
    fn an_unsorted_pivot_writes_no_sort() {
        assert!(!opengrid_json::to_string(&pivot(SIMPLE)).contains("sort"));
        let sorted = pivot(
            r#"{"source":"orders","rows":["country"],"values":[{"fn":"count","as":"n"}],
                "sort":[{"field":"country","by":"n","direction":"desc"}]}"#,
        );
        let json = opengrid_json::to_string(&sorted);
        assert_eq!(
            opengrid_json::from_str::<PivotQuery>(&json).unwrap(),
            sorted
        );
    }

    /// The pivot query round-trips through its JSON form.
    #[test]
    fn a_pivot_reads_and_writes_itself() {
        let pivot = pivot(SIMPLE);
        let json = opengrid_json::to_string(&pivot);
        assert_eq!(opengrid_json::from_str::<PivotQuery>(&json).unwrap(), pivot);
    }
}
