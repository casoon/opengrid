//! The `<opengrid-table>` model: query building, result parsing and the native
//! `<table>` as pure patch data.
//!
//! Point 14 fills the empty skeleton of point 13 with real columns and rows.
//! Everything in this module is portable data: the query JSON is built from the
//! host attributes, the engine's result JSON is parsed into a [`TableModel`] and
//! the markup is computed as [`Patch`]es. The same functions run on the host in
//! unit tests and in the browser through the renderer
//! (plan/spezifikation/11-crates.md §Portabilität).
//!
//! Table mode stays native HTML with maximum semantics
//! (plan/spezifikation/09-accessibility.md §Zwei Rendering-Modi): `<caption>`,
//! `<thead>`/`<tbody>`, `<th scope="col">` and a `<button>` per header carrying
//! `aria-sort` on the `<th>`. Sorting is single-column — table mode is display,
//! not interaction (plan/spezifikation/02-query-modell.md §Struktur).

use serde_json::{Value, json};

use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

/// The custom element name (E1).
pub const TABLE_TAG: &str = "opengrid-table";

/// The host attribute naming the source in the query's `source` field.
pub const DATASOURCE_ATTRIBUTE: &str = "datasource";

/// The host attribute listing the selected fields, comma-separated.
pub const COLUMNS_ATTRIBUTE: &str = "columns";

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[LABEL_ATTRIBUTE, DATASOURCE_ATTRIBUTE, COLUMNS_ATTRIBUTE];

/// The direction of the single-column sort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

impl SortDirection {
    /// The wire value in the query's `sort[].direction` field.
    pub fn as_query_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }

    /// The ARIA token for `aria-sort`.
    pub fn as_aria_sort(self) -> &'static str {
        match self {
            Self::Asc => "ascending",
            Self::Desc => "descending",
        }
    }
}

/// One output column with its already formatted cell text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnModel {
    /// The output name.
    pub name: String,
    /// One text per row, in row order.
    pub values: Vec<String>,
}

/// The parsed result the table renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableModel {
    /// The output columns, in schema order (E14).
    pub columns: Vec<ColumnModel>,
}

/// The nodes [`build_table`] creates, so a later cycle can patch them in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableNodes {
    /// The `<table>`.
    pub table: NodeId,
    /// The `<caption>`.
    pub caption: NodeId,
}

/// Splits the `columns` attribute into field names.
///
/// Whitespace around each name is trimmed and empty entries are dropped, so a
/// trailing comma is harmless.
pub fn parse_columns(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Builds the query JSON for a table render.
///
/// `sort` is omitted entirely when nothing is sorted, and carried as one
/// `{ "field", "direction" }` object otherwise
/// (plan/spezifikation/02-query-modell.md §JSON-Vertrag).
pub fn query_json(source: &str, columns: &[String], sort: Option<(&str, SortDirection)>) -> String {
    let mut query = serde_json::Map::new();
    query.insert("source".to_owned(), Value::String(source.to_owned()));
    query.insert(
        "select".to_owned(),
        Value::Array(columns.iter().cloned().map(Value::String).collect()),
    );
    if let Some((field, direction)) = sort {
        query.insert(
            "sort".to_owned(),
            json!([{ "field": field, "direction": direction.as_query_str() }]),
        );
    }
    Value::Object(query).to_string()
}

/// Parses the engine's result JSON `{ "columns": [ { "name", "values" } ] }`.
///
/// Only the column envelope matters here; `total_count`/`row_count` are ignored
/// because table mode shows the whole page. Values become their display text:
/// JSON `null` is empty text, everything else its scalar form (decimals already
/// arrive as strings, E13).
pub fn parse_result(result_json: &str) -> Result<TableModel, String> {
    let value: Value =
        serde_json::from_str(result_json).map_err(|error| format!("result JSON: {error}"))?;
    let columns = value
        .get("columns")
        .and_then(Value::as_array)
        .ok_or_else(|| "result has no columns".to_owned())?;

    let mut parsed = Vec::with_capacity(columns.len());
    for column in columns {
        let name = column
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "a column has no name".to_owned())?
            .to_owned();
        let values = column
            .get("values")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("column {name:?} has no values"))?;
        parsed.push(ColumnModel {
            name,
            values: values.iter().map(value_text).collect(),
        });
    }
    Ok(TableModel { columns: parsed })
}

