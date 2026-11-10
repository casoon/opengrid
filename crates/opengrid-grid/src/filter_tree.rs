//! The nested filter (issue #324): a tree of conditions joined by `and` and
//! `or`, which the filter row — one condition per column, and-ed — cannot hold.
//!
//! # One model, in the wire form
//!
//! The tree **is** a query's [`FilterExpr`], restricted to what its two editors
//! — the search expression and the builder dialog — can write: `and`, `or` and
//! conditions. So a view holds it as the wire writes a filter (`where`), the
//! query and-s it as it is, and nothing translates between a view and a query.
//! `not` and `in` are left out on purpose: neither editor could show them, and
//! a filter the reader sees but cannot edit is a second truth.
//!
//! A condition's value is typed (a number is a JSON number). The editors work
//! on text, as the filter row does, so a condition reads back as the row's
//! [`FilterEntry`] ([`entry`]) and is typed like one ([`condition`]).

use opengrid_query::{CmpOp, FilterExpr};
use opengrid_types::Schema;

use crate::filter_row::{FilterEntry, FilterOp, FilterProblem, filter_expr};

/// Whether `filter` is a tree a view may hold: `and` and `or` with at least
/// one member, and conditions on columns `known` says the grid has. What is
/// not, in words.
pub fn check(filter: &FilterExpr, known: &dyn Fn(&str) -> bool) -> Result<(), String> {
    match filter {
        FilterExpr::And(items) | FilterExpr::Or(items) => {
            if items.is_empty() {
                return Err("a group has no member".to_owned());
            }
            items.iter().try_for_each(|item| check(item, known))
        }
        FilterExpr::Not(_) => Err("not is not part of a view's filter".to_owned()),
        FilterExpr::Cmp { op: CmpOp::In, .. } => {
            Err("in is not part of a view's filter".to_owned())
        }
        FilterExpr::Cmp { field, .. }
        | FilterExpr::IsNull { field }
        | FilterExpr::IsNotNull { field }
        | FilterExpr::IsEmpty { field } => {
            if known(field.as_str()) {
                Ok(())
            } else {
                Err(format!("{} is not a column of this grid", field.as_str()))
            }
        }
    }
}

