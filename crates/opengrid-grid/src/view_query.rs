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
/// [`filter_expr`]; `direction` is the wire token `"asc"`/`"desc"`.
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
/// order [`crate::grouping::group_query_json`] gives the groups, written the
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
/// first shown column (S6) as the element's does.
///
/// A filter or facet value that is not a value of its column is an error, one
/// sentence per value: the element would say the same in its status line.
pub fn view_query(
    source: &str,
    declared: &[String],
    view: &crate::grid_view::GridView,
    schema: &opengrid_types::Schema,
) -> Result<String, Vec<String>> {
    let columns = view.columns.effective(declared);
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
    let filter =
        crate::facets::effective(row.as_ref(), &selections, None, schema).map_err(|problems| {
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

    #[test]
    fn a_value_its_column_cannot_hold_is_named() {
        let problems =
            query(r#"{"filters":[{"column":"qty","op":"gte","value":"many"}]}"#).unwrap_err();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("qty") && problems[0].contains("many"));
    }
}
