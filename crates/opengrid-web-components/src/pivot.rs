//! The `<opengrid-pivot>` model: query building, result parsing and the native
//! `<table>` as pure patch data (plan point 32).
//!
//! # Why Table Mode
//!
//! 09-accessibility.md says it outright: not every table is forced into a
//! complex ARIA grid. A two-level column header made of `colspan` and
//! `scope="colgroup"` is solved HTML that every screen reader knows, while a
//! **virtual** pivot with spanning headers is, by 06-pivot.md's own reckoning,
//! the riskiest thing in the project (R5). V1 therefore renders the whole pivot
//! — no virtualization, no paging — and the limits of point 30 are what make
//! that safe: 256 columns, 2000 rows, and an error beyond them.
//!
//! Everything here is portable data, so the same functions run in unit tests on
//! the host and in the browser through the renderer.

use serde_json::{Value, json};

use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

use crate::shared::element;

use crate::texts::GridTexts;

/// The custom element name (E1).
pub const PIVOT_TAG: &str = "opengrid-pivot";

/// The host attribute naming the source.
pub const DATASOURCE_ATTRIBUTE: &str = "datasource";

/// The host attribute listing the row dimensions, comma-separated.
pub const ROWS_ATTRIBUTE: &str = "rows";

/// The host attribute listing the column dimensions, comma-separated.
pub const COLUMNS_ATTRIBUTE: &str = "columns";

/// The host attribute carrying the measures, as the JSON of the wire form.
///
/// `values='[{"field":"qty","fn":"sum","as":"total"}]'`. Deliberately the
/// contract's own shape rather than an invented mini-syntax: one notation to
/// learn, and nothing to keep in step with the query model.
pub const VALUES_ATTRIBUTE: &str = "values";

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[
    LABEL_ATTRIBUTE,
    DATASOURCE_ATTRIBUTE,
    ROWS_ATTRIBUTE,
    COLUMNS_ATTRIBUTE,
    VALUES_ATTRIBUTE,
];

/// A comma-separated attribute as a list of names, empties dropped.
pub fn parse_dimensions(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Builds the pivot request JSON from the host attributes.
///
/// The measures are passed through as they were written: if they are not JSON,
/// or not an array, the element says so instead of sending something the server
/// would reject with a less helpful sentence.
pub fn pivot_json(
    source: &str,
    rows: &[String],
    columns: &[String],
    values: Option<&str>,
) -> Result<String, String> {
    let raw = values.unwrap_or_default().trim();
    if raw.is_empty() {
        return Err("the values attribute is empty: a pivot needs a measure".to_owned());
    }
    let values: Value = serde_json::from_str(raw)
        .map_err(|error| format!("the values attribute is not JSON: {error}"))?;
    if !values.is_array() {
        return Err("the values attribute must be a JSON array of measures".to_owned());
    }
    Ok(json!({
        "source": source,
        "rows": rows,
        "columns": columns,
        "values": values,
    })
    .to_string())
}

/// One generated column: what it stands for, and the measure it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnHeader {
    /// The column-dimension values; `None` is NULL. Empty when the pivot has
    /// no column dimension.
    pub path: Vec<Option<String>>,
    /// The measure alias.
    pub measure: String,
}

/// A pivot answer, ready to render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PivotModel {
    /// The row dimensions, outermost first.
    pub row_dimensions: Vec<String>,
    pub columns: Vec<ColumnHeader>,
    /// Per row: how many row dimensions are set. See
    /// [`opengrid_pivot::PivotResult::row_levels`] — a subtotal is marked by
    /// this number, never by a NULL.
    pub levels: Vec<u16>,
    /// Per row, one cell per row dimension and then per column. `None` is
    /// NULL — which a **header** must not render as nothing (see
    /// [`GridTexts::dimension`]).
    pub rows: Vec<Vec<Option<String>>>,
}

impl PivotModel {
    /// How many measures repeat under each column value.
    pub fn measures(&self) -> usize {
        if self.columns.is_empty() {
            return 0;
        }
        let first = &self.columns[0].path;
        self.columns
            .iter()
            .take_while(|column| &column.path == first)
            .count()
    }

