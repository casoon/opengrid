//! The `<opengrid-grid>` model: query building, result parsing, the keyboard
//! navigation and the `<table role="grid">` as pure patch data.
//!
//! Grid mode is the interactive sibling of table mode
//! (plan/spezifikation/09-accessibility.md §Zwei Rendering-Modi): a native
//! `<table role="grid">` whose cells are focusable and whose header cells toggle
//! the sort. Everything in this module is portable data — the query JSON is
//! built from the host attributes, the engine's result JSON is parsed into the
//! [`GridState`] of point 15, a key is mapped onto the next focused cell, and
//! the markup is computed as [`Patch`]es. The same functions run on the host in
//! unit tests and in the browser through the renderer
//! (plan/spezifikation/11-crates.md §Portabilität).
//!
//! # Display schema (Phase B)
//!
//! The provider result JSON carries only column names, not types: the typed
//! schema wire form is point 23. The grid therefore builds a **display schema**
//! whose fields are all [`DataType::Utf8`] from the result's column names, purely
//! for view bookkeeping (column order, count, header text). This is deliberate
//! and temporary — point 23 replaces it with the real types, at which point the
//! values can be formatted per type. Until then every value travels as its
//! display text (the wire notation of E13, decimals already strings).
//!
//! # Counting rules (09)
//!
//! `aria-rowcount` is `total_count + 1` because the header row counts;
//! `aria-colcount` is the number of columns. Every row carries `aria-rowindex`,
//! 1-based including the header: the header row is 1, the first data row is 2,
//! so logical row `r` is `r + 2`. Point 17 fixes and tests the same rule for the
//! virtualized window; point 16 uses it for the loaded page.
//!
//! # Focus
//!
//! Exactly one cell carries `tabindex="0"`, every other cell `tabindex="-1"`
//! (roving tabindex). The active cell is an [`ActiveCell`] — a header cell or a
//! logical data cell. The element tracks it and mirrors a data cell into
//! [`GridState::set_focus`], so the state machine stays the owner of the logical
//! data focus; a header cell is element-level because the logical data model has
//! no header row. The DOM glue focuses the matching node and scrolls it into
//! view.

use opengrid_datasource::QueryResult;
use opengrid_grid::{CellRef, GridState};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};
use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

/// The custom element name (E1).
pub const GRID_TAG: &str = "opengrid-grid";

/// The host attribute naming the source in the query's `source` field.
pub const DATASOURCE_ATTRIBUTE: &str = "datasource";

/// The host attribute listing the selected fields, comma-separated.
pub const COLUMNS_ATTRIBUTE: &str = "columns";

/// The host attribute for the page size (the query's `limit`); default 100.
pub const PAGE_SIZE_ATTRIBUTE: &str = "page-size";

/// The page size used when the `page-size` attribute is absent or invalid.
pub const DEFAULT_PAGE_SIZE: u64 = 100;

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[
    LABEL_ATTRIBUTE,
    DATASOURCE_ATTRIBUTE,
    COLUMNS_ATTRIBUTE,
    PAGE_SIZE_ATTRIBUTE,
];

/// Which cell owns the roving tabindex.
///
/// A header cell is addressed by its column alone; the logical data model has no
/// header row, so it cannot be a [`CellRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveCell {
    /// A `<th>` of the header row.
    Header {
        /// Column index into the schema.
        col: usize,
    },
    /// A `<td>` of a loaded data row.
    Data(CellRef),
}

impl ActiveCell {
    /// The column of this cell.
    pub const fn col(self) -> usize {
        match self {
            Self::Header { col } => col,
            Self::Data(cell) => cell.col,
        }
    }

    /// The logical data cell, or `None` for a header cell.
    pub const fn data(self) -> Option<CellRef> {
        match self {
            Self::Header { .. } => None,
            Self::Data(cell) => Some(cell),
        }
    }
}

/// A key the grid handles (decoded from a `keydown` in the element).
///
/// The matrix is the one from plan/spezifikation/09-accessibility.md §Tastatur im
/// Grid Mode. `Tab`/`Shift+Tab` are deliberately absent: the roving tabindex lets
/// the browser move out of the grid on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKey {
    /// One row up.
    ArrowUp,
    /// One row down.
    ArrowDown,
    /// One column left.
    ArrowLeft,
    /// One column right.
    ArrowRight,
    /// First column of the current row.
    Home,
    /// Last column of the current row.
    End,
    /// First cell of the grid.
    CtrlHome,
    /// Last cell of the grid.
    CtrlEnd,
    /// One page up.
    PageUp,
    /// One page down.
    PageDown,
}

