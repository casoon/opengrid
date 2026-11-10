//! The view as a value (plan point 59).
//!
//! # Why this exists before anything that fills it
//!
//! A "saved view" is nothing but serialized view state. Once that is true, the
//! element does not need a view-management UI at all — naming, storing,
//! deleting, putting one in a URL are all the page's business, and the element
//! only has to be able to hand its view out and take one back. That is the same
//! line `docs/api.md` already draws for editing: *the component edits, the page
//! saves.*
//!
//! So this module comes **first**, before grouping (62), facets (66) or the
//! search bar (67), for the reason point 35 fixed the event contract before
//! anything else fired an event: whoever needs a contract first writes it, and
//! everything after hangs off it. The alternative is ten features each
//! inventing its own way to remember and report itself.
//!
//! # What is in a view, and what is not
//!
//! In: everything the reader chose about *what is shown and how* — sort,
//! filters, column order, hidden columns, column widths, density. The fields
//! nothing fills yet (`group`, `expanded`, `facets`) are declared here anyway,
//! because later points should add content to one structure rather than
//! structure of their own.
//!
//! **The selection is in it where it names records** (issue #322): `selected`
//! holds the keys of the selected records — of a grid with a `row-key`, or of
//! a tree's nodes. **A selection by position is out**: without a key the grid
//! names a row by its number in the result, and after a different sort those
//! positions hold different records (`docs/api.md`). A restored view carrying
//! positions would not be incomplete, it would be *wrong*.
//!
//! # Where the state actually lives
//!
//! Nowhere near one place, which is the other reason to have this module: the
//! sort is in [`GridState`](crate::GridState), the filters are in the
//! DOM inputs of the filter row, the column layout is in
//! [`ColumnLayout`] and the density is an
//! attribute. Gathering and restoring that is exactly the work this module
//! names, and the element glue is the only place that knows all four.

use opengrid_json::{Json as Value, Object as Map};

use crate::filter_row::{FilterEntry, FilterOp};
use crate::layout::ColumnLayout;

/// The whole view state, as one value.
/// The densities a view may name; anything else is `normal` (the element's
/// `DENSITIES` carry the same names with their measures).
pub const DENSITY_NAMES: [&str; 3] = ["compact", "normal", "comfortable"];

/// The most levels a grid groups by (issue #321; two until then). Every open
/// group above the innermost level costs one query for the groups under it,
/// so the number is bounded.
pub const MAX_GROUP_LEVELS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridView {
    /// Sort keys, outermost first, as `(column, "asc" | "desc")`.
    pub sort: Vec<(String, String)>,
    /// One entry per column that carries a filter. Columns without one are
    /// left out rather than written as empty: a view is what the reader chose.
    pub filters: Vec<FilterEntry>,
    /// The reader's column layout: order, hidden, widths.
    pub columns: ColumnLayout,
    /// `compact`, `normal` or `comfortable`.
    pub density: String,
    /// The columns grouped by, outermost first (point 62).
    pub group: Vec<String>,
    /// The open groups, each as its path of keys in wire JSON (point 62).
    pub expanded: Vec<Vec<Value>>,
    /// The reader's aggregate per column, as `(column, fn)`, sorted by column
    /// so the same choice always serialises the same way (point 63).
    pub aggregates: Vec<(String, String)>,
    /// Whether the filter row shows (point 65). On unless turned off.
    pub filter_row: bool,
    /// The facet selections, `{ column: selection }` (point 66).
    pub facets: Value,
    /// The keys of the selected records (issue #322), in the one order
    /// [`selected_keys`] gives them — empty where the grid selects by
    /// position, and then not written.
    pub selected: Vec<Value>,
}

