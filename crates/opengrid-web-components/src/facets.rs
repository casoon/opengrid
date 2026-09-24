//! Facets (plan point 66): what a reader chose in the sidebar, as filters.
//!
//! # The counting rule
//!
//! > A facet's counts are taken **without its own restriction**.
//!
//! Otherwise, after ticking "Alpha", every other customer counts 0 and the
//! facet can only be undone, never used. This is the one rule of the point that
//! can be broken without anybody seeing it — so it lives here, in one function
//! ([`effective`] with `except`), with a test that bites.
//!
//! It costs **one** `group` query per facet column, not one per value: grouped
//! by that column, filtered by everything else (F4, decided 2026-09-24: always
//! count).
//!
//! # Facets are filters
//!
//! A selection is AND-ed onto the filter row's filter, and each kind becomes an
//! ordinary expression the engine already runs: a list of values is an `or` of
//! comparisons, a range and a period are two bounds. NULL among the chosen
//! values is `is_null` — `eq null` is *unknown* under three-valued logic and
//! matches nothing (S1) — and the empty string is `eq ""`, a different value
//! (S14).

use opengrid_query::{CmpOp, FilterExpr};
use opengrid_types::{DataType, FieldName, Schema};
use serde_json::{Map, Value};

use crate::presentation::FacetKind;

/// What a reader chose in one facet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    /// The chosen values of a list or pills facet, as wire JSON (NULL is
    /// `null`). Empty means "no restriction".
    Values(Vec<Value>),
    /// A range of numbers, as typed; either bound may be empty.
    Range { min: String, max: String },
    /// A period of dates (`YYYY-MM-DD`); either bound may be empty.
    Period { from: String, to: String },
}

impl Selection {
    /// Whether it restricts anything.
    pub fn is_active(&self) -> bool {
        match self {
            Selection::Values(values) => !values.is_empty(),
            Selection::Range { min, max } => !min.trim().is_empty() || !max.trim().is_empty(),
            Selection::Period { from, to } => !from.trim().is_empty() || !to.trim().is_empty(),
        }
    }

    /// The empty selection of a facet kind.
    pub fn empty(kind: FacetKind) -> Self {
        match kind {
            FacetKind::List | FacetKind::Pills => Selection::Values(Vec::new()),
            FacetKind::Range => Selection::Range {
                min: String::new(),
                max: String::new(),
            },
            FacetKind::Period => Selection::Period {
                from: String::new(),
                to: String::new(),
            },
        }
    }

    /// The selection as the view writes it.
    pub fn to_json(&self) -> Value {
        match self {
            Selection::Values(values) => serde_json::json!({ "values": values }),
            Selection::Range { min, max } => serde_json::json!({ "min": min, "max": max }),
            Selection::Period { from, to } => serde_json::json!({ "from": from, "to": to }),
        }
    }

    /// Reads a selection from the view.
    pub fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let text = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        if let Some(values) = object.get("values") {
            return values
                .as_array()
                .map(|values| Selection::Values(values.clone()));
        }
        if object.contains_key("min") || object.contains_key("max") {
            return Some(Selection::Range {
                min: text("min"),
                max: text("max"),
            });
        }
        if object.contains_key("from") || object.contains_key("to") {
            return Some(Selection::Period {
                from: text("from"),
                to: text("to"),
            });
        }
        None
    }
}

/// A bound that is not a value of its column, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub column: String,
    pub value: String,
}

/// One facet's selection as a filter, or `None` when it restricts nothing.
///
/// Answers `Err` for a bound that is not a value of the column ("abc" as a
/// minimum amount): said, not dropped — a range that silently ignored its
/// minimum would look applied and be half of it.
pub fn filter_of(
    column: &str,
    data_type: DataType,
    selection: &Selection,
) -> Result<Option<FilterExpr>, Problem> {
    let Ok(field) = FieldName::new(column) else {
        return Ok(None);
    };
    match selection {
        Selection::Values(values) => {
            let mut parts: Vec<FilterExpr> = values
                .iter()
                .map(|value| {
                    if value.is_null() {
                        FilterExpr::IsNull {
                            field: field.clone(),
                        }
                    } else {
                        FilterExpr::Cmp {
                            field: field.clone(),
                            op: CmpOp::Eq,
                            value: value.clone(),
                        }
                    }
                })
                .collect();
            Ok(match parts.len() {
                0 => None,
                1 => parts.pop(),
                _ => Some(FilterExpr::Or(parts)),
            })
        }
        Selection::Range { min, max } => bounds(&field, column, data_type, min, max),
        Selection::Period { from, to } => bounds(&field, column, data_type, from, to),
    }
}

fn bounds(
    field: &FieldName,
    column: &str,
    data_type: DataType,
    low: &str,
    high: &str,
) -> Result<Option<FilterExpr>, Problem> {
    let mut parts = Vec::new();
    for (text, op) in [(low, CmpOp::Gte), (high, CmpOp::Lte)] {
        if text.trim().is_empty() {
            continue;
        }
        let Some(value) = crate::grid::literal(text, data_type) else {
            return Err(Problem {
                column: column.to_owned(),
                value: text.trim().to_owned(),
            });
        };
        parts.push(FilterExpr::Cmp {
            field: field.clone(),
            op,
            value,
        });
    }
    Ok(match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(FilterExpr::And(parts)),
    })
}