/// The nodes [`build_grid`] creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridNodes {
    /// The `<table role="grid">`.
    pub table: NodeId,
}

/// Splits the `columns` attribute into field names (same rule as table mode).
pub fn parse_columns(raw: Option<&str>) -> Vec<String> {
    crate::table::parse_columns(raw)
}

/// The page size from the `page-size` attribute, or [`DEFAULT_PAGE_SIZE`].
///
/// A missing, non-numeric or zero value falls back to the default, so a typo
/// never turns into a zero-row query.
pub fn parse_page_size(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_PAGE_SIZE)
}

/// The display schema for the initial render, before the first result arrives.
///
/// All fields are [`DataType::Utf8`] — the display schema decision documented on
/// the module. Names that are not valid identifiers are skipped; the result
/// schema replaces this one on the first [`GridState::apply_result`].
pub fn initial_schema(columns: &[String]) -> Schema {
    let fields = columns
        .iter()
        .filter_map(|name| {
            FieldName::new(name.as_str())
                .ok()
                .map(|name| Field::new(name, DataType::Utf8))
        })
        .collect();
    Schema::new(fields)
}

/// Builds the query JSON for a grid render.
///
/// The shape is `{ "source", "select", "sort", "limit", "offset" }`
/// (plan/spezifikation/02-query-modell.md §JSON-Vertrag); `sort` is omitted when
/// nothing is sorted and carried as one `{ "field", "direction" }` object
/// otherwise. `direction` is the wire token `"asc"`/`"desc"`.
pub fn query_json(
    source: &str,
    columns: &[String],
    sort: Option<(&str, &str)>,
    offset: u64,
    limit: u64,
) -> String {
    use serde_json::{Value as Json, json};
    let mut query = serde_json::Map::new();
    query.insert("source".to_owned(), Json::String(source.to_owned()));
    query.insert(
        "select".to_owned(),
        Json::Array(columns.iter().cloned().map(Json::String).collect()),
    );
    if let Some((field, direction)) = sort {
        query.insert(
            "sort".to_owned(),
            json!([{ "field": field, "direction": direction }]),
        );
    }
    query.insert("limit".to_owned(), json!(limit));
    query.insert("offset".to_owned(), json!(offset));
    Json::Object(query).to_string()
}

/// Parses the engine's result JSON into a [`QueryResult`] with the display
/// schema.
///
/// The wire shape is `{ "total_count", "row_count", "columns": [ { "name",
/// "values" } ] }`. Values become their display text as [`Value::Utf8`] — JSON
/// `null` is the empty string — because the wire form carries no types (point
/// 23). A malformed result is an error, not a panic.
pub fn parse_result(result_json: &str) -> Result<QueryResult, String> {
    use serde_json::Value as Json;
    let value: Json =
        serde_json::from_str(result_json).map_err(|error| format!("result JSON: {error}"))?;
    let total_count = value
        .get("total_count")
        .and_then(Json::as_u64)
        .ok_or_else(|| "result has no total_count".to_owned())?;
    let columns = value
        .get("columns")
        .and_then(Json::as_array)
        .ok_or_else(|| "result has no columns".to_owned())?;

    let mut fields = Vec::with_capacity(columns.len());
    let mut data = Vec::with_capacity(columns.len());
    for column in columns {
        let name = column
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| "a column has no name".to_owned())?;
        let field_name =
            FieldName::new(name).map_err(|_| format!("invalid column name {name:?}"))?;
        let values = column
            .get("values")
            .and_then(Json::as_array)
            .ok_or_else(|| format!("column {name:?} has no values"))?;
        fields.push(Field::new(field_name, DataType::Utf8));
        data.push(
            values
                .iter()
                .map(|value| Value::Utf8(value_text(value)))
                .collect(),
        );
    }
    Ok(QueryResult::new(Schema::new(fields), data, total_count))
}

/// The display text of one wire value (JSON `null` is the empty string).
fn value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The display text of a typed cell value.
fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        Value::Int64(number) => number.to_string(),
        Value::Float64(number) => number.to_string(),
        Value::Decimal(decimal) => decimal.to_string(),
        Value::Utf8(text) => text.clone(),
        Value::Date(date) => date.to_string(),
        Value::Timestamp(timestamp) => timestamp.to_string(),
    }
}