    /// The distinct column-dimension values, in order.
    pub fn groups(&self) -> Vec<&[Option<String>]> {
        let mut groups: Vec<&[Option<String>]> = Vec::new();
        for column in &self.columns {
            if groups.last() != Some(&column.path.as_slice()) {
                groups.push(&column.path);
            }
        }
        groups
    }
}

/// Reads the pivot wire form of point 53.
pub fn parse_result(json: &str) -> Result<PivotModel, String> {
    let body: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;

    let row_dimensions: Vec<String> = body["row_dimensions"]
        .as_array()
        .ok_or("result has no row_dimensions")?
        .iter()
        .map(|name| name.as_str().unwrap_or_default().to_owned())
        .collect();

    let columns: Vec<ColumnHeader> = body["columns"]
        .as_array()
        .ok_or("result has no columns")?
        .iter()
        .map(|column| ColumnHeader {
            path: column["path"]
                .as_array()
                .map(|path| path.iter().map(render).collect())
                .unwrap_or_default(),
            measure: column["measure"].as_str().unwrap_or_default().to_owned(),
        })
        .collect();

    let levels: Vec<u16> = body["levels"]
        .as_array()
        .ok_or("result has no levels")?
        .iter()
        .map(|level| level.as_u64().unwrap_or_default() as u16)
        .collect();

    // The cells are the ordinary result form (E17): column-oriented, so they
    // are transposed here and nowhere else.
    let cells = body["result"]["columns"]
        .as_array()
        .ok_or("result has no cells")?;
    let mut rows = Vec::with_capacity(levels.len());
    for index in 0..levels.len() {
        rows.push(
            cells
                .iter()
                .map(|column| column["values"].get(index).and_then(render))
                .collect(),
        );
    }

    Ok(PivotModel {
        row_dimensions,
        columns,
        levels,
        rows,
    })
}