/// The display text of one wire value.
fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

/// The visible sort mark for an `aria-sort` token, empty when unsorted.
///
/// The same glyphs as the grid (point 49), from the same place, so the two
/// elements cannot drift apart. The leading no-break space is the separator:
/// table mode ships **no** stylesheet — the page styles a native table — so
/// there is nowhere to put a margin without making the table's first shadow
/// rule a default every page would have to override. The space sits inside the
/// `aria-hidden` span, so nothing reads it.
fn direction_mark(aria_sort: &str) -> String {
    let glyph = match aria_sort {
        "ascending" => crate::grid::ASCENDING_GLYPH,
        "descending" => crate::grid::DESCENDING_GLYPH,
        _ => return String::new(),
    };
    format!("\u{a0}{glyph}")
}

/// Appends a text-only `<p role="alert">` error to `buffer`.
///
/// Table mode has no status area — it renders a result, not an interactive grid
/// — so a failure replaces it with this alert. `message` is the finished
/// sentence from the component's texts (point 48) and `lang` its language, which
/// is written onto the paragraph so it is announced in that language; an empty
/// `lang` leaves the document's.
pub fn build_error(buffer: &mut PatchBuffer, nodes: &mut NodeAllocator, message: &str, lang: &str) {
    let paragraph = element(buffer, nodes, Some(NodeId::ROOT), "p");
    buffer.push(Patch::SetAttribute {
        node: paragraph,
        name: "role".to_owned(),
        value: "alert".to_owned(),
    });
    if !lang.trim().is_empty() {
        buffer.push(Patch::SetAttribute {
            node: paragraph,
            name: "lang".to_owned(),
            value: lang.to_owned(),
        });
    }
    buffer.push(Patch::SetText {
        node: paragraph,
        text: message.to_owned(),
    });
}