/// The first logical row of the page that contains `row`.
pub const fn page_offset_for(row: u64, page_size: u64) -> u64 {
    (row / page_size) * page_size
}

/// The active cell after pressing `key`, before any reload.
///
/// Movement is clamped exactly as plan/spezifikation/09-accessibility.md §Tastatur
/// im Grid Mode asks: arrows stay inside the rendered rows/columns, `Home`/`End`
/// stay on the row, the page keys move by `page_size` rows and `Ctrl+Home`/`End`
/// jump to the first/last cell of the whole result. The caller derives from the
/// answer whether the page has to be reloaded and, if so, to which offset.
pub fn move_active(
    active: ActiveCell,
    key: GridKey,
    ncols: usize,
    total_count: u64,
    offset: u64,
    loaded_rows: usize,
    page_size: u64,
) -> ActiveCell {
    if ncols == 0 {
        return active;
    }
    let last_col = ncols - 1;
    let header = |col: usize| ActiveCell::Header {
        col: col.min(last_col),
    };
    let data = |row: u64, col: usize| ActiveCell::Data(CellRef::new(row, col.min(last_col)));

    match key {
        GridKey::ArrowUp => match active {
            ActiveCell::Header { .. } => active,
            ActiveCell::Data(cell) if cell.row > offset => data(cell.row - 1, cell.col),
            ActiveCell::Data(cell) => ActiveCell::Header { col: cell.col },
        },
        GridKey::ArrowDown => match active {
            ActiveCell::Header { col } => {
                if loaded_rows == 0 {
                    active
                } else {
                    data(offset, col)
                }
            }
            ActiveCell::Data(cell) => {
                let last_loaded = offset + loaded_rows as u64 - 1;
                if cell.row < last_loaded {
                    data(cell.row + 1, cell.col)
                } else {
                    active
                }
            }
        },
        GridKey::ArrowLeft => match active {
            ActiveCell::Header { col } => header(col.saturating_sub(1)),
            ActiveCell::Data(cell) => data(cell.row, cell.col.saturating_sub(1)),
        },
        GridKey::ArrowRight => match active {
            ActiveCell::Header { col } => header(col + 1),
            ActiveCell::Data(cell) => data(cell.row, cell.col + 1),
        },
        GridKey::Home => match active {
            ActiveCell::Header { .. } => header(0),
            ActiveCell::Data(cell) => data(cell.row, 0),
        },
        GridKey::End => match active {
            ActiveCell::Header { .. } => header(last_col),
            ActiveCell::Data(cell) => data(cell.row, last_col),
        },
        GridKey::CtrlHome => header(0),
        GridKey::CtrlEnd => {
            if total_count == 0 {
                header(last_col)
            } else {
                data(total_count - 1, last_col)
            }
        }
        GridKey::PageUp => match active {
            ActiveCell::Header { .. } => active,
            ActiveCell::Data(cell) => data(cell.row.saturating_sub(page_size), cell.col),
        },
        GridKey::PageDown => match active {
            ActiveCell::Header { col } => {
                if total_count == 0 {
                    active
                } else {
                    data((offset + page_size).min(total_count - 1), col)
                }
            }
            ActiveCell::Data(cell) => data((cell.row + page_size).min(total_count - 1), cell.col),
        },
    }
}

/// The page offset a key asks for, given the cell it moved to.
///
/// `Ctrl+Home`/`Ctrl+End` name the first/last page directly; any move that lands
/// on a data cell asks for the page containing it. A header move leaves the page
/// alone. Returns `None` when the current offset already shows the target, so the
/// caller can move focus without a reload.
pub fn requested_offset(
    key: GridKey,
    next: ActiveCell,
    offset: u64,
    total_count: u64,
    page_size: u64,
) -> Option<u64> {
    let wanted = match key {
        GridKey::CtrlHome => 0,
        GridKey::CtrlEnd if total_count > 0 => page_offset_for(total_count - 1, page_size),
        _ => match next {
            ActiveCell::Data(cell) => page_offset_for(cell.row, page_size),
            ActiveCell::Header { .. } => offset,
        },
    };
    (wanted != offset).then_some(wanted)
}