/// A selection's keys as a view and an event hold them: each once, in the
/// order of the keys — numbers ascending (`2` before `10`, which their text is
/// not), everything else by its JSON text. So the same selection is the same
/// list, whatever order it was made or handed in.
pub fn selected_keys(keys: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut keyed: Vec<(String, Value)> =
        keys.into_iter().map(|key| (key.to_string(), key)).collect();
    keyed.sort_by(|a, b| match (a.1.as_f64(), b.1.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y).then_with(|| a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    keyed.dedup_by(|a, b| a.0 == b.0);
    keyed.into_iter().map(|(_, key)| key).collect()
}

impl Default for GridView {
    fn default() -> Self {
        Self {
            sort: Vec::new(),
            filters: Vec::new(),
            columns: ColumnLayout::default(),
            density: String::new(),
            group: Vec::new(),
            expanded: Vec::new(),
            aggregates: Vec::new(),
            filter_row: true,
            facets: Value::Object(Map::new()),
            selected: Vec::new(),
        }
    }
}

/// What a view could not be read from, so the status line can say it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewProblem {
    /// The field of the view the problem is in (`sort`, `filters`, …).
    pub field: String,
    /// What was wrong with it, in words.
    pub reason: String,
}

impl ViewProblem {
    fn new(field: &str, reason: impl Into<String>) -> Self {
        Self {
            field: field.to_owned(),
            reason: reason.into(),
        }
    }
}

impl GridView {
    /// The view as JSON, in the shape a page reads and writes.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();

        out.insert(
            "sort".to_owned(),
            Value::Array(
                self.sort
                    .iter()
                    .map(|(field, direction)| {
                        let mut entry = Map::new();
                        entry.insert("field".to_owned(), Value::String(field.clone()));
                        entry.insert("direction".to_owned(), Value::String(direction.clone()));
                        Value::Object(entry)
                    })
                    .collect(),
            ),
        );

        out.insert(
            "filters".to_owned(),
            Value::Array(
                self.filters
                    .iter()
                    .map(|entry| {
                        let mut filter = Map::new();
                        filter.insert("column".to_owned(), Value::String(entry.column.clone()));
                        filter.insert("op".to_owned(), Value::String(entry.op.as_str().to_owned()));
                        filter.insert("value".to_owned(), Value::String(entry.value.clone()));
                        Value::Object(filter)
                    })
                    .collect(),
            ),
        );

        let mut columns = Map::new();
        columns.insert(
            "order".to_owned(),
            Value::Array(
                self.columns
                    .order()
                    .iter()
                    .map(|name| Value::String(name.clone()))
                    .collect(),
            ),
        );
        columns.insert(
            "hidden".to_owned(),
            Value::Array(
                self.columns
                    .hidden()
                    .iter()
                    .map(|name| Value::String(name.clone()))
                    .collect(),
            ),
        );
        let mut widths = Map::new();
        for (name, width) in self.columns.widths() {
            widths.insert(name, Value::Number(width.into()));
        }
        columns.insert("widths".to_owned(), Value::Object(widths));
        out.insert("columns".to_owned(), Value::Object(columns));

        out.insert("density".to_owned(), Value::String(self.density.clone()));

        out.insert(
            "group".to_owned(),
            Value::Array(self.group.iter().cloned().map(Value::String).collect()),
        );
        out.insert(
            "expanded".to_owned(),
            Value::Array(self.expanded.iter().cloned().map(Value::Array).collect()),
        );
        let mut aggregates = Map::new();
        for (column, function) in &self.aggregates {
            aggregates.insert(column.clone(), Value::String(function.clone()));
        }
        out.insert("aggregates".to_owned(), Value::Object(aggregates));
        out.insert("filterRow".to_owned(), Value::Bool(self.filter_row));
        out.insert("facets".to_owned(), self.facets.clone());
        // Only a selection by key, and only when there is one: a grid that
        // selects by position has nothing to write here.
        if !self.selected.is_empty() {
            out.insert("selected".to_owned(), Value::Array(self.selected.clone()));
        }

        Value::Object(out)
    }

    /// Reads a view, refusing anything that names a column the grid does not
    /// have.
    ///
    /// **Nothing is dropped silently.** A view that mentions an unknown column
    /// is a page bug or a view saved against a different data source, and
    /// either way the reader has to be told — the alternative is a grid that
    /// looks restored and is not (the rule from point 56 §Das Modell §2).
    pub fn from_json(value: &Value, declared: &[String]) -> Result<Self, Vec<ViewProblem>> {
        let mut problems = Vec::new();
        let Some(object) = value.as_object() else {
            return Err(vec![ViewProblem::new("view", "the view is not an object")]);
        };

        let known = |name: &str| declared.iter().any(|column| column == name);

        let mut sort = Vec::new();
        if let Some(entries) = object.get("sort") {
            match entries.as_array() {
                Some(entries) => {
                    for entry in entries {
                        let field = entry.get("field").and_then(Value::as_str);
                        let direction = entry
                            .get("direction")
                            .and_then(Value::as_str)
                            .unwrap_or("asc");
                        match field {
                            Some(field) if known(field) => {
                                if direction == "asc" || direction == "desc" {
                                    sort.push((field.to_owned(), direction.to_owned()));
                                } else {
                                    problems.push(ViewProblem::new(
                                        "sort",
                                        format!("{field}: {direction} is not a direction"),
                                    ));
                                }
                            }
                            Some(field) => problems.push(ViewProblem::new(
                                "sort",
                                format!("{field} is not a column of this grid"),
                            )),
                            None => {
                                problems.push(ViewProblem::new("sort", "a sort key has no field"))
                            }
                        }
                    }
                }
                None => problems.push(ViewProblem::new("sort", "sort is not a list")),
            }
        }

        let mut filters = Vec::new();
        if let Some(entries) = object.get("filters") {
            match entries.as_array() {
                Some(entries) => {
                    for entry in entries {
                        let column = entry.get("column").and_then(Value::as_str);
                        let op = entry.get("op").and_then(Value::as_str).unwrap_or("eq");
                        let text = entry.get("value").and_then(Value::as_str).unwrap_or("");
                        match (column, FilterOp::parse(op)) {
                            (Some(column), Some(op)) if known(column) => {
                                filters.push(FilterEntry {
                                    column: column.to_owned(),
                                    op,
                                    value: text.to_owned(),
                                });
                            }
                            (Some(column), Some(_)) => problems.push(ViewProblem::new(
                                "filters",
                                format!("{column} is not a column of this grid"),
                            )),
                            (Some(column), None) => problems.push(ViewProblem::new(
                                "filters",
                                format!("{column}: {op} is not an operator"),
                            )),
                            (None, _) => {
                                problems.push(ViewProblem::new("filters", "a filter has no column"))
                            }
                        }
                    }
                }
                None => problems.push(ViewProblem::new("filters", "filters is not a list")),
            }
        }

        let mut columns = ColumnLayout::default();
        if let Some(layout) = object.get("columns").and_then(Value::as_object) {
            let names = |key: &str, problems: &mut Vec<ViewProblem>| -> Vec<String> {
                layout
                    .get(key)
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|name| name.as_str())
                            .filter(|name| {
                                if known(name) {
                                    true
                                } else {
                                    problems.push(ViewProblem::new(
                                        "columns",
                                        format!("{name} is not a column of this grid"),
                                    ));
                                    false
                                }
                            })
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            columns.set_order(names("order", &mut problems));
            for name in names("hidden", &mut problems) {
                columns.set_hidden(&name, true);
            }
            if let Some(widths) = layout.get("widths").and_then(Value::as_object) {
                for (name, width) in widths {
                    match (known(name), width.as_u64()) {
                        (true, Some(width)) => columns.set_width(name, width as u32),
                        (false, _) => problems.push(ViewProblem::new(
                            "columns",
                            format!("{name} is not a column of this grid"),
                        )),
                        (true, None) => problems.push(ViewProblem::new(
                            "columns",
                            format!("{name}: the width is not a number"),
                        )),
                    }
                }
            }
        }

        let density = object
            .get("density")
            .and_then(Value::as_str)
            .unwrap_or("normal");
        let density = if DENSITY_NAMES.contains(&density.trim()) {
            density.trim().to_owned()
        } else {
            "normal".to_owned()
        };

        let mut group = Vec::new();
        if let Some(entries) = object.get("group") {
            match entries.as_array() {
                Some(entries) => {
                    for name in entries.iter().filter_map(Value::as_str) {
                        if known(name) {
                            group.push(name.to_owned());
                        } else {
                            problems.push(ViewProblem::new(
                                "group",
                                format!("{name} is not a column of this grid"),
                            ));
                        }
                    }
                    if group.len() > MAX_GROUP_LEVELS {
                        problems.push(ViewProblem::new(
                            "group",
                            format!("{} levels, at most {}", group.len(), MAX_GROUP_LEVELS),
                        ));
                    }
                }
                None => problems.push(ViewProblem::new("group", "group is not a list")),
            }
        }
        // An open group names keys, not columns, so there is nothing to check
        // it against before the groups are counted: a path that matches no
        // group simply opens nothing (the same rule as a filter that hides it).
        let expanded = object
            .get("expanded")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_array)
                    .filter(|path| !path.is_empty() && path.len() <= group.len().max(1))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // The aggregate names are checked here; whether a column's *type*
        // allows one is only known once a result has come, and is checked then.
        let mut aggregates = Vec::new();
        if let Some(entries) = object.get("aggregates") {
            match entries.as_object() {
                Some(entries) => {
                    for (column, function) in entries {
                        let function = function.as_str().unwrap_or("");
                        if !known(column) {
                            problems.push(ViewProblem::new(
                                "aggregates",
                                format!("{column} is not a column of this grid"),
                            ));
                        } else if crate::summary::aggregate_from(function).is_none() {
                            problems.push(ViewProblem::new(
                                "aggregates",
                                format!("{column}: {function} is not an aggregate"),
                            ));
                        } else {
                            aggregates.push((column.clone(), function.to_owned()));
                        }
                    }
                    aggregates.sort_unstable();
                }
                None => problems.push(ViewProblem::new(
                    "aggregates",
                    "aggregates is not an object",
                )),
            }
        }

        let filter_row = object
            .get("filterRow")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        // Facets name columns; which *kind* each is only the configuration
        // knows, so the selection is kept as written and read against it later.
        let mut facets = Map::new();
        if let Some(entries) = object.get("facets") {
            match entries.as_object() {
                Some(entries) => {
                    for (column, selection) in entries {
                        if !known(column) {
                            problems.push(ViewProblem::new(
                                "facets",
                                format!("{column} is not a column of this grid"),
                            ));
                        } else if crate::facets::Selection::from_json(selection).is_none() {
                            problems.push(ViewProblem::new(
                                "facets",
                                format!("{column}: not a facet selection"),
                            ));
                        } else {
                            facets.insert(column.clone(), selection.clone());
                        }
                    }
                }
                None => problems.push(ViewProblem::new("facets", "facets is not an object")),
            }
        }

        // Keys name records, not columns: whether one exists only the source
        // knows, and the element asks it. Here only the shape is checked — a
        // key is a string, a number or a boolean.
        let mut selected = Vec::new();
        if let Some(keys) = object.get("selected") {
            match keys.as_array() {
                Some(keys)
                    if keys
                        .iter()
                        .all(|key| key.is_string() || key.is_number() || key.is_boolean()) =>
                {
                    // As written: the element puts them in order
                    // ([`selected_keys`]); the engine, which answers a view's
                    // query, has no use for them.
                    selected = keys.clone();
                }
                _ => problems.push(ViewProblem::new(
                    "selected",
                    "selected is not a list of keys",
                )),
            }
        }

        if problems.is_empty() {
            Ok(Self {
                sort,
                filters,
                columns,
                density,
                group,
                expanded,
                aggregates,
                filter_row,
                facets: Value::Object(facets),
                selected,
            })
        } else {
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> Vec<String> {
        ["id", "customer", "amount"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn filled() -> GridView {
        let mut columns = ColumnLayout::default();
        columns.set_order(vec!["amount".to_owned(), "id".to_owned()]);
        columns.set_hidden("customer", true);
        columns.set_width("amount", 180);
        GridView {
            sort: vec![("amount".to_owned(), "desc".to_owned())],
            filters: vec![FilterEntry {
                column: "customer".to_owned(),
                op: FilterOp::IsNull,
                value: String::new(),
            }],
            columns,
            density: "compact".to_owned(),
            group: vec!["customer".to_owned()],
            expanded: vec![vec![Value::Null], vec![Value::String("Alpha".to_owned())]],
            aggregates: vec![("amount".to_owned(), "avg".to_owned())],
            filter_row: false,
            facets: opengrid_json::json!({ "customer": { "values": [null, "Alpha"] } }),
            selected: Vec::new(),
        }
    }

    /// The point of the whole module: a view written out and read back is the
    /// same view. Without this, "save a view" is a guess.
    #[test]
    fn a_view_survives_the_round_trip() {
        let view = filled();
        let back = GridView::from_json(&view.to_json(), &declared()).expect("reads back");
        assert_eq!(back, view);
    }

    /// The empty view is a view, not an error — a grid nobody touched yet.
    #[test]
    fn an_untouched_grid_round_trips_too() {
        let view = GridView {
            density: "normal".to_owned(),
            ..GridView::default()
        };
        let back = GridView::from_json(&view.to_json(), &declared()).expect("reads back");
        assert_eq!(back, view);
    }

    /// `facets` is written before point 66 fills it, so a view stored today
    /// still reads afterwards; `group` and `expanded` carry point 62's state.
    #[test]
    fn the_later_fields_are_already_there() {
        let json = filled().to_json();
        for field in ["group", "expanded", "facets"] {
            assert!(json.get(field).is_some(), "{field} is missing from a view");
        }
        assert_eq!(json["group"], opengrid_json::json!(["customer"]));
        // A NULL key is a key: the NULL group can be open like any other.
        assert_eq!(json["expanded"][0], opengrid_json::json!([null]));
    }

    /// An aggregate that is not one, or on a column that is not there, is named.
    #[test]
    fn a_wrong_aggregate_is_refused() {
        for aggregates in [
            opengrid_json::json!({ "amount": "median" }),
            opengrid_json::json!({ "nope": "sum" }),
        ] {
            let json = opengrid_json::json!({ "aggregates": aggregates });
            let problems = GridView::from_json(&json, &declared()).expect_err("is refused");
            assert_eq!(problems[0].field, "aggregates");
        }
    }

    /// Grouping by a column the grid does not have, or by four, is refused —
    /// by three it is not.
    #[test]
    fn a_grouping_the_grid_cannot_have_is_refused() {
        let mut declared = declared();
        declared.push("country".to_owned());
        for group in [
            opengrid_json::json!(["nope"]),
            opengrid_json::json!(["id", "customer", "amount", "country"]),
        ] {
            let json = opengrid_json::json!({ "group": group });
            let problems = GridView::from_json(&json, &declared).expect_err("is refused");
            assert_eq!(problems[0].field, "group");
        }
        let json = opengrid_json::json!({ "group": ["id", "customer", "amount"] });
        let view = GridView::from_json(&json, &declared).expect("three levels are a view");
        assert_eq!(view.group, ["id", "customer", "amount"]);
    }

    /// Nothing is dropped silently: a view against a different data source is
    /// reported, not half-applied.
    #[test]
    fn an_unknown_column_is_named_everywhere_it_appears() {
        let json = opengrid_json::json!({
            "sort": [{ "field": "nope", "direction": "asc" }],
            "filters": [{ "column": "nope", "op": "eq", "value": "x" }],
            "columns": { "order": ["nope"], "hidden": ["nope"], "widths": { "nope": 100 } },
            "density": "normal",
        });
        let problems = GridView::from_json(&json, &declared()).expect_err("is refused");
        let fields: Vec<&str> = problems.iter().map(|p| p.field.as_str()).collect();
        assert!(fields.contains(&"sort"));
        assert!(fields.contains(&"filters"));
        assert!(fields.contains(&"columns"));
        for problem in &problems {
            assert!(
                problem.reason.contains("nope"),
                "{problem:?} does not say which column"
            );
        }
    }

    /// A direction that is neither is a mistake worth naming — silently sorting
    /// ascending would look like it worked.
    #[test]
    fn a_direction_that_is_neither_is_refused() {
        let json = opengrid_json::json!({ "sort": [{ "field": "id", "direction": "sideways" }] });
        let problems = GridView::from_json(&json, &declared()).expect_err("is refused");
        assert_eq!(problems.len(), 1);
        assert!(problems[0].reason.contains("sideways"));
    }

    /// An unknown density is the `normal` one, exactly as the attribute is —
    /// one rule, in one place.
    #[test]
    fn an_unknown_density_is_the_normal_one() {
        let json = opengrid_json::json!({ "density": "roomy" });
        let view = GridView::from_json(&json, &declared()).expect("reads");
        assert_eq!(view.density, "normal");
    }

    /// A selection by key is part of a view (issue #322); one by position is
    /// not, and a view that carries one does not smuggle it back in.
    #[test]
    fn a_view_holds_a_selection_by_key_and_none_by_position() {
        // Nothing selected by key: the field is not written at all.
        assert!(filled().to_json().get("selected").is_none());
        assert!(filled().to_json().get("selection").is_none());

        // The keys round-trip; in order they are each once, numbers first.
        let json = opengrid_json::json!({ "density": "normal", "selected": [7, "b", 3, 7, "a"] });
        let mut view = GridView::from_json(&json, &declared()).expect("reads");
        assert_eq!(view.selected.len(), 5);
        view.selected = selected_keys(view.selected);
        assert_eq!(
            view.selected,
            opengrid_json::json!([3, 7, "a", "b"])
                .as_array()
                .unwrap()
                .clone()
        );
        assert_eq!(
            view.to_json()["selected"],
            opengrid_json::json!([3, 7, "a", "b"])
        );
        // Numbers by value, not by text.
        let numbers = selected_keys(
            opengrid_json::json!([10, 9, 100])
                .as_array()
                .unwrap()
                .clone(),
        );
        assert_eq!(Value::Array(numbers), opengrid_json::json!([9, 10, 100]));
        let again = GridView::from_json(&view.to_json(), &declared()).expect("reads");
        assert_eq!(again, view);

        // Positions under the old name are not read.
        let smuggled = opengrid_json::json!({ "density": "normal", "selection": [1, 2, 3] });
        let view = GridView::from_json(&smuggled, &declared()).expect("reads");
        assert_eq!(
            view,
            GridView {
                density: "normal".to_owned(),
                ..GridView::default()
            }
        );
    }

    /// Keys are untrusted input: anything but a list of scalars is named, and
    /// the view is not applied.
    #[test]
    fn a_selection_that_is_not_a_list_of_keys_is_a_problem() {
        for bad in [
            opengrid_json::json!({ "selected": 7 }),
            opengrid_json::json!({ "selected": [[1]] }),
            opengrid_json::json!({ "selected": [null] }),
            opengrid_json::json!({ "selected": [{ "id": 1 }] }),
        ] {
            let problems = GridView::from_json(&bad, &declared()).expect_err("refused");
            assert_eq!(problems[0].field, "selected", "{bad}");
        }
    }
}