/// The filter a query runs under: the filter row's, **and** every facet's —
/// except the one named by `except`, which is how a facet counts its values
/// without its own restriction.
///
/// Facets on columns the schema does not have are skipped: a hidden column
/// keeps its selection (it comes back), but cannot restrict a query that does
/// not select it.
pub fn effective(
    base: Option<&FilterExpr>,
    facets: &[(String, Selection)],
    except: Option<&str>,
    schema: &Schema,
) -> Result<Option<FilterExpr>, Vec<Problem>> {
    let mut parts: Vec<FilterExpr> = base.cloned().into_iter().collect();
    let mut problems = Vec::new();
    for (column, selection) in facets {
        if Some(column.as_str()) == except {
            continue;
        }
        let Some(field) = schema
            .fields()
            .iter()
            .find(|field| field.name.as_str() == column)
        else {
            continue;
        };
        match filter_of(column, field.data_type, selection) {
            Ok(Some(expr)) => parts.push(expr),
            Ok(None) => {}
            Err(problem) => problems.push(problem),
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(FilterExpr::And(parts)),
    })
}

/// The facets as the view writes them: `{ column: selection }`.
pub fn to_json(facets: &[(String, Selection)]) -> Value {
    let mut out = Map::new();
    for (column, selection) in facets {
        if selection.is_active() {
            out.insert(column.clone(), selection.to_json());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_types::Field;

    fn schema() -> Schema {
        Schema::new(vec![
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("country").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("amount").unwrap(), DataType::Int64),
        ])
    }

    fn values(list: &[Value]) -> Selection {
        Selection::Values(list.to_vec())
    }

    #[test]
    fn a_facet_counts_without_its_own_restriction() {
        // The rule of the point: with "Alpha" ticked, the customer facet is
        // counted under everything *but* that — or every other customer counts 0.
        let facets = vec![
            (
                "customer".to_owned(),
                values(&[Value::String("Alpha".to_owned())]),
            ),
            (
                "country".to_owned(),
                values(&[Value::String("DE".to_owned())]),
            ),
        ];
        let for_customer = effective(None, &facets, Some("customer"), &schema()).unwrap();
        assert_eq!(
            for_customer,
            Some(FilterExpr::Cmp {
                field: FieldName::new("country").unwrap(),
                op: CmpOp::Eq,
                value: Value::String("DE".to_owned()),
            }),
            "the customer counts must see the country facet and not their own"
        );
        let for_rows = effective(None, &facets, None, &schema()).unwrap();
        assert!(matches!(for_rows, Some(FilterExpr::And(parts)) if parts.len() == 2));
    }

    #[test]
    fn null_among_the_values_is_is_null_and_empty_is_eq() {
        let selection = values(&[Value::Null, Value::String(String::new())]);
        let Ok(Some(FilterExpr::Or(parts))) = filter_of("country", DataType::Utf8, &selection)
        else {
            panic!("two values are an or");
        };
        assert_eq!(
            parts[0],
            FilterExpr::IsNull {
                field: FieldName::new("country").unwrap()
            }
        );
        assert!(matches!(&parts[1], FilterExpr::Cmp { op: CmpOp::Eq, value, .. } if value == ""));
    }

    #[test]
    fn a_range_takes_either_bound_alone() {
        let min_only = Selection::Range {
            min: "10".to_owned(),
            max: String::new(),
        };
        assert!(matches!(
            filter_of("amount", DataType::Int64, &min_only),
            Ok(Some(FilterExpr::Cmp { op: CmpOp::Gte, .. }))
        ));
        let both = Selection::Range {
            min: "10".to_owned(),
            max: "20".to_owned(),
        };
        assert!(matches!(
            filter_of("amount", DataType::Int64, &both),
            Ok(Some(FilterExpr::And(_)))
        ));
    }

    #[test]
    fn a_bound_that_is_not_a_value_is_named() {
        let wrong = Selection::Range {
            min: "abc".to_owned(),
            max: String::new(),
        };
        assert_eq!(
            filter_of("amount", DataType::Int64, &wrong),
            Err(Problem {
                column: "amount".to_owned(),
                value: "abc".to_owned()
            })
        );
    }

    #[test]
    fn nothing_chosen_restricts_nothing() {
        let facets = vec![("customer".to_owned(), values(&[]))];
        assert_eq!(effective(None, &facets, None, &schema()), Ok(None));
    }

    #[test]
    fn the_filter_row_is_kept_under_the_facets() {
        let base = FilterExpr::IsNotNull {
            field: FieldName::new("amount").unwrap(),
        };
        let facets = vec![(
            "customer".to_owned(),
            values(&[Value::String("Alpha".to_owned())]),
        )];
        let Ok(Some(FilterExpr::And(parts))) = effective(Some(&base), &facets, None, &schema())
        else {
            panic!("the facets are and-ed onto the filter row");
        };
        assert_eq!(parts[0], base);
    }

    #[test]
    fn a_selection_round_trips_through_the_view() {
        for selection in [
            values(&[Value::Null, Value::String("Alpha".to_owned())]),
            Selection::Range {
                min: "1".to_owned(),
                max: "".to_owned(),
            },
            Selection::Period {
                from: "2026-01-01".to_owned(),
                to: "2026-12-31".to_owned(),
            },
        ] {
            assert_eq!(
                Selection::from_json(&selection.to_json()),
                Some(selection.clone())
            );
        }
    }
}