/// Appends the grid to `buffer` and answers its nodes.
///
/// Renders the loaded page only: a header row (row index 1) with one
/// `<th scope="col">` per column and one `<tr>` per loaded logical row, each
/// carrying its 1-based `aria-rowindex` (`r + 2`). Exactly one cell carries
/// `tabindex="0"` — `active` — and every other cell `tabindex="-1"`.
pub fn build_grid(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    label: Option<&str>,
    state: &GridState,
    active: ActiveCell,
    sort: Option<(&str, &str)>,
) -> GridNodes {
    let fields = state.schema().fields();
    let ncols = fields.len();
    let total_count = state.total_count();
    let offset = state.window().offset;
    let loaded_rows = state.loaded_rows();

    let table = element(buffer, nodes, Some(NodeId::ROOT), "table");
    buffer.push(Patch::SetAttribute {
        node: table,
        name: "role".to_owned(),
        value: "grid".to_owned(),
    });
    if let Some((name, value)) = mirror_label(label) {
        buffer.push(Patch::SetAttribute {
            node: table,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    buffer.push(Patch::SetAttribute {
        node: table,
        name: "aria-rowcount".to_owned(),
        value: (total_count + 1).to_string(),
    });
    buffer.push(Patch::SetAttribute {
        node: table,
        name: "aria-colcount".to_owned(),
        value: ncols.to_string(),
    });

    let thead = element(buffer, nodes, Some(table), "thead");
    let header_row = element(buffer, nodes, Some(thead), "tr");
    buffer.push(Patch::SetAttribute {
        node: header_row,
        name: "aria-rowindex".to_owned(),
        value: "1".to_owned(),
    });
    for (col, field) in fields.iter().enumerate() {
        let th = element(buffer, nodes, Some(header_row), "th");
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "scope".to_owned(),
            value: "col".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "data-col".to_owned(),
            value: col.to_string(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "aria-sort".to_owned(),
            value: aria_sort_for(sort, field.name.as_str()).to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "tabindex".to_owned(),
            value: tabindex_for(active == ActiveCell::Header { col }).to_owned(),
        });
        buffer.push(Patch::SetText {
            node: th,
            text: field.name.as_str().to_owned(),
        });
    }

    let tbody = element(buffer, nodes, Some(table), "tbody");
    for slot in 0..loaded_rows {
        let row = offset + slot as u64;
        let tr = element(buffer, nodes, Some(tbody), "tr");
        buffer.push(Patch::SetAttribute {
            node: tr,
            name: "aria-rowindex".to_owned(),
            value: (row + 2).to_string(),
        });
        for col in 0..ncols {
            let td = element(buffer, nodes, Some(tr), "td");
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "data-row".to_owned(),
                value: row.to_string(),
            });
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "data-col".to_owned(),
                value: col.to_string(),
            });
            let is_active = active == ActiveCell::Data(CellRef::new(row, col));
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "tabindex".to_owned(),
                value: tabindex_for(is_active).to_owned(),
            });
            let text = state
                .cell(CellRef::new(row, col))
                .map(cell_text)
                .unwrap_or_default();
            buffer.push(Patch::SetText { node: td, text });
        }
    }

    GridNodes { table }
}

/// The `aria-sort` token of a column for the active single sort.
fn aria_sort_for(sort: Option<(&str, &str)>, name: &str) -> &'static str {
    match sort {
        Some((field, "asc")) if field == name => "ascending",
        Some((field, "desc")) if field == name => "descending",
        _ => "none",
    }
}

/// The `tabindex` value of a cell: `0` for the active one, `-1` otherwise.
fn tabindex_for(is_active: bool) -> &'static str {
    if is_active { "0" } else { "-1" }
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

