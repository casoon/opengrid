//! The query a grid asks, as JSON (issue #144): its window, its whole view,
//! a tree level — and the query of a view without an element.

use opengrid_query::FilterExpr;

/// Builds the query JSON for a grid render.
///
/// The shape is `{ "source", "select", "filter"?, "sort"?, "limit", "offset" }`
/// (plan/spezifikation/02-query-modell.md §JSON-Vertrag). `sorts` is the whole
/// sort list in the user's order (point 18 multi-sort); each entry becomes a
/// `{ "field", "direction" }` object and the key is omitted when nothing is
/// sorted. `filter` is serialized from the [`FilterExpr`] built by
/// [`filter_expr`](crate::filter_row::filter_expr); `direction` is the wire token `"asc"`/`"desc"`.
pub fn query_json(
    source: &str,
    columns: &[String],
    sorts: &[(String, &str)],
    filter: Option<&FilterExpr>,
    offset: u64,
    limit: u64,
) -> String {
    use opengrid_json::{Json, json};
    let sort = sorts
        .iter()
        .map(|(field, direction)| json!({ "field": field, "direction": direction }))
        .collect();
    let mut query = query_object(source, columns, sort, filter);
    query.insert("limit".to_owned(), json!(limit));
    query.insert("offset".to_owned(), json!(offset));
    Json::Object(query).to_string()
}

/// The query of a whole view — the same as the grid asks, without a window
/// (plan point 82): what a page exports.
///
/// `groups` come first, ascending with **NULL last** stated explicitly — the
/// order `group_query_json` in `opengrid-web-components` gives the groups, written the
/// same way, so the rows of a grouped grid come in the order it shows them.
/// Then `sorts`, without the keys `groups` already has.
pub fn view_query_json(
    source: &str,
    columns: &[String],
    groups: &[String],
    sorts: &[(String, &str)],
    filter: Option<&FilterExpr>,
) -> String {
    use opengrid_json::json;
    let sort = groups
        .iter()
        .map(|field| json!({ "field": field, "direction": "asc", "nulls": "last" }))
        .chain(
            sorts
                .iter()
                .filter(|(field, _)| !groups.contains(field))
                .map(|(field, direction)| json!({ "field": field, "direction": direction })),
        )
        .collect();
    opengrid_json::Json::Object(query_object(source, columns, sort, filter)).to_string()
}

/// Source, projection, filter and sort — everything but the window.
fn query_object(
    source: &str,
    columns: &[String],
    sort: Vec<opengrid_json::Json>,
    filter: Option<&FilterExpr>,
) -> opengrid_json::Object {
    use opengrid_json::Json;
    let mut query = opengrid_json::Object::new();
    query.insert("source".to_owned(), Json::String(source.to_owned()));
    query.insert(
        "select".to_owned(),
        Json::Array(columns.iter().cloned().map(Json::String).collect()),
    );
    if let Some(filter) = filter {
        let filter = opengrid_json::ToJson::to_json(&filter);
        query.insert("filter".to_owned(), filter);
    }
    if !sort.is_empty() {
        query.insert("sort".to_owned(), Json::Array(sort));
    }
    query
}

/// The query of one level of a tree (issue #135): the rows of `select` —
/// the shown columns, and the key when they do not hold it — at the level
/// `tree` names, under the grid's filter and order, at most `limit` of them.
/// `query` with the collation `locale` on every sort key that is a text column
/// of `schema` (rule S4a, issue #391) — the grid's `collation` attribute put
/// into a query it is about to send. Keys on other columns, and on names the
/// schema does not have (an aggregate's alias), stay as they are: a locale
/// orders text, and the query model refuses it anywhere else.
///
/// `None` when nothing changes — no sort, no text key, or not a query.
pub fn collated(
    query: &str,
    locale: &opengrid_query::Locale,
    schema: &opengrid_types::Schema,
) -> Option<String> {
    use opengrid_json::Json;
    let mut parsed = Json::parse(query).ok()?;
    if !matches!(parsed.get("sort"), Some(Json::Array(_))) {
        return None;
    }
    let Json::Array(keys) = &mut parsed["sort"] else {
        return None;
    };
    let mut changed = false;
    for key in keys {
        let Json::Object(key) = key else {
            continue;
        };
        let text = key
            .get("field")
            .and_then(Json::as_str)
            .and_then(|field| schema.field(field))
            .is_some_and(|field| field.data_type == opengrid_types::DataType::Utf8);
        if text {
            key.insert(
                "collation".to_owned(),
                Json::String(locale.as_str().to_owned()),
            );
            changed = true;
        }
    }
    changed.then(|| parsed.to_string())
}