/// Appends the table to `buffer` and answers its nodes.
///
/// Without a model this is the empty skeleton of point 13 (`<table>` and
/// `<caption>` only). With a model it adds `<thead>` with one sortable
/// `<th scope="col">` per column and the `<tbody>` rows. `sort` names the one
/// sorted column, if any; every other header gets `aria-sort="none"`.
pub fn build_table(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    label: Option<&str>,
    model: Option<&TableModel>,
    sort: Option<(&str, SortDirection)>,
) -> TableNodes {
    let table = element(buffer, nodes, Some(NodeId::ROOT), "table");
    if let Some((name, value)) = mirror_label(label) {
        buffer.push(Patch::SetAttribute {
            node: table,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }

    let caption = element(buffer, nodes, Some(table), "caption");
    buffer.push(Patch::SetText {
        node: caption,
        text: label.unwrap_or_default().to_owned(),
    });

    if let Some(model) = model {
        let thead = element(buffer, nodes, Some(table), "thead");
        let header_row = element(buffer, nodes, Some(thead), "tr");
        for column in &model.columns {
            let th = element(buffer, nodes, Some(header_row), "th");
            buffer.push(Patch::SetAttribute {
                node: th,
                name: "scope".to_owned(),
                value: "col".to_owned(),
            });
            buffer.push(Patch::SetAttribute {
                node: th,
                name: "data-column".to_owned(),
                value: column.name.clone(),
            });
            let aria_sort = match sort {
                Some((field, direction)) if field == column.name => direction.as_aria_sort(),
                _ => "none",
            };
            buffer.push(Patch::SetAttribute {
                node: th,
                name: "aria-sort".to_owned(),
                value: aria_sort.to_owned(),
            });

            let button = element(buffer, nodes, Some(th), "button");
            buffer.push(Patch::SetAttribute {
                node: button,
                name: "type".to_owned(),
                value: "button".to_owned(),
            });
            buffer.push(Patch::SetAttribute {
                node: button,
                name: "data-column".to_owned(),
                value: column.name.clone(),
            });
            // The name lives in its own span so the direction mark can sit
            // beside it without joining the button's accessible name — the
            // direction reaches assistive technology through `aria-sort` on the
            // `<th>`, and must not be announced a second time (point 50, the
            // pattern of point 49).
            let name = element(buffer, nodes, Some(button), "span");
            buffer.push(Patch::SetText {
                node: name,
                text: column.name.clone(),
            });
            let mark = crate::grid::marker(buffer, nodes, button, "sort-direction");
            buffer.push(Patch::SetText {
                node: mark,
                text: direction_mark(aria_sort),
            });
        }

        let rows = model.columns.first().map_or(0, |first| first.values.len());
        let tbody = element(buffer, nodes, Some(table), "tbody");
        for row in 0..rows {
            let tr = element(buffer, nodes, Some(tbody), "tr");
            for column in &model.columns {
                let td = element(buffer, nodes, Some(tr), "td");
                buffer.push(Patch::SetText {
                    node: td,
                    text: column.values.get(row).cloned().unwrap_or_default(),
                });
            }
        }
    }

    TableNodes { table, caption }
}

/// Creates an element and appends it to `parent`, in patch order.
fn element(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: Option<NodeId>,
    tag: &str,
) -> NodeId {
    let node = nodes.alloc();
    buffer.push(Patch::CreateElement {
        node,
        tag: tag.to_owned(),
    });
    if let Some(parent) = parent {
        buffer.push(Patch::AppendChild {
            parent,
            child: node,
        });
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sorted column shows its direction; the others show nothing, and the
    /// name stays in its own span so the button's accessible name is the column
    /// (point 50).
    #[test]
    fn the_sorted_header_shows_its_direction() {
        let model = TableModel {
            columns: vec![
                ColumnModel {
                    name: "customer".to_owned(),
                    values: vec!["Alpha".to_owned()],
                },
                ColumnModel {
                    name: "qty".to_owned(),
                    values: vec!["1".to_owned()],
                },
            ],
        };

        let marks = |sort: Option<(&str, SortDirection)>| -> Vec<String> {
            let mut nodes = NodeAllocator::new();
            let mut buffer = PatchBuffer::new();
            build_table(&mut buffer, &mut nodes, None, Some(&model), sort);
            // Every span that is a sort mark, in document order.
            let marked: Vec<NodeId> = buffer
                .patches()
                .iter()
                .filter_map(|patch| match patch {
                    Patch::SetAttribute { node, name, value }
                        if name == "part" && value == "sort-direction" =>
                    {
                        Some(*node)
                    }
                    _ => None,
                })
                .collect();
            marked
                .iter()
                .map(|mark| {
                    buffer
                        .patches()
                        .iter()
                        .find_map(|patch| match patch {
                            Patch::SetText { node, text } if node == mark => Some(text.clone()),
                            _ => None,
                        })
                        .expect("every mark is written")
                })
                .collect()
        };

        // The name is in its own span, not on the button: that is what keeps the
        // mark out of the button's accessible name.
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_table(&mut buffer, &mut nodes, None, Some(&model), None);
        let buttons: Vec<NodeId> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::CreateElement { node, tag } if tag == "button" => Some(*node),
                _ => None,
            })
            .collect();
        assert_eq!(buttons.len(), 2);
        assert!(
            !buffer.patches().iter().any(|patch| matches!(
                patch,
                Patch::SetText { node, .. } if buttons.contains(node)
            )),
            "the column name must live in a span, not directly on the button"
        );

        assert_eq!(marks(None), ["", ""]);
        assert_eq!(
            marks(Some(("customer", SortDirection::Asc))),
            ["\u{a0}▲", ""]
        );
        assert_eq!(marks(Some(("qty", SortDirection::Desc))), ["", "\u{a0}▼"]);

        // The marks themselves are hidden from assistive technology: the
        // direction is already on the `<th>` as `aria-sort`.
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_table(
            &mut buffer,
            &mut nodes,
            None,
            Some(&model),
            Some(("customer", SortDirection::Asc)),
        );
        let marked: Vec<NodeId> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { node, name, value }
                    if name == "part" && value == "sort-direction" =>
                {
                    Some(*node)
                }
                _ => None,
            })
            .collect();
        assert_eq!(marked.len(), 2);
        for mark in &marked {
            assert!(
                buffer.patches().iter().any(|patch| matches!(
                    patch,
                    Patch::SetAttribute { node, name, value }
                        if node == mark && name == "aria-hidden" && value == "true"
                )),
                "a sort mark must be hidden from assistive technology"
            );
        }
    }

    /// The alert carries the language of the sentence, and only when there is
    /// one to carry (point 48).
    #[test]
    fn the_error_declares_the_language_of_its_sentence() {
        let lang_of = |lang: &str| -> Option<String> {
            let mut nodes = NodeAllocator::new();
            let mut buffer = PatchBuffer::new();
            build_error(&mut buffer, &mut nodes, "Die Daten fehlen.", lang);
            buffer.patches().iter().find_map(|patch| match patch {
                Patch::SetAttribute { name, value, .. } if name == "lang" => Some(value.clone()),
                _ => None,
            })
        };
        assert_eq!(lang_of("de"), Some("de".to_owned()));
        // An empty language leaves the document's in place.
        assert_eq!(lang_of(""), None);
    }

    /// Column names are trimmed and empty entries dropped.
    #[test]
    fn columns_attribute_is_a_trimmed_list() {
        assert_eq!(
            parse_columns(Some(" customer , amount ,qty,")),
            ["customer", "amount", "qty"]
        );
        assert!(parse_columns(None).is_empty());
        assert!(parse_columns(Some("  ")).is_empty());
    }

    /// An unsorted query omits `sort`; the select keeps the column order.
    #[test]
    fn an_unsorted_query_has_no_sort_key() {
        let query = query_json("orders", &["a".to_owned(), "b".to_owned()], None);
        let value: Value = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(value["source"], "orders");
        assert_eq!(value["select"], json!(["a", "b"]));
        assert!(value.get("sort").is_none());
    }

    /// A sorted query carries exactly one sort object.
    #[test]
    fn a_sorted_query_names_one_direction() {
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            Some(("customer", SortDirection::Desc)),
        );
        let value: Value = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([{ "field": "customer", "direction": "desc" }])
        );
    }

    /// The wire result becomes text: null empty, decimals already strings.
    #[test]
    fn result_json_parses_into_a_table_model() {
        let result = r#"{
            "total_count": 3,
            "row_count": 3,
            "columns": [
                { "name": "customer", "values": ["Alpha", null, ""] },
                { "name": "amount", "values": ["10.00", "20.00", null] }
            ]
        }"#;
        let model = parse_result(result).expect("parses");
        assert_eq!(model.columns.len(), 2);
        assert_eq!(model.columns[0].name, "customer");
        assert_eq!(model.columns[0].values, ["Alpha", "", ""]);
        assert_eq!(model.columns[1].values, ["10.00", "20.00", ""]);
    }

    /// A malformed result is an error, not a panic.
    #[test]
    fn a_result_without_columns_is_an_error() {
        assert!(parse_result("{}").is_err());
        assert!(parse_result("not json").is_err());
    }

    /// A data render is one patch list with scoped header buttons and
    /// `aria-sort` on the sorted column only.
    #[test]
    fn a_sorted_table_marks_exactly_one_header() {
        let model = TableModel {
            columns: vec![
                ColumnModel {
                    name: "customer".to_owned(),
                    values: vec!["Alpha".to_owned(), "Beta".to_owned()],
                },
                ColumnModel {
                    name: "qty".to_owned(),
                    values: vec!["1".to_owned(), "2".to_owned()],
                },
            ],
        };
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_table(
            &mut buffer,
            &mut nodes,
            Some("Bestellungen"),
            Some(&model),
            Some(("customer", SortDirection::Asc)),
        );

        let sorts: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { name, value, .. } if name == "aria-sort" => {
                    Some(value.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(sorts, ["ascending", "none"]);

        assert!(
            buffer
                .patches()
                .iter()
                .any(|patch| matches!(patch, Patch::SetText { text, .. } if text == "Alpha")),
            "cells carry their text"
        );
        assert!(!buffer.is_empty(), "the whole table is one patch list");
    }

    /// The skeleton of point 13 is unchanged: table plus caption, no cells.
    #[test]
    fn a_model_without_data_is_still_the_skeleton() {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_table(&mut buffer, &mut nodes, None, None, None);

        assert!(!buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::CreateElement { tag, .. }
                if tag == "thead" || tag == "tbody"
        )));
    }
}