/// Appends a text-only `<p role="alert">` error to `buffer` (as table mode).
pub fn build_error(buffer: &mut PatchBuffer, nodes: &mut NodeAllocator, message: &str) {
    let paragraph = element(buffer, nodes, Some(NodeId::ROOT), "p");
    buffer.push(Patch::SetAttribute {
        node: paragraph,
        name: "role".to_owned(),
        value: "alert".to_owned(),
    });
    buffer.push(Patch::SetText {
        node: paragraph,
        text: message.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_grid::Window;

    fn state_with(rows: &[&str], total: u64, offset: u64, page_size: u64) -> GridState {
        let schema = Schema::new(vec![
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("qty").unwrap(), DataType::Utf8),
        ]);
        let mut state = GridState::new(schema);
        state.set_window(Window::new(offset, page_size));
        let customers = rows
            .iter()
            .map(|row| Value::Utf8((*row).to_owned()))
            .collect();
        let qtys = (0..rows.len())
            .map(|index| Value::Utf8(index.to_string()))
            .collect();
        state.apply_result(QueryResult::new(
            Schema::new(vec![
                Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
                Field::new(FieldName::new("qty").unwrap(), DataType::Utf8),
            ]),
            vec![customers, qtys],
            total,
        ));
        state
    }

    /// The columns attribute is the table's trimmed list.
    #[test]
    fn columns_attribute_is_a_trimmed_list() {
        assert_eq!(
            parse_columns(Some(" a , b ,,")),
            ["a".to_owned(), "b".to_owned()]
        );
    }

    /// The page size falls back to 100 on a missing, broken or zero value.
    #[test]
    fn page_size_falls_back_to_the_default() {
        assert_eq!(parse_page_size(None), 100);
        assert_eq!(parse_page_size(Some(" 25 ")), 25);
        assert_eq!(parse_page_size(Some("0")), 100);
        assert_eq!(parse_page_size(Some("nope")), 100);
    }

    /// The query carries select, limit and offset, and omits sort when unsorted.
    #[test]
    fn an_unsorted_query_has_limit_and_offset() {
        use serde_json::{Value as Json, json};
        let query = query_json("orders", &["a".to_owned()], None, 100, 100);
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(value["source"], "orders");
        assert_eq!(value["select"], json!(["a"]));
        assert_eq!(value["limit"], json!(100));
        assert_eq!(value["offset"], json!(100));
        assert!(value.get("sort").is_none());
    }

    /// A sorted query carries exactly one sort object.
    #[test]
    fn a_sorted_query_names_one_direction() {
        use serde_json::{Value as Json, json};
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            Some(("customer", "desc")),
            0,
            100,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([{ "field": "customer", "direction": "desc" }])
        );
    }

    /// The result becomes an all-`Utf8` display schema and text values.
    #[test]
    fn result_json_parses_into_a_display_schema() {
        let result = r#"{
            "total_count": 7,
            "row_count": 2,
            "columns": [
                { "name": "customer", "values": ["Alpha", null] },
                { "name": "amount", "values": ["10.00", "20.00"] }
            ]
        }"#;
        let parsed = parse_result(result).expect("parses");
        assert_eq!(parsed.total_count, 7);
        assert_eq!(parsed.row_count(), 2);
        assert_eq!(parsed.schema.len(), 2);
        assert!(
            parsed
                .schema
                .fields()
                .iter()
                .all(|field| field.data_type == DataType::Utf8)
        );
        assert_eq!(
            parsed.columns[0],
            [Value::Utf8("Alpha".to_owned()), Value::Utf8(String::new())]
        );
    }

    /// A malformed result is an error, not a panic.
    #[test]
    fn a_result_without_columns_is_an_error() {
        assert!(parse_result("{}").is_err());
        assert!(parse_result("not json").is_err());
    }

    /// The table carries role, counts, row indexes and exactly one `tabindex=0`.
    #[test]
    fn the_rendered_grid_counts_rows_and_columns() {
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 2);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            Some("Bestellungen"),
            &state,
            ActiveCell::Header { col: 0 },
            None,
        );

        let attributes = |name: &str| -> Vec<String> {
            buffer
                .patches()
                .iter()
                .filter_map(|patch| match patch {
                    Patch::SetAttribute {
                        name: attr, value, ..
                    } if attr == name => Some(value.clone()),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(attributes("role"), ["grid"]);
        assert_eq!(attributes("aria-label"), ["Bestellungen"]);
        assert_eq!(attributes("aria-rowcount"), ["6"]);
        assert_eq!(attributes("aria-colcount"), ["2"]);
        // Header row 1, then the two loaded logical rows 0 and 1 → 2 and 3.
        assert_eq!(attributes("aria-rowindex"), ["1", "2", "3"]);
        // Exactly one cell: the active header cell.
        assert_eq!(attributes("tabindex"), ["0", "-1", "-1", "-1", "-1", "-1"]);
    }

    /// The sorted column is the only one with a non-`none` `aria-sort`.
    #[test]
    fn only_the_sorted_header_carries_aria_sort() {
        let state = state_with(&["Gamma"], 1, 0, 1);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            None,
            &state,
            ActiveCell::Data(CellRef::new(0, 1)),
            Some(("qty", "asc")),
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
        assert_eq!(sorts, ["none", "ascending"]);
    }

    /// Arrow keys move by one and clamp to the loaded rows and the columns.
    #[test]
    fn arrows_move_and_clamp() {
        let active = ActiveCell::Data(CellRef::new(0, 0));
        let down = move_active(active, GridKey::ArrowDown, 2, 5, 0, 2, 2);
        assert_eq!(down, ActiveCell::Data(CellRef::new(1, 0)));
        // At the last loaded row the move is clamped.
        assert_eq!(
            move_active(down, GridKey::ArrowDown, 2, 5, 0, 2, 2),
            ActiveCell::Data(CellRef::new(1, 0))
        );
        // Up from the first loaded row enters the header.
        assert_eq!(
            move_active(active, GridKey::ArrowUp, 2, 5, 0, 2, 2),
            ActiveCell::Header { col: 0 }
        );
        // Down from the header enters the first loaded row.
        assert_eq!(
            move_active(
                ActiveCell::Header { col: 0 },
                GridKey::ArrowDown,
                2,
                5,
                0,
                2,
                2
            ),
            ActiveCell::Data(CellRef::new(0, 0))
        );
        // Right/left clamp at the last/first column.
        assert_eq!(
            move_active(active, GridKey::ArrowRight, 2, 5, 0, 2, 2),
            ActiveCell::Data(CellRef::new(0, 1))
        );
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(0, 1)),
                GridKey::ArrowRight,
                2,
                5,
                0,
                2,
                2
            ),
            ActiveCell::Data(CellRef::new(0, 1))
        );
        assert_eq!(
            move_active(active, GridKey::ArrowLeft, 2, 5, 0, 2, 2),
            ActiveCell::Data(CellRef::new(0, 0))
        );
    }

    /// Home/End move within the row; Ctrl+Home/End jump to the grid's corners.
    #[test]
    fn home_end_and_ctrl_jump() {
        let active = ActiveCell::Data(CellRef::new(1, 1));
        assert_eq!(
            move_active(active, GridKey::Home, 2, 5, 0, 2, 2),
            ActiveCell::Data(CellRef::new(1, 0))
        );
        assert_eq!(
            move_active(active, GridKey::End, 2, 5, 0, 2, 2),
            ActiveCell::Data(CellRef::new(1, 1))
        );
        assert_eq!(
            move_active(active, GridKey::CtrlHome, 2, 5, 0, 2, 2),
            ActiveCell::Header { col: 0 }
        );
        assert_eq!(
            move_active(active, GridKey::CtrlEnd, 2, 5, 0, 2, 2),
            ActiveCell::Data(CellRef::new(4, 1))
        );
    }

    /// Page keys move by a page; the requested offset is the page containing the
    /// target.
    #[test]
    fn page_keys_move_by_a_page() {
        let active = ActiveCell::Data(CellRef::new(0, 0));
        let next = move_active(active, GridKey::PageDown, 2, 5, 0, 2, 2);
        assert_eq!(next, ActiveCell::Data(CellRef::new(2, 0)));
        assert_eq!(requested_offset(GridKey::PageDown, next, 0, 5, 2), Some(2));

        let up = move_active(
            ActiveCell::Data(CellRef::new(3, 0)),
            GridKey::PageUp,
            2,
            5,
            2,
            2,
            2,
        );
        assert_eq!(up, ActiveCell::Data(CellRef::new(1, 0)));
        assert_eq!(requested_offset(GridKey::PageUp, up, 2, 5, 2), Some(0));

        // PageDown at the last page keeps the page but clamps the row.
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(4, 0)),
                GridKey::PageDown,
                2,
                5,
                4,
                1,
                2
            ),
            ActiveCell::Data(CellRef::new(4, 0))
        );
    }

    /// An arrow inside the loaded page asks for no reload.
    #[test]
    fn a_move_inside_the_page_needs_no_reload() {
        let next = ActiveCell::Data(CellRef::new(1, 0));
        assert_eq!(requested_offset(GridKey::ArrowDown, next, 0, 5, 2), None);
        // Ctrl+End asks for the last page.
        let end = ActiveCell::Data(CellRef::new(4, 1));
        assert_eq!(requested_offset(GridKey::CtrlEnd, end, 0, 5, 2), Some(4));
    }
}