pub fn tree_query_json(
    source: &str,
    select: &[String],
    sorts: &[(String, &str)],
    filter: Option<&FilterExpr>,
    tree: opengrid_json::Json,
    limit: u64,
) -> String {
    use opengrid_json::{Json, json};
    let sort = sorts
        .iter()
        .map(|(field, direction)| json!({ "field": field, "direction": direction }))
        .collect();
    let mut query = query_object(source, select, sort, filter);
    query.insert("tree".to_owned(), tree);
    query.insert("limit".to_owned(), json!(limit));
    Json::Object(query).to_string()
}

/// The query of `view` on a grid whose `columns` attribute is `declared`
/// (issue #144): what the element's `get_query()` answers once the view is
/// applied — the shown columns in layout order, the filter row and the facets
/// typed against `schema`, the grouping, and the sort, which falls back to the
/// first shown column (S6) as the element's does. The columns the view pins
/// stand at their end of the row; a pin or a band of the page's `set_columns`
/// is not part of a view, and not known here.
///
/// A filter or facet value that is not a value of its column is an error, one
/// sentence per value: the element would say the same in its status line.
pub fn view_query(
    source: &str,
    declared: &[String],
    view: &crate::grid_view::GridView,
    schema: &opengrid_types::Schema,
) -> Result<String, Vec<String>> {
    let columns = view.columns.effective(declared, &Default::default());
    let Some(first) = columns.first() else {
        return Err(vec!["the view shows no column".to_owned()]);
    };
    let not_a_value =
        |column: &str, value: &str| format!("{column}: {value:?} is not a value of this column");
    let row = crate::filter_row::filter_expr(&view.filters, schema).map_err(|problems| {
        problems
            .iter()
            .map(|problem| not_a_value(&problem.column, &problem.value))
            .collect::<Vec<_>>()
    })?;
    let selections: Vec<(String, crate::facets::Selection)> = view
        .facets
        .as_object()
        .map(|facets| {
            facets
                .iter()
                .filter_map(|(column, selection)| {
                    crate::facets::Selection::from_json(selection)
                        .map(|selection| (column.clone(), selection))
                })
                .collect()
        })
        .unwrap_or_default();
    // The nested filter is a filter already (issue #324): and-ed as it is.
    let nested = match &view.nested {
        Some(tree) => Some(
            <opengrid_query::FilterExpr as opengrid_json::FromJson>::from_json(tree)
                .map_err(|error| vec![format!("where: {error}")])?,
        ),
        None => None,
    };
    let base = match (row, nested) {
        (Some(row), Some(nested)) => Some(FilterExpr::And(vec![row, nested])),
        (row, nested) => row.or(nested),
    };
    let filter =
        crate::facets::effective(base.as_ref(), &selections, None, schema).map_err(|problems| {
            problems
                .iter()
                .map(|problem| not_a_value(&problem.column, &problem.value))
                .collect::<Vec<_>>()
        })?;
    // A grouping on a column the grid does not show is refused there, and the
    // grid runs ungrouped.
    let groups: Vec<String> = if view.group.iter().all(|column| columns.contains(column)) {
        view.group.clone()
    } else {
        Vec::new()
    };
    let mut sorts: Vec<(String, &str)> = view
        .sort
        .iter()
        .map(|(field, direction)| {
            let direction = if direction == "desc" { "desc" } else { "asc" };
            (field.clone(), direction)
        })
        .collect();
    if sorts.is_empty() {
        sorts.push((first.clone(), "asc"));
    }
    Ok(view_query_json(
        source,
        &columns,
        &groups,
        &sorts,
        filter.as_ref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid_view::GridView;
    use opengrid_json::Json;
    use opengrid_types::{DataType, Field, FieldName, Schema};

    /// The grid's collation goes on the text keys of a sort and nowhere else,
    /// and the query it makes is one the model accepts (#391).
    #[test]
    fn a_collation_goes_on_the_text_keys_only() {
        let schema = Schema::new(vec![
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
            Field::new(FieldName::new("name").unwrap(), DataType::Utf8),
        ]);
        let locale = opengrid_query::Locale::parse("de").unwrap();
        let query = query_json(
            "orders",
            &["id".to_owned(), "name".to_owned()],
            &[("name".to_owned(), "desc"), ("id".to_owned(), "asc")],
            None,
            0,
            10,
        );
        let collated = collated(&query, &locale, &schema).expect("a text key");
        let parsed = Json::parse(&collated).unwrap();
        assert_eq!(parsed["sort"][0]["collation"].as_str(), Some("de"));
        assert_eq!(parsed["sort"][0]["direction"].as_str(), Some("desc"));
        assert!(parsed["sort"][1].get("collation").is_none());
        let model: opengrid_query::Query = opengrid_json::from_str(&collated).unwrap();
        model
            .validate(&schema, &opengrid_query::Limits::default())
            .expect("the model takes it");

        // Nothing to do is said as such, not as a copy.
        let by_id = query_json(
            "orders",
            &["id".to_owned()],
            &[("id".to_owned(), "asc")],
            None,
            0,
            10,
        );
        assert_eq!(super::collated(&by_id, &locale, &schema), None);
        assert_eq!(super::collated("not a query", &locale, &schema), None);
    }

    fn schema() -> Schema {
        let field = |name: &str, data_type| Field::new(FieldName::new(name).unwrap(), data_type);
        Schema::new(vec![
            field("id", DataType::Int64),
            field("country", DataType::Utf8),
            field("qty", DataType::Int64),
        ])
    }

    fn query(view: &str) -> Result<Json, Vec<String>> {
        let declared: Vec<String> = ["id", "country", "qty"].map(String::from).to_vec();
        let view = GridView::from_json(&Json::parse(view).unwrap(), &declared).unwrap();
        view_query("orders", &declared, &view, &schema()).map(|json| Json::parse(&json).unwrap())
    }

    /// The shown columns in their order, typed literals, and the sort.
    #[test]
    fn a_view_is_its_query() {
        let query = query(
            r#"{"sort":[{"field":"qty","direction":"desc"}],
                "filters":[{"column":"qty","op":"gte","value":"5"}],
                "columns":{"order":["qty"],"hidden":["country"]}}"#,
        )
        .unwrap();
        assert_eq!(query["select"], opengrid_json::json!(["qty", "id"]));
        assert_eq!(
            query["filter"]["and"][0]["value"],
            opengrid_json::json!(5),
            "typed, not text"
        );
        assert_eq!(query["sort"][0]["field"], opengrid_json::json!("qty"));
        assert!(query.get("limit").is_none(), "the whole view, no window");
    }

    /// No sort: the first shown column, as the grid pages (S6).
    #[test]
    fn an_empty_view_sorts_by_the_first_column() {
        let query = query("{}").unwrap();
        assert_eq!(
            query["sort"],
            opengrid_json::json!([{ "field": "id", "direction": "asc" }])
        );
    }

    /// The nested filter is and-ed onto the filter row's, as written.
    #[test]
    fn the_nested_filter_is_part_of_the_query() {
        let tree = r#"{"or":[{"field":"country","op":"eq","value":"DE"},{"field":"qty","op":"gte","value":1000}]}"#;
        // Alone, it is the filter.
        let alone = query(&format!(r#"{{"where":{tree}}}"#)).unwrap();
        assert_eq!(alone["filter"], Json::parse(tree).unwrap());
        let both = query(&format!(
            r#"{{"filters":[{{"column":"qty","op":"lt","value":"5"}}],"where":{tree}}}"#
        ))
        .unwrap();
        assert_eq!(
            both["filter"]["and"][0]["and"][0]["op"],
            opengrid_json::json!("lt")
        );
        assert_eq!(both["filter"]["and"][1], Json::parse(tree).unwrap());
    }

    #[test]
    fn a_value_its_column_cannot_hold_is_named() {
        let problems =
            query(r#"{"filters":[{"column":"qty","op":"gte","value":"many"}]}"#).unwrap_err();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("qty") && problems[0].contains("many"));
    }
}