/// A JSON scalar as a cell, keeping NULL apart from the empty string.
///
/// A data cell renders both as nothing; a **header** must not (S14 —
/// `""` is not NULL, and an empty header cell is announced as silence).
fn render(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// A data cell as text. NULL and the empty string both render as nothing —
/// there is a value or there is not, and the column header says which column.
fn cell(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

/// Builds the whole pivot as one patch list.
///
/// Two header rows when there is a column dimension — values with `colspan` and
/// `scope="colgroup"`, measures under them with `scope="col"` — and one when
/// there is not. Every data row starts with `<th scope="row">`, so each cell is
/// announced with the row it is in and the column it is under.
pub fn build_pivot(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    label: Option<&str>,
    model: Option<&PivotModel>,
    status: &str,
    state: &str,
    texts: &GridTexts,
) {
    let layout = element(buffer, nodes, Some(NodeId::ROOT), "div");
    attribute(buffer, layout, "part", "layout");

    let status_node = element(buffer, nodes, Some(layout), "p");
    attribute(buffer, status_node, "part", "status");
    attribute(buffer, status_node, "role", "status");
    attribute(buffer, status_node, "aria-live", "polite");
    attribute(buffer, status_node, "data-state", state);
    if !texts.lang.is_empty() {
        attribute(buffer, status_node, "lang", &texts.lang);
    }
    buffer.push(Patch::SetText {
        node: status_node,
        text: status.to_owned(),
    });

    let table = element(buffer, nodes, Some(layout), "table");
    if let Some((name, value)) = mirror_label(label) {
        attribute(buffer, table, name, value);
    }
    let caption = element(buffer, nodes, Some(table), "caption");
    buffer.push(Patch::SetText {
        node: caption,
        text: label.unwrap_or_default().to_owned(),
    });

    if let Some(model) = model {
        let measures = model.measures();
        let groups = model.groups();
        let nested = groups.first().is_some_and(|path| !path.is_empty());

        let thead = element(buffer, nodes, Some(table), "thead");
        attribute(buffer, thead, "part", "header");
        let first_row = element(buffer, nodes, Some(thead), "tr");
        for name in &model.row_dimensions {
            let th = element(buffer, nodes, Some(first_row), "th");
            attribute(buffer, th, "scope", "col");
            if nested {
                // The dimension name spans both header rows, so the row below
                // holds only the measures.
                attribute(buffer, th, "rowspan", "2");
            }
            buffer.push(Patch::SetText {
                node: th,
                text: name.clone(),
            });
        }

        if nested {
            for group in &groups {
                let th = element(buffer, nodes, Some(first_row), "th");
                attribute(buffer, th, "scope", "colgroup");
                attribute(buffer, th, "colspan", &measures.to_string());
                buffer.push(Patch::SetText {
                    node: th,
                    text: group
                        .iter()
                        .map(|value| texts.dimension(value.as_deref()))
                        .collect::<Vec<_>>()
                        .join(" · "),
                });
            }
            let second_row = element(buffer, nodes, Some(thead), "tr");
            for column in &model.columns {
                let th = element(buffer, nodes, Some(second_row), "th");
                attribute(buffer, th, "scope", "col");
                buffer.push(Patch::SetText {
                    node: th,
                    text: column.measure.clone(),
                });
            }
        } else {
            for column in &model.columns {
                let th = element(buffer, nodes, Some(first_row), "th");
                attribute(buffer, th, "scope", "col");
                buffer.push(Patch::SetText {
                    node: th,
                    text: column.measure.clone(),
                });
            }
        }

        let tbody = element(buffer, nodes, Some(table), "tbody");
        let dimensions = model.row_dimensions.len();
        for (index, cells) in model.rows.iter().enumerate() {
            let level = usize::from(*model.levels.get(index).unwrap_or(&0));
            let tr = element(buffer, nodes, Some(tbody), "tr");
            attribute(buffer, tr, "data-level", &level.to_string());
            // Table Mode ships no stylesheet (the same choice as
            // `<opengrid-table>`), so the page needs a hook to shade a total
            // row. The text says it as well — shading alone is not information
            // (WCAG 1.4.1).
            attribute(
                buffer,
                tr,
                "part",
                if level < dimensions {
                    "row total-row"
                } else {
                    "row"
                },
            );

            if level < dimensions {
                // A subtotal: one header spanning the dimension columns, and it
                // **says** that it is a total (WCAG 1.4.1 — not colour alone).
                attribute(buffer, tr, "data-total", "true");
                let th = element(buffer, nodes, Some(tr), "th");
                attribute(buffer, th, "scope", "row");
                if dimensions > 1 {
                    attribute(buffer, th, "colspan", &dimensions.to_string());
                }
                let text = if level == 0 {
                    texts.total.clone()
                } else {
                    texts
                        .subtotal(&texts.dimension(cells.get(level - 1).and_then(|c| c.as_deref())))
                };
                buffer.push(Patch::SetText { node: th, text });
            } else {
                for value in cells.iter().take(dimensions) {
                    let th = element(buffer, nodes, Some(tr), "th");
                    attribute(buffer, th, "scope", "row");
                    buffer.push(Patch::SetText {
                        node: th,
                        text: texts.dimension(value.as_deref()),
                    });
                }
            }

            for value in cells.iter().skip(dimensions) {
                let td = element(buffer, nodes, Some(tr), "td");
                buffer.push(Patch::SetText {
                    node: td,
                    text: cell(value),
                });
            }
        }
    }
}

fn attribute(buffer: &mut PatchBuffer, node: NodeId, name: &str, value: &str) {
    buffer.push(Patch::SetAttribute {
        node,
        name: name.to_owned(),
        value: value.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{
        "row_dimensions": ["country"],
        "columns": [
            {"path": [2025], "measure": "total"},
            {"path": [2026], "measure": "total"}
        ],
        "levels": [1, 1, 1, 0],
        "result": {
            "total_count": 4,
            "row_count": 4,
            "columns": [
                {"name": "country", "type": "utf8", "nullable": true,
                 "values": ["DE", "FR", null, null]},
                {"name": "total_0", "type": "int64", "nullable": true,
                 "values": [1, null, null, 1]},
                {"name": "total_1", "type": "int64", "nullable": true,
                 "values": [258, 172, 269, 699]}
            ]
        }
    }"#;

    fn model() -> PivotModel {
        parse_result(ANSWER).expect("the pivot wire form parses")
    }

    fn markup(model: Option<&PivotModel>) -> String {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_pivot(
            &mut buffer,
            &mut nodes,
            Some("Orders"),
            model,
            "3 matches",
            "ready",
            &GridTexts::default(),
        );
        format!("{:?}", buffer.patches())
    }

    #[test]
    fn a_measure_list_must_be_json() {
        let rows = vec!["country".to_owned()];
        assert!(pivot_json("orders", &rows, &[], Some(r#"[{"fn":"count","as":"n"}]"#)).is_ok());
        assert!(pivot_json("orders", &rows, &[], None).is_err());
        assert!(pivot_json("orders", &rows, &[], Some("count(qty)")).is_err());
        assert!(
            pivot_json("orders", &rows, &[], Some(r#"{"fn":"count"}"#)).is_err(),
            "an object is not a list of measures"
        );
    }

    #[test]
    fn dimensions_come_from_a_comma_separated_attribute() {
        assert_eq!(
            parse_dimensions(Some(" country , customer ,")),
            vec!["country".to_owned(), "customer".to_owned()]
        );
        assert!(parse_dimensions(None).is_empty());
    }

    /// The wire form of point 53 becomes rows, columns and levels.
    #[test]
    fn the_pivot_wire_form_becomes_a_model() {
        let model = model();
        assert_eq!(model.row_dimensions, vec!["country".to_owned()]);
        assert_eq!(model.measures(), 1, "one measure under each column value");
        assert_eq!(model.groups().len(), 2);
        assert_eq!(model.levels, vec![1, 1, 1, 0]);
        // Row 2 is the NULL **group** and row 3 the grand total. Both carry a
        // NULL in the dimension column; only the level tells them apart (P2).
        assert_eq!(model.levels[2], 1);
        assert_eq!(model.levels[3], 0);
        assert_eq!(model.rows[2][0], None);
        assert_eq!(model.rows[3][0], None);
    }

    /// The two-level column header: values with `colspan`, measures below.
    #[test]
    fn a_column_dimension_gives_a_two_level_header() {
        let markup = markup(Some(&model()));
        assert!(markup.contains("colgroup"), "{markup}");
        assert!(
            markup.contains("rowspan"),
            "the dimension name spans both rows"
        );
        assert!(markup.contains("\"2025\""), "{markup}");
    }

    /// A subtotal says that it is one. Colour alone is not information
    /// (WCAG 1.4.1), and a screen reader has nothing but the text.
    #[test]
    fn a_total_row_is_readable_as_a_total() {
        let markup = markup(Some(&model()));
        assert!(markup.contains("data-total"), "{markup}");
        assert!(
            markup.contains("Total"),
            "the grand total carries its name: {markup}"
        );
    }

    /// Every data row starts with a row header, so a cell is announced with the
    /// row it is in.
    #[test]
    fn every_row_has_a_header() {
        let markup = markup(Some(&model()));
        assert_eq!(
            markup.matches("name: \"scope\", value: \"row\"").count(),
            4,
            "{markup}"
        );
    }

    /// **No header cell is ever empty.** NULL and the empty string are two
    /// different groups (S10, S14) and both need a word: a blank header is
    /// silence to a screen reader and a shrug to everyone else.
    #[test]
    fn a_null_group_and_an_empty_string_group_are_both_named() {
        let texts = GridTexts::default();
        assert_eq!(texts.dimension(None), "(no value)");
        assert_eq!(texts.dimension(Some("")), "(empty)");
        assert_eq!(texts.dimension(Some("DE")), "DE");

        // The fixture's third row is the NULL country group.
        let markup = markup(Some(&model()));
        assert!(
            markup.contains("(no value)"),
            "the NULL group needs a name, and the total is not it"
        );
    }

    /// Without a model there is a status line and an empty table — the state
    /// before the first answer, not an error.
    #[test]
    fn the_skeleton_is_a_status_line_and_an_empty_table() {
        let markup = markup(None);
        assert!(markup.contains("status"));
        assert!(markup.contains("table"));
        assert!(!markup.contains("tbody"));
    }
}