/// A condition as the filter row's entry — its value as text, the way a
/// person types it — or `None` for a group.
pub fn entry(filter: &FilterExpr) -> Option<FilterEntry> {
    let (field, op, value) = match filter {
        FilterExpr::Cmp { field, op, value } => {
            let text = match value {
                opengrid_json::Json::String(text) => text.clone(),
                // `has_all` takes its elements separated by commas.
                opengrid_json::Json::Array(items) => items
                    .iter()
                    .map(|item| match item {
                        opengrid_json::Json::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            (field, FilterOp::Cmp(*op), text)
        }
        FilterExpr::IsNull { field } => (field, FilterOp::IsNull, String::new()),
        FilterExpr::IsNotNull { field } => (field, FilterOp::IsNotNull, String::new()),
        FilterExpr::IsEmpty { field } => (field, FilterOp::IsEmpty, String::new()),
        FilterExpr::And(_) | FilterExpr::Or(_) | FilterExpr::Not(_) => return None,
    };
    Some(FilterEntry {
        column: field.as_str().to_owned(),
        op,
        value,
    })
}

/// One entry as a condition, its value typed for its column — the filter
/// row's own translation ([`filter_expr`]). `None` for what the row skips: a
/// blank value, a column the schema does not have.
pub fn condition(
    entry: &FilterEntry,
    schema: &Schema,
) -> Result<Option<FilterExpr>, FilterProblem> {
    match filter_expr(std::slice::from_ref(entry), schema) {
        Ok(Some(FilterExpr::And(mut items))) => Ok(items.pop()),
        Ok(_) => Ok(None),
        Err(mut problems) => Err(problems.remove(0)),
    }
}

/// The members of a group as one tree, tidied: a member that is a group of
/// the same kind joins it, a group of one is that one, and no member is no
/// filter. `any` is `or`.
pub fn group(any: bool, members: Vec<FilterExpr>) -> Option<FilterExpr> {
    let mut items = Vec::with_capacity(members.len());
    for member in members {
        match (any, member) {
            (false, FilterExpr::And(inner)) | (true, FilterExpr::Or(inner)) => items.extend(inner),
            (_, member) => items.push(member),
        }
    }
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ if any => Some(FilterExpr::Or(items)),
        _ => Some(FilterExpr::And(items)),
    }
}

/// The tree in words: its conditions as `condition` says them, joined by the
/// words `and` and `or`, a group inside a group in parentheses.
pub fn words(
    filter: &FilterExpr,
    condition: &dyn Fn(&FilterEntry) -> String,
    and: &str,
    or: &str,
) -> String {
    let (items, word) = match filter {
        FilterExpr::And(items) => (items, and),
        FilterExpr::Or(items) => (items, or),
        other => {
            return entry(other)
                .map(|entry| condition(&entry))
                .unwrap_or_default();
        }
    };
    items
        .iter()
        .map(|item| {
            let text = words(item, condition, and, or);
            match item {
                FilterExpr::And(_) | FilterExpr::Or(_) => format!("({text})"),
                _ => text,
            }
        })
        .collect::<Vec<_>>()
        .join(&format!(" {word} "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_json::{FromJson, Json, ToJson, json};
    use opengrid_types::{DataType, Field, FieldName};

    fn filter(json: Json) -> FilterExpr {
        FilterExpr::from_json(&json).unwrap()
    }

    fn known(name: &str) -> bool {
        ["country", "amount", "tags"].contains(&name)
    }

    fn schema() -> Schema {
        let field = |name: &str, data_type| Field::new(FieldName::new(name).unwrap(), data_type);
        Schema::new(vec![
            field("country", DataType::Utf8),
            field("amount", DataType::Int64),
            field("tags", DataType::ListUtf8),
        ])
    }

    fn plain(entry: &FilterEntry) -> String {
        if entry.value.is_empty() {
            format!("{} {}", entry.column, entry.op.as_str())
        } else {
            format!("{} {} {}", entry.column, entry.op.as_str(), entry.value)
        }
    }

    #[test]
    fn a_view_takes_and_or_and_conditions_on_its_columns() {
        let tree = filter(json!({ "or": [
            { "field": "country", "op": "eq", "value": "DE" },
            { "and": [
                { "field": "amount", "op": "gte", "value": 1000 },
                { "field": "country", "op": "is_null" },
            ] },
        ] }));
        assert_eq!(check(&tree, &known), Ok(()));
    }

    #[test]
    fn what_no_editor_can_show_is_refused_by_name() {
        let refused = |json: Json| check(&filter(json), &known).unwrap_err();
        assert!(
            refused(json!({ "not": { "field": "country", "op": "eq", "value": "DE" } }))
                .contains("not")
        );
        assert!(
            refused(json!({ "field": "country", "op": "in", "value": ["DE"] })).starts_with("in ")
        );
        assert!(refused(json!({ "or": [] })).contains("no member"));
        // Deep inside a group, too.
        assert_eq!(
            refused(json!({ "or": [{ "and": [{ "field": "nope", "op": "is_null" }] }] })),
            "nope is not a column of this grid"
        );
    }

    /// A condition reads back as text and types again to the same condition.
    #[test]
    fn a_condition_is_an_entry_and_back() {
        for json in [
            json!({ "field": "country", "op": "eq", "value": "DE" }),
            json!({ "field": "amount", "op": "gte", "value": 1000 }),
            json!({ "field": "amount", "op": "lt", "value": -5 }),
            json!({ "field": "tags", "op": "has_all", "value": ["a", "b"] }),
            json!({ "field": "tags", "op": "is_empty" }),
            json!({ "field": "country", "op": "is_null" }),
            json!({ "field": "country", "op": "is_not_null" }),
        ] {
            let tree = filter(json.clone());
            let entry = entry(&tree).unwrap();
            assert_eq!(
                condition(&entry, &schema()).unwrap().unwrap().to_json(),
                json,
                "{entry:?}"
            );
        }
        assert_eq!(entry(&filter(json!({ "and": [] }))), None);
        assert_eq!(
            entry(&filter(
                json!({ "field": "tags", "op": "has_all", "value": ["a", "b"] })
            ))
            .unwrap()
            .value,
            "a, b"
        );
    }

    #[test]
    fn a_value_its_column_cannot_hold_is_a_problem_and_a_blank_one_is_nothing() {
        let entry = |value: &str| FilterEntry {
            column: "amount".to_owned(),
            op: FilterOp::Cmp(CmpOp::Gt),
            value: value.to_owned(),
        };
        assert_eq!(
            condition(&entry("many"), &schema()),
            Err(FilterProblem {
                column: "amount".to_owned(),
                value: "many".to_owned()
            })
        );
        assert_eq!(condition(&entry(" "), &schema()), Ok(None));
    }

    #[test]
    fn a_group_is_tidied() {
        let leaf = |value: i64| filter(json!({ "field": "amount", "op": "eq", "value": value }));
        assert_eq!(group(true, Vec::new()), None);
        assert_eq!(group(true, vec![leaf(1)]), Some(leaf(1)));
        assert_eq!(
            group(false, vec![leaf(1), leaf(2)]),
            Some(FilterExpr::And(vec![leaf(1), leaf(2)]))
        );
        // The same kind joins, the other kind stays a group.
        assert_eq!(
            group(
                true,
                vec![
                    FilterExpr::Or(vec![leaf(1), leaf(2)]),
                    FilterExpr::And(vec![leaf(3), leaf(4)]),
                ]
            ),
            Some(FilterExpr::Or(vec![
                leaf(1),
                leaf(2),
                FilterExpr::And(vec![leaf(3), leaf(4)]),
            ]))
        );
    }

    #[test]
    fn the_words_join_conditions_and_put_inner_groups_in_parentheses() {
        let tree = filter(json!({ "and": [
            { "field": "country", "op": "eq", "value": "DE" },
            { "or": [
                { "field": "amount", "op": "gte", "value": 1000 },
                { "field": "tags", "op": "is_empty" },
            ] },
        ] }));
        assert_eq!(
            words(&tree, &plain, "and", "or"),
            "country eq DE and (amount gte 1000 or tags is_empty)"
        );
        assert_eq!(
            words(
                &filter(json!({ "or": [
                    { "field": "country", "op": "eq", "value": "DE" },
                    { "field": "amount", "op": "gte", "value": 1000 },
                ] })),
                &plain,
                "und",
                "oder"
            ),
            "country eq DE oder amount gte 1000"
        );
        assert_eq!(
            words(
                &filter(json!({ "field": "country", "op": "eq", "value": "DE" })),
                &plain,
                "and",
                "or"
            ),
            "country eq DE"
        );
    }
}
