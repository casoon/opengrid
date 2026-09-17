//! The `<opengrid-grid>` model: query building, result parsing, the virtualized
//! window/pool arithmetic, the keyboard navigation and the `<table role="grid">`
//! as pure patch data.
//!
//! Grid mode is the interactive sibling of table mode
//! (plan/spezifikation/09-accessibility.md §Zwei Rendering-Modi): a native
//! `<table role="grid">` whose cells are focusable and whose header cells toggle
//! the sort. Everything in this module is portable data — the query JSON is
//! built from the host attributes, the engine's result JSON is parsed into the
//! [`GridState`] of point 15, a key is mapped onto the next focused cell, the
//! virtual window is derived from a scroll offset, and the markup is computed as
//! [`Patch`]es. The same functions run on the host in unit tests and in the
//! browser through the renderer (plan/spezifikation/11-crates.md §Portabilität).
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
//! # Virtualization (point 17)
//!
//! At 100 000 logical rows the grid keeps a **fixed pool** of DOM rows and
//! recycles them while scrolling; no new nodes appear per scroll step
//! (plan/spezifikation/08-rendering.md §Change Detection, 13-risiken.md R2).
//!
//! * The **row pool** is [`DEFAULT_POOL_SIZE`] rows by default (the host
//!   `window-size` overrides it), each row a `<tr>` with one `<td>` per column.
//!   The pool is built once; scrolling only patches text, `data-row`,
//!   `aria-rowindex` and the row's `translateY`.
//! * **Row height** is the CSS custom property `--grid-row-height` (default
//!   [`DEFAULT_ROW_HEIGHT`]px). The element resolves it once per host and passes
//!   the pixel value into the portable window math, so a logical row always
//!   occupies exactly that many pixels and the math stays stable.
//! * **Window math.** A logical row `r` sits at `r * row_height` inside the
//!   `<tbody>`, which acts as the sizer (`height = total_count * row_height`)
//!   and is `position: relative`; the rows are `position: absolute` and moved by
//!   `transform: translateY(...)`. The first visible row is therefore
//!   `scrollTop / row_height` ([`visible_start`]) and the fetched window starts
//!   [`OVERSCAN`] rows above it ([`window_offset`]), clamped so the last window
//!   ends at the last row. The `<thead>` is `position: sticky`.
//! * **Range fetching.** The provider is asked for exactly the window
//!   (`limit` = pool, `offset` = window start), never the whole result.
//! * **Focus pinning.** [`assign_pool`] keeps the slot that already holds the
//!   focused logical row, so scrolling never rewrites (and thus never blurs) the
//!   focused cell; the other slots are filled with the new window rows. A pinned
//!   row outside the window is left untouched and stays rendered (its content may
//!   be stale, which is invisible while it is scrolled out of view).
//!
//! # Counting rules (09)
//!
//! `aria-rowcount` is `total_count + 1` because the header row counts;
//! `aria-colcount` is the number of columns. Every row carries `aria-rowindex`,
//! 1-based including the header: the header row is 1, the first data row is 2,
//! so logical row `r` is `r + 2`.
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
use opengrid_grid::{CellRef, GridState, Window};
use opengrid_query::{CmpOp, FilterExpr};
use opengrid_types::{DataType, Field, FieldName, Schema, Value};
use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

/// The custom element name (E1).
pub const GRID_TAG: &str = "opengrid-grid";

/// The host attribute naming the source in the query's `source` field.
pub const DATASOURCE_ATTRIBUTE: &str = "datasource";

/// The host attribute listing the selected fields, comma-separated.
pub const COLUMNS_ATTRIBUTE: &str = "columns";

/// The host attribute for the recycled row pool (the query's `limit`).
///
/// Point 16 paged with this attribute; point 17 reinterprets it as the
/// **window/pool size** — how many rows are rendered and fetched at once while
/// scrolling. Review renamed the attribute from its old paging name to
/// `window-size` to match that meaning.
pub const WINDOW_SIZE_ATTRIBUTE: &str = "window-size";

/// The pool size used when the `window-size` attribute is absent or invalid.
pub const DEFAULT_POOL_SIZE: u64 = 40;

/// The default pixel height of one logical row (the virtualization contract).
///
/// The host can override it with the CSS custom property
/// [`ROW_HEIGHT_PROPERTY`]; the element resolves the computed value once per
/// host and feeds it into the portable window math.
pub const DEFAULT_ROW_HEIGHT: u64 = 32;

/// The CSS custom property a host can set to override the row height.
///
/// A shadow-root stylesheet seeds it with [`DEFAULT_ROW_HEIGHT`]; a document
/// rule on the host (or an inline style) overrides it and the value inherits
/// into the shadow tree.
pub const ROW_HEIGHT_PROPERTY: &str = "--grid-row-height";

/// Rows kept above the first visible row so scrolling stays smooth.
pub const OVERSCAN: u64 = 6;

/// Fallback viewport height in rows for `PageUp`/`PageDown` when the browser
/// cannot report a laid-out height.
pub const DEFAULT_VIEWPORT_ROWS: u64 = 12;

/// The fixed pixel height of the filter row (`part="filter"`).
///
/// The row is a single horizontal line of controls, so its height is constant
/// regardless of the column count; the viewport below it takes the rest of the
/// host. A host can override it with [`FILTER_HEIGHT_PROPERTY`], but the default
/// keeps the virtualization math and the fixtures deterministic.
pub const FILTER_HEIGHT: u64 = 40;

/// The CSS custom property overriding [`FILTER_HEIGHT`].
pub const FILTER_HEIGHT_PROPERTY: &str = "--grid-filter-height";

/// The operators the type-agnostic filter row offers, in display order.
///
/// The wire names are exactly the query's (plan/spezifikation/02-query-modell.md
/// §Operatoren V1); `is_null`/`is_not_null` are not offered because the
/// type-agnostic input always has a value to compare.
pub const FILTER_OPERATORS: &[&str] = &[
    "contains",
    "starts_with",
    "eq",
    "ne",
    "gt",
    "gte",
    "lt",
    "lte",
];

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[
    LABEL_ATTRIBUTE,
    DATASOURCE_ATTRIBUTE,
    COLUMNS_ATTRIBUTE,
    WINDOW_SIZE_ATTRIBUTE,
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
    /// A `<td>` of a data row.
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
    /// One viewport up.
    PageUp,
    /// One viewport down.
    PageDown,
}

/// The nodes [`build_grid`] creates and [`patch_grid`] updates.
///
/// The whole grid tree is built once; the ids stay valid because the element
/// keeps the [`Dom`](opengrid_web_core::renderer::Dom) that created them. Every
/// later frame patches these nodes instead of rebuilding the table, which is what
/// makes row recycling (and focus survival) possible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridNodes {
    /// The type-agnostic filter row above the table (`part="filter"`).
    pub filter: FilterNodes,
    /// The scrollable viewport (`overflow-y: auto`) inside the shadow root.
    pub viewport: NodeId,
    /// The table's `<tbody>`, used as the sizer (`height = total * row_height`).
    pub tbody: NodeId,
    /// The `<table role="grid">`.
    pub table: NodeId,
    /// The header cells, one per column.
    pub header_cells: Vec<GridHeaderNodes>,
    /// The recycled pool: one entry per slot.
    pub rows: Vec<GridRowNodes>,
}

/// The nodes of one header cell: the `<th>` and its visible multi-sort index.
///
/// The column name lives in a static child `<span>`; the index `<span>` is
/// `aria-hidden`, so showing it never changes the header's accessible name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridHeaderNodes {
    /// The `<th scope="col">`.
    pub cell: NodeId,
    /// The `<span>` carrying the multi-sort order (empty for a single sort).
    pub index: NodeId,
}

/// The nodes of the filter row (plan point 18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterNodes {
    /// The row container (`part="filter"`).
    pub container: NodeId,
    /// The `role="status"` result-count line.
    pub status: NodeId,
    /// The "Clear" button (`part="filter-clear"`).
    pub clear: NodeId,
    /// One operator `select` + value `input` per column.
    pub columns: Vec<FilterColumnNodes>,
}

/// The controls of one column's filter group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterColumnNodes {
    /// The operator `<select>`.
    pub select: NodeId,
    /// The value `<input type="text">`.
    pub input: NodeId,
}

/// The nodes of one recycled pool row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRowNodes {
    /// The `<tr>`.
    pub row: NodeId,
    /// The `<td>`s, one per column.
    pub cells: Vec<NodeId>,
}

impl GridNodes {
    /// The pool size (number of recycled row slots).
    pub fn pool(&self) -> usize {
        self.rows.len()
    }
}

/// Splits the `columns` attribute into field names (same rule as table mode).
pub fn parse_columns(raw: Option<&str>) -> Vec<String> {
    crate::table::parse_columns(raw)
}

/// The pool size from the `window-size` attribute, or [`DEFAULT_POOL_SIZE`].
///
/// A missing, non-numeric or zero value falls back to the default, so a typo
/// never turns into a zero-row pool.
pub fn parse_window_size(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_POOL_SIZE)
}

/// The row height from a computed CSS value, or [`DEFAULT_ROW_HEIGHT`].
///
/// Accepts a `<number>px` value (e.g. `48px`); anything else — absent, a
/// different unit, or zero — falls back to the default, so the window math
/// always has a positive, well-defined height.
pub fn parse_row_height(raw: &str) -> u64 {
    raw.trim()
        .strip_suffix("px")
        .and_then(|number| number.trim().parse::<u64>().ok())
        .filter(|height| *height > 0)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
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

/// One row of the filter UI: a column, an operator and the typed value.
///
/// Values are always plain strings — the display schema is all `Utf8` in Phase B
/// and the result JSON carries no types (point 23). The literal is therefore sent
/// as a JSON string; comparing it works for text columns and is a documented
/// limitation for the others until point 23.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterEntry {
    /// The output column the comparison names.
    pub column: String,
    /// The comparison operator.
    pub op: CmpOp,
    /// The raw value text; an empty (or whitespace-only) value means "no filter".
    pub value: String,
}

/// The `filter` expression of the filter row: the `and` of every non-empty entry.
///
/// Entries with a blank value are skipped; an all-blank/empty input clears the
/// filter (`None`). A column that is not a valid field identifier is skipped
/// rather than turning into a malformed query.
pub fn filter_expr(entries: &[FilterEntry]) -> Option<FilterExpr> {
    let comparisons: Vec<FilterExpr> = entries
        .iter()
        .filter(|entry| !entry.value.trim().is_empty())
        .filter_map(|entry| {
            FieldName::new(entry.column.as_str())
                .ok()
                .map(|field| FilterExpr::Cmp {
                    field,
                    op: entry.op,
                    value: serde_json::Value::String(entry.value.clone()),
                })
        })
        .collect();
    if comparisons.is_empty() {
        None
    } else {
        Some(FilterExpr::And(comparisons))
    }
}

/// The visible result-count line (`role="status"`).
///
/// German wording, matching the plan point 18 ("N Treffer"); couples to point 41,
/// which formalises the status area.
pub fn count_text(total_count: u64) -> String {
    format!("{total_count} Treffer")
}

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
    use serde_json::{Value as Json, json};
    let mut query = serde_json::Map::new();
    query.insert("source".to_owned(), Json::String(source.to_owned()));
    query.insert(
        "select".to_owned(),
        Json::Array(columns.iter().cloned().map(Json::String).collect()),
    );
    if let Some(filter) = filter {
        let filter = serde_json::to_value(filter).expect("a filter expression serializes");
        query.insert("filter".to_owned(), filter);
    }
    if !sorts.is_empty() {
        let sorts: Vec<Json> = sorts
            .iter()
            .map(|(field, direction)| json!({ "field": field, "direction": direction }))
            .collect();
        query.insert("sort".to_owned(), Json::Array(sorts));
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

/// The first logical row visible at `scroll_top`.
///
/// With rows at `r * row_height` and a sticky header, the row whose top is at or
/// below the scroll offset is `scroll_top / row_height`.
pub const fn visible_start(scroll_top: u64, row_height: u64) -> u64 {
    scroll_top / row_height
}

/// The window start for a visible row, keeping [`OVERSCAN`] rows above it.
///
/// Clamped to `[0, total_count - pool]` so the last window ends at the last row
/// and never runs past the result.
pub fn window_offset(visible_start: u64, total_count: u64, pool: u64) -> u64 {
    window_offset_for_row(visible_start, total_count, pool)
}

/// The window start that contains `row`, keeping [`OVERSCAN`] rows above it and
/// clamped so the window stays inside `[0, total_count)`.
///
/// `lead` shrinks with a small pool so the row is always inside the window even
/// when the pool is shorter than the overscan.
pub fn window_offset_for_row(row: u64, total_count: u64, pool: u64) -> u64 {
    let max_offset = total_count.saturating_sub(pool);
    let lead = OVERSCAN.min(pool.saturating_sub(1));
    row.saturating_sub(lead).min(max_offset)
}

/// The logical rows of a window, hidded by `total_count`.
pub fn window_rows(offset: u64, total_count: u64, pool: u64) -> Vec<u64> {
    let end = offset.saturating_add(pool).min(total_count);
    (offset..end).collect()
}

/// Assigns logical rows to pool slots, **pinning the focused row**.
///
/// Rules (point 17):
///
/// * A slot whose row is still inside `rows` keeps it, so scrolling reuses the
///   same node for the same logical row where possible.
/// * The slot that already holds `focus` always keeps it — even when the row
///   left the window — so the focused DOM node is never reassigned and focus
///   survives scrolling.
/// * Remaining rows fill the free slots in order. If the pinned row leaves no
///   room for the whole window, one window row is dropped from the edge farthest
///   from the focus (still invisible thanks to the overscan).
pub fn assign_pool(
    old: &[Option<u64>],
    focus: Option<u64>,
    rows: &[u64],
    pool: usize,
) -> Vec<Option<u64>> {
    use std::collections::HashSet;

    let row_set: HashSet<u64> = rows.iter().copied().collect();
    let mut new = vec![None; pool];
    for (slot, current) in old.iter().enumerate().take(pool) {
        if let Some(row) = current
            && (row_set.contains(row) || Some(*row) == focus)
        {
            new[slot] = Some(*row);
        }
    }

    let pinned = new.iter().filter(|slot| slot.is_some()).count();
    let capacity = pool.saturating_sub(pinned);
    let mut to_place: Vec<u64> = rows
        .iter()
        .copied()
        .filter(|row| !new.contains(&Some(*row)))
        .collect();
    if to_place.len() > capacity {
        let drop = to_place.len() - capacity;
        // The focus is below the window when it scrolls down past it: drop the
        // rows at the top (smallest), otherwise drop the bottom (largest).
        let focus_below = focus
            .map(|focus| rows.last().is_none_or(|last| focus > *last))
            .unwrap_or(false);
        if focus_below {
            to_place.drain(0..drop);
        } else {
            let keep = to_place.len() - drop;
            to_place.truncate(keep);
        }
    }

    let mut free: Vec<usize> = (0..pool).filter(|slot| new[*slot].is_none()).collect();
    free.reverse();
    for row in to_place {
        if let Some(slot) = free.pop() {
            new[slot] = Some(row);
        }
    }
    new
}

/// The active cell after pressing `key`, before any reload.
///
/// Movement is clamped as plan/spezifikation/09-accessibility.md §Tastatur im
/// Grid Mode asks: arrows stay inside the columns and the whole result, `Home`/
/// `End` stay on the row, the page keys move by `viewport_rows` and
/// `Ctrl+Home`/`Ctrl+End` jump to the first/last cell of the whole result. The
/// caller derives from the answer whether the window has to be reloaded and, if
/// so, to which offset.
pub fn move_active(
    active: ActiveCell,
    key: GridKey,
    ncols: usize,
    total_count: u64,
    viewport_rows: u64,
) -> ActiveCell {
    if ncols == 0 {
        return active;
    }
    let last_col = ncols - 1;
    let last_row = total_count.saturating_sub(1);
    let header = |col: usize| ActiveCell::Header {
        col: col.min(last_col),
    };
    let data =
        |row: u64, col: usize| ActiveCell::Data(CellRef::new(row.min(last_row), col.min(last_col)));

    match key {
        GridKey::ArrowUp => match active {
            ActiveCell::Header { .. } => active,
            ActiveCell::Data(cell) if cell.row == 0 => ActiveCell::Header { col: cell.col },
            ActiveCell::Data(cell) => data(cell.row - 1, cell.col),
        },
        GridKey::ArrowDown => match active {
            ActiveCell::Header { col } => {
                if total_count == 0 {
                    active
                } else {
                    data(0, col)
                }
            }
            ActiveCell::Data(cell) => data(cell.row + 1, cell.col),
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
                data(last_row, last_col)
            }
        }
        GridKey::PageUp => match active {
            ActiveCell::Header { .. } => active,
            ActiveCell::Data(cell) => data(cell.row.saturating_sub(viewport_rows), cell.col),
        },
        GridKey::PageDown => match active {
            ActiveCell::Header { col } => {
                if total_count == 0 {
                    active
                } else {
                    data(viewport_rows.saturating_sub(1), col)
                }
            }
            ActiveCell::Data(cell) => data(cell.row.saturating_add(viewport_rows), cell.col),
        },
    }
}

/// The window offset a key asks for, given the cell it moved to.
///
/// A move that lands inside the current window needs no reload (`None`).
/// `Ctrl+Home`/`Ctrl+End` name the first/last window directly; any other move to
/// a data cell outside the window asks for the window that contains it.
pub fn requested_window(
    key: GridKey,
    next: ActiveCell,
    window: Window,
    total_count: u64,
    pool: u64,
) -> Option<u64> {
    let wanted = match key {
        GridKey::CtrlHome => 0,
        GridKey::CtrlEnd if total_count > 0 => {
            window_offset_for_row(total_count - 1, total_count, pool)
        }
        _ => match next {
            ActiveCell::Data(cell) if !window.contains(cell.row) => {
                window_offset_for_row(cell.row, total_count, pool)
            }
            _ => window.offset,
        },
    };
    (wanted != window.offset).then_some(wanted)
}

/// Builds the empty grid skeleton (point 17): the scrollable viewport, the
/// `<table role="grid">` with its sticky header and a fixed pool of empty data
/// rows, all hidden until the first result arrives.
///
/// This runs exactly once per data-attribute configuration. [`patch_grid`] then
/// recycles the returned nodes for every frame.
pub fn build_grid(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    label: Option<&str>,
    schema: &Schema,
    pool: usize,
) -> GridNodes {
    let fields = schema.fields();
    let ncols = fields.len();

    // One shadow-root stylesheet: the `--grid-row-height` default plus the
    // positioning rules the virtualized rows need. The property is declared on
    // `:host` with the default and can be overridden from the document (or an
    // inline style) on the host; the inner elements inherit the resolved value.
    let styles = format!(
        ":host {{ {ROW_HEIGHT_PROPERTY}: {DEFAULT_ROW_HEIGHT}px; {FILTER_HEIGHT_PROPERTY}: {FILTER_HEIGHT}px; }}
         [part=\"layout\"] {{ display: flex; flex-direction: column; height: 100%; min-height: 0; }}
         [part=\"filter\"] {{ display: flex; align-items: center; gap: 0.5rem; box-sizing: border-box;
                             flex: 0 0 auto;
                             height: var({FILTER_HEIGHT_PROPERTY}); padding: 0 0.5rem;
                             overflow-x: auto; overflow-y: hidden; white-space: nowrap; }}
         [part=\"filter\"] select, [part=\"filter\"] input, [part=\"filter\"] button {{ font: inherit; }}
         [part=\"filter-status\"] {{ margin-left: auto; }}
         [part=\"viewport\"] {{ flex: 1 1 0; min-height: 0; overflow-y: auto; position: relative; display: block; }}
         [part=\"sort-index\"] {{ margin-left: 0.25rem; font-size: 0.75em; }}
         table {{ width: 100%; table-layout: fixed; border-collapse: collapse; }}
         thead {{ position: sticky; top: 0; z-index: 2; background: Canvas; color: CanvasText; }}
         tbody tr {{ position: absolute; left: 0; width: 100%; display: table; table-layout: fixed; }}
         th, td {{ height: var({ROW_HEIGHT_PROPERTY}); box-sizing: border-box; padding: 0 8px;
                   white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
         th {{ background: Canvas; text-align: left; }}"
    );
    let style = element(buffer, nodes, Some(NodeId::ROOT), "style");
    buffer.push(Patch::SetText {
        node: style,
        text: styles,
    });

    let layout = element(buffer, nodes, Some(NodeId::ROOT), "div");
    buffer.push(Patch::SetAttribute {
        node: layout,
        name: "part".to_owned(),
        value: "layout".to_owned(),
    });

    let filter = build_filter(buffer, nodes, layout, fields);

    let viewport = element(buffer, nodes, Some(layout), "div");
    buffer.push(Patch::SetAttribute {
        node: viewport,
        name: "part".to_owned(),
        value: "viewport".to_owned(),
    });

    let table = element(buffer, nodes, Some(viewport), "table");
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
        value: "1".to_owned(),
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
    let mut header_cells = Vec::with_capacity(ncols);
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
            value: "none".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "tabindex".to_owned(),
            value: "-1".to_owned(),
        });
        // The column name lives in its own span so the multi-sort index can be
        // added as a second, `aria-hidden` span without touching the name.
        let name = element(buffer, nodes, Some(th), "span");
        buffer.push(Patch::SetText {
            node: name,
            text: field.name.as_str().to_owned(),
        });
        let index = element(buffer, nodes, Some(th), "span");
        buffer.push(Patch::SetAttribute {
            node: index,
            name: "part".to_owned(),
            value: "sort-index".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: index,
            name: "aria-hidden".to_owned(),
            value: "true".to_owned(),
        });
        header_cells.push(GridHeaderNodes { cell: th, index });
    }

    let tbody = element(buffer, nodes, Some(table), "tbody");
    set_style(buffer, tbody, "position: relative; height: 0px;");

    let mut rows = Vec::with_capacity(pool);
    for _ in 0..pool {
        let tr = element(buffer, nodes, Some(tbody), "tr");
        set_style(buffer, tr, ROW_HIDDEN_STYLE);
        let mut cells = Vec::with_capacity(ncols);
        for col in 0..ncols {
            let td = element(buffer, nodes, Some(tr), "td");
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "data-col".to_owned(),
                value: col.to_string(),
            });
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "tabindex".to_owned(),
                value: "-1".to_owned(),
            });
            cells.push(td);
        }
        rows.push(GridRowNodes { row: tr, cells });
    }

    GridNodes {
        filter,
        viewport,
        tbody,
        table,
        header_cells,
        rows,
    }
}

/// Builds the type-agnostic filter row: one operator `select` and one value
/// `input` per column, plus a clear button and the result count (point 18).
///
/// It sits **outside** the `role="grid"` table (`part="filter"`), so the grid's
/// roving tabindex and keyboard matrix are untouched; the controls are ordinary
/// focusable form elements. Each control is labelled with its column name via
/// `aria-label` (all references stay inside the shadow root, E8/R6).
fn build_filter(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    fields: &[Field],
) -> FilterNodes {
    let container = element(buffer, nodes, Some(parent), "div");
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "part".to_owned(),
        value: "filter".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "data-filter".to_owned(),
        value: String::new(),
    });
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "role".to_owned(),
        value: "group".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "aria-label".to_owned(),
        value: "Filter".to_owned(),
    });

    let mut columns = Vec::with_capacity(fields.len());
    for (col, field) in fields.iter().enumerate() {
        let group = element(buffer, nodes, Some(container), "span");
        set_style(
            buffer,
            group,
            "display: inline-flex; align-items: center; gap: 0.25rem;",
        );

        let select = element(buffer, nodes, Some(group), "select");
        buffer.push(Patch::SetAttribute {
            node: select,
            name: "part".to_owned(),
            value: "filter-operator".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: select,
            name: "data-col".to_owned(),
            value: col.to_string(),
        });
        buffer.push(Patch::SetAttribute {
            node: select,
            name: "aria-label".to_owned(),
            value: format!("{} operator", field.name),
        });
        for (option_index, op) in FILTER_OPERATORS.iter().enumerate() {
            let option = element(buffer, nodes, Some(select), "option");
            buffer.push(Patch::SetAttribute {
                node: option,
                name: "value".to_owned(),
                value: (*op).to_owned(),
            });
            if option_index == 0 {
                buffer.push(Patch::SetAttribute {
                    node: option,
                    name: "selected".to_owned(),
                    value: String::new(),
                });
            }
            buffer.push(Patch::SetText {
                node: option,
                text: (*op).to_owned(),
            });
        }

        let input = element(buffer, nodes, Some(group), "input");
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "type".to_owned(),
            value: "text".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "part".to_owned(),
            value: "filter-value".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "data-col".to_owned(),
            value: col.to_string(),
        });
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "aria-label".to_owned(),
            value: format!("{} value", field.name),
        });
        set_style(buffer, input, "width: 6rem;");
        columns.push(FilterColumnNodes { select, input });
    }

    let clear = element(buffer, nodes, Some(container), "button");
    buffer.push(Patch::SetAttribute {
        node: clear,
        name: "type".to_owned(),
        value: "button".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: clear,
        name: "part".to_owned(),
        value: "filter-clear".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: clear,
        name: "data-filter-clear".to_owned(),
        value: String::new(),
    });
    buffer.push(Patch::SetText {
        node: clear,
        text: "Clear".to_owned(),
    });

    let status = element(buffer, nodes, Some(container), "span");
    buffer.push(Patch::SetAttribute {
        node: status,
        name: "part".to_owned(),
        value: "filter-status".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: status,
        name: "role".to_owned(),
        value: "status".to_owned(),
    });

    FilterNodes {
        container,
        status,
        clear,
        columns,
    }
}

/// Updates the skeleton in one frame: sizes the sizer, refreshes the header and
/// the result count and recycles the pool rows for the current window.
///
/// `slots` is the slot → logical row assignment (computed by [`assign_pool`]).
/// `pinned_slot` is the slot holding the focused cell: it is left completely
/// untouched, so the focused DOM node keeps its `data-row` and the browser keeps
/// the focus. Every other slot gets its new `aria-rowindex`, `data-row`, text
/// and `translateY` — no node is created or removed.
///
/// `sorts` is the whole sort list in order (point 18): a column's `aria-sort`
/// reflects its key and, when more than one key is active, its 1-based position
/// is shown in the header's `aria-hidden` index span.
#[allow(clippy::too_many_arguments)]
pub fn patch_grid(
    buffer: &mut PatchBuffer,
    nodes: &GridNodes,
    state: &GridState,
    slots: &[Option<u64>],
    active: ActiveCell,
    sorts: &[(String, &str)],
    pinned_slot: Option<usize>,
    row_height: u64,
) {
    let fields = state.schema().fields();
    let total_count = state.total_count();

    buffer.push(Patch::SetAttribute {
        node: nodes.tbody,
        name: "style".to_owned(),
        value: format!(
            "position: relative; height: {}px;",
            total_count * row_height
        ),
    });
    buffer.push(Patch::SetAttribute {
        node: nodes.table,
        name: "aria-rowcount".to_owned(),
        value: (total_count + 1).to_string(),
    });
    buffer.push(Patch::SetText {
        node: nodes.filter.status,
        text: count_text(total_count),
    });

    for (col, header) in nodes.header_cells.iter().enumerate() {
        let key = fields
            .get(col)
            .and_then(|field| sort_position(sorts, field.name.as_str()));
        buffer.push(Patch::SetAttribute {
            node: header.cell,
            name: "aria-sort".to_owned(),
            value: key
                .map_or("none", |position| aria_sort(sorts[position].1))
                .to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: header.cell,
            name: "tabindex".to_owned(),
            value: tabindex_for(active == ActiveCell::Header { col }).to_owned(),
        });
        let index = if sorts.len() > 1 {
            key.map(|position| (position + 1).to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        buffer.push(Patch::SetText {
            node: header.index,
            text: index,
        });
    }

    for (slot, row_nodes) in nodes.rows.iter().enumerate() {
        if Some(slot) == pinned_slot {
            continue;
        }
        match slots.get(slot).copied().flatten() {
            Some(row) => {
                set_style(buffer, row_nodes.row, &row_style(row, row_height));
                buffer.push(Patch::SetAttribute {
                    node: row_nodes.row,
                    name: "aria-rowindex".to_owned(),
                    value: (row + 2).to_string(),
                });
                for (col, cell) in row_nodes.cells.iter().enumerate() {
                    buffer.push(Patch::SetAttribute {
                        node: *cell,
                        name: "data-row".to_owned(),
                        value: row.to_string(),
                    });
                    let is_active = active == ActiveCell::Data(CellRef::new(row, col));
                    buffer.push(Patch::SetAttribute {
                        node: *cell,
                        name: "tabindex".to_owned(),
                        value: tabindex_for(is_active).to_owned(),
                    });
                    let text = state
                        .cell(CellRef::new(row, col))
                        .map(cell_text)
                        .unwrap_or_default();
                    buffer.push(Patch::SetText { node: *cell, text });
                }
            }
            None => {
                set_style(buffer, row_nodes.row, ROW_HIDDEN_STYLE);
                buffer.push(Patch::RemoveAttribute {
                    node: row_nodes.row,
                    name: "aria-rowindex".to_owned(),
                });
            }
        }
    }
}

/// The inline style placing a visible pool row at its logical position.
///
/// The static `position: absolute` box comes from the shadow stylesheet; only
/// the logical offset is per-row.
fn row_style(row: u64, row_height: u64) -> String {
    format!("transform: translateY({}px);", row * row_height)
}

/// Hides a pool slot that holds no logical row in the current window.
const ROW_HIDDEN_STYLE: &str = "display: none;";

/// Sets an element's `style` attribute.
fn set_style(buffer: &mut PatchBuffer, node: NodeId, style: &str) {
    buffer.push(Patch::SetAttribute {
        node,
        name: "style".to_owned(),
        value: style.to_owned(),
    });
}

/// The position of a column in the sort list, if it is a key.
fn sort_position(sorts: &[(String, &str)], name: &str) -> Option<usize> {
    sorts.iter().position(|(field, _)| field == name)
}

/// The `aria-sort` token for a wire direction.
fn aria_sort(direction: &str) -> &'static str {
    match direction {
        "asc" => "ascending",
        "desc" => "descending",
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

    fn state_with(rows: &[&str], total: u64, offset: u64, pool: u64) -> GridState {
        let schema = Schema::new(vec![
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("qty").unwrap(), DataType::Utf8),
        ]);
        let mut state = GridState::new(schema);
        state.set_window(Window::new(offset, pool));
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

    /// The pool size falls back to the default on a missing, broken or zero value.
    #[test]
    fn pool_size_falls_back_to_the_default() {
        assert_eq!(parse_window_size(None), 40);
        assert_eq!(parse_window_size(Some(" 25 ")), 25);
        assert_eq!(parse_window_size(Some("0")), 40);
        assert_eq!(parse_window_size(Some("nope")), 40);
    }

    /// The row height accepts `<number>px` and falls back on anything else.
    #[test]
    fn row_height_parses_pixels_and_falls_back() {
        assert_eq!(parse_row_height("48px"), 48);
        assert_eq!(parse_row_height(" 20px "), 20);
        assert_eq!(parse_row_height("48"), 32);
        assert_eq!(parse_row_height("48em"), 32);
        assert_eq!(parse_row_height("0px"), 32);
        assert_eq!(parse_row_height(""), 32);
    }

    /// The query carries select, limit and offset, and omits sort when unsorted.
    #[test]
    fn an_unsorted_query_has_limit_and_offset() {
        use serde_json::{Value as Json, json};
        let query = query_json("orders", &["a".to_owned()], &[], None, 100, 40);
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(value["source"], "orders");
        assert_eq!(value["select"], json!(["a"]));
        assert_eq!(value["limit"], json!(40));
        assert_eq!(value["offset"], json!(100));
        assert!(value.get("sort").is_none());
        assert!(value.get("filter").is_none());
    }

    /// A sorted query carries one sort object per key, in order.
    #[test]
    fn a_sorted_query_names_one_direction() {
        use serde_json::{Value as Json, json};
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[("customer".to_owned(), "desc")],
            None,
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([{ "field": "customer", "direction": "desc" }])
        );
    }

    /// A multi-sort query keeps the keys in the user's order.
    #[test]
    fn a_multi_sort_query_keeps_the_key_order() {
        use serde_json::{Value as Json, json};
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[
                ("customer".to_owned(), "asc"),
                ("amount".to_owned(), "desc"),
            ],
            None,
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([
                { "field": "customer", "direction": "asc" },
                { "field": "amount", "direction": "desc" }
            ])
        );
    }

    /// The filter expression is the `and` of the non-empty entries.
    #[test]
    fn the_filter_query_is_the_and_of_the_entries() {
        use serde_json::{Value as Json, json};
        let entries = [
            FilterEntry {
                column: "customer".to_owned(),
                op: CmpOp::Contains,
                value: "Al".to_owned(),
            },
            FilterEntry {
                column: "qty".to_owned(),
                op: CmpOp::Gt,
                value: "2".to_owned(),
            },
        ];
        let filter = filter_expr(&entries).expect("a filter");
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[("customer".to_owned(), "asc")],
            Some(&filter),
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["filter"],
            json!({ "and": [
                { "field": "customer", "op": "contains", "value": "Al" },
                { "field": "qty", "op": "gt", "value": "2" }
            ]})
        );
    }

    /// Blank entries are skipped; an all-blank input clears the filter.
    #[test]
    fn blank_filter_entries_do_not_build_a_filter() {
        assert_eq!(filter_expr(&[]), None);
        let blank = [FilterEntry {
            column: "customer".to_owned(),
            op: CmpOp::Eq,
            value: "  ".to_owned(),
        }];
        assert_eq!(filter_expr(&blank), None);
        let one = [FilterEntry {
            column: "customer".to_owned(),
            op: CmpOp::Eq,
            value: "DE".to_owned(),
        }];
        assert!(filter_expr(&one).is_some());
    }

    /// The count line uses the German wording of the point.
    #[test]
    fn count_text_formats_the_result_line() {
        assert_eq!(count_text(0), "0 Treffer");
        assert_eq!(count_text(1_234), "1234 Treffer");
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

    /// The skeleton builds role, counts, header and exactly `pool` hidden rows.
    #[test]
    fn the_skeleton_counts_rows_and_columns() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, Some("Bestellungen"), &schema, 3);

        assert_eq!(view.pool(), 3);
        assert_eq!(view.header_cells.len(), 2);
        assert_eq!(view.rows[0].cells.len(), 2);
        assert_eq!(view.filter.columns.len(), 2);

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

        // The filter group, the status line and the grid itself carry roles.
        assert_eq!(attributes("role"), ["group", "status", "grid"]);
        // The filter row is labelled separately; the grid label comes last.
        let labels = attributes("aria-label");
        assert_eq!(labels.first().map(String::as_str), Some("Filter"));
        assert_eq!(labels.last().map(String::as_str), Some("Bestellungen"));
        assert_eq!(attributes("aria-colcount"), ["2"]);
        // Only the header row carries an index in the skeleton; the pool rows
        // get theirs from `patch_grid`.
        assert_eq!(attributes("aria-rowindex"), ["1"]);
        let parts = attributes("part");
        assert_eq!(&parts[..2], ["layout", "filter"]);
        assert!(parts.iter().any(|part| part == "viewport"));
        assert_eq!(
            parts
                .iter()
                .filter(|part| part.as_str() == "filter-operator")
                .count(),
            2
        );
        assert_eq!(
            parts
                .iter()
                .filter(|part| part.as_str() == "sort-index")
                .count(),
            2
        );
    }

    /// Patching the pool recycles the existing rows: counts, rowindexes and
    /// exactly one `tabindex=0`.
    #[test]
    fn patching_recycles_the_pool() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, Some("Bestellungen"), &schema, 4);
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 4);
        let slots = assign_pool(&[None; 4], None, &window_rows(0, 5, 4), 4);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[],
            None,
            DEFAULT_ROW_HEIGHT,
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

        assert_eq!(attributes("aria-rowcount"), ["6"]);
        // The header row (1) is set in the skeleton; this frame patches the four
        // data rows 0..4 → 2..5.
        assert_eq!(attributes("aria-rowindex"), ["2", "3", "4", "5"]);
        // Exactly one tabindex 0: the active header cell.
        let tabindexes = attributes("tabindex");
        assert_eq!(
            tabindexes
                .iter()
                .filter(|value| value.as_str() == "0")
                .count(),
            1
        );
    }

    /// The sorted column is the only one with a non-`none` `aria-sort`.
    #[test]
    fn only_the_sorted_header_carries_aria_sort() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, None, &schema, 1);
        let state = state_with(&["Gamma"], 1, 0, 1);
        let slots = assign_pool(&[None], None, &window_rows(0, 1, 1), 1);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Data(CellRef::new(0, 1)),
            &[("qty".to_owned(), "asc")],
            None,
            DEFAULT_ROW_HEIGHT,
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

    /// Multi-sort marks each key with its direction and shows its order index.
    #[test]
    fn a_multi_sort_shows_each_key_with_its_order() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, None, &schema, 1);
        let state = state_with(&["Gamma"], 1, 0, 1);
        let slots = assign_pool(&[None], None, &window_rows(0, 1, 1), 1);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[("customer".to_owned(), "asc"), ("qty".to_owned(), "desc")],
            None,
            DEFAULT_ROW_HEIGHT,
        );

        let aria: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { name, value, .. } if name == "aria-sort" => {
                    Some(value.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(aria, ["ascending", "descending"]);

        let text_of = |node: NodeId| -> String {
            buffer
                .patches()
                .iter()
                .find_map(|patch| match patch {
                    Patch::SetText {
                        node: current,
                        text,
                    } if *current == node => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        assert_eq!(text_of(view.header_cells[0].index), "1");
        assert_eq!(text_of(view.header_cells[1].index), "2");
        assert_eq!(text_of(view.filter.status), "1 Treffer");
    }

    /// A single sort leaves the header's order index empty.
    #[test]
    fn a_single_sort_hides_the_order_index() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, None, &schema, 1);
        let state = state_with(&["Gamma"], 1, 0, 1);
        let slots = assign_pool(&[None], None, &window_rows(0, 1, 1), 1);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[("customer".to_owned(), "asc")],
            None,
            DEFAULT_ROW_HEIGHT,
        );
        let indexes: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetText { node, text } if *node == view.header_cells[0].index => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(indexes, [""]);
    }

    /// The sizer height is `total_count * row_height`.
    #[test]
    fn the_sizer_reflects_the_total_count() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, None, &schema, 2);
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 2);
        let slots = assign_pool(&[None, None], None, &window_rows(0, 5, 2), 2);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[],
            None,
            DEFAULT_ROW_HEIGHT,
        );
        assert!(buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::SetAttribute { node, name, value }
                if *node == view.tbody && name == "style" && value == "position: relative; height: 160px;"
        )));
    }

    /// A non-default row height drives both the sizer and the row offsets.
    #[test]
    fn a_custom_row_height_rescales_the_sizer_and_the_rows() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, None, &schema, 3);
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 3);
        let slots = assign_pool(&[None, None, None], None, &window_rows(0, 5, 3), 3);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[],
            None,
            48,
        );

        // The sizer is `total * 48`, not `total * 32`.
        assert!(buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::SetAttribute { node, name, value }
                if *node == view.tbody && name == "style" && value == "position: relative; height: 240px;"
        )));
        // Row 2 sits at `2 * 48`.
        assert!(buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::SetAttribute { node, name, value }
                if *node == view.rows[2].row && name == "style" && value == "transform: translateY(96px);"
        )));
    }

    /// The pinned slot is not patched, so the focused node keeps its `data-row`.
    #[test]
    fn the_pinned_slot_is_left_untouched() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(&mut buffer, &mut nodes, None, &schema, 4);
        // Slot 3 holds the focused row 6; the window scrolled to rows 20..24.
        let old = [Some(20), Some(21), Some(22), Some(6)];
        let focus = Some(6);
        let rows = window_rows(20, 100, 4);
        let slots = assign_pool(&old, focus, &rows, 4);
        let pinned = old.iter().position(|slot| *slot == focus);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state_with(&[], 100, 20, 4),
            &slots,
            ActiveCell::Data(CellRef::new(6, 0)),
            &[],
            pinned,
            DEFAULT_ROW_HEIGHT,
        );

        let pinned_row = view.rows[3].row;
        assert!(
            !buffer.patches().iter().any(|patch| matches!(
                patch,
                Patch::SetAttribute { node, .. } if *node == pinned_row
            )),
            "the pinned row node must not be patched"
        );
        assert!(slots.contains(&focus));
    }

    /// Arrow keys move by one and clamp to the whole result, not the window.
    #[test]
    fn arrows_move_and_clamp() {
        let active = ActiveCell::Data(CellRef::new(0, 0));
        let down = move_active(active, GridKey::ArrowDown, 2, 5, 3);
        assert_eq!(down, ActiveCell::Data(CellRef::new(1, 0)));
        // At the last row the move is clamped.
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(4, 0)),
                GridKey::ArrowDown,
                2,
                5,
                3
            ),
            ActiveCell::Data(CellRef::new(4, 0))
        );
        // Up from the first row enters the header.
        assert_eq!(
            move_active(active, GridKey::ArrowUp, 2, 5, 3),
            ActiveCell::Header { col: 0 }
        );
        // Down from the header enters the first row.
        assert_eq!(
            move_active(ActiveCell::Header { col: 0 }, GridKey::ArrowDown, 2, 5, 3),
            ActiveCell::Data(CellRef::new(0, 0))
        );
        // Right/left clamp at the last/first column.
        assert_eq!(
            move_active(active, GridKey::ArrowRight, 2, 5, 3),
            ActiveCell::Data(CellRef::new(0, 1))
        );
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(0, 1)),
                GridKey::ArrowRight,
                2,
                5,
                3
            ),
            ActiveCell::Data(CellRef::new(0, 1))
        );
        assert_eq!(
            move_active(active, GridKey::ArrowLeft, 2, 5, 3),
            ActiveCell::Data(CellRef::new(0, 0))
        );
    }

    /// Home/End move within the row; Ctrl+Home/End jump to the grid's corners.
    #[test]
    fn home_end_and_ctrl_jump() {
        let active = ActiveCell::Data(CellRef::new(1, 1));
        assert_eq!(
            move_active(active, GridKey::Home, 2, 5, 3),
            ActiveCell::Data(CellRef::new(1, 0))
        );
        assert_eq!(
            move_active(active, GridKey::End, 2, 5, 3),
            ActiveCell::Data(CellRef::new(1, 1))
        );
        assert_eq!(
            move_active(active, GridKey::CtrlHome, 2, 5, 3),
            ActiveCell::Header { col: 0 }
        );
        assert_eq!(
            move_active(active, GridKey::CtrlEnd, 2, 5, 3),
            ActiveCell::Data(CellRef::new(4, 1))
        );
    }

    /// Page keys move by a viewport and clamp to the whole result.
    #[test]
    fn page_keys_move_by_a_viewport() {
        let active = ActiveCell::Data(CellRef::new(0, 0));
        let next = move_active(active, GridKey::PageDown, 2, 100, 10);
        assert_eq!(next, ActiveCell::Data(CellRef::new(10, 0)));
        let up = move_active(
            ActiveCell::Data(CellRef::new(30, 0)),
            GridKey::PageUp,
            2,
            100,
            10,
        );
        assert_eq!(up, ActiveCell::Data(CellRef::new(20, 0)));
        // At the last row the move clamps.
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(99, 0)),
                GridKey::PageDown,
                2,
                100,
                10
            ),
            ActiveCell::Data(CellRef::new(99, 0))
        );
    }

    /// A move inside the window asks for no reload; a move outside does.
    #[test]
    fn a_move_outside_the_window_requests_it() {
        let window = Window::new(0, 10);
        assert_eq!(
            requested_window(
                GridKey::ArrowDown,
                ActiveCell::Data(CellRef::new(5, 0)),
                window,
                100,
                10
            ),
            None
        );
        assert_eq!(
            requested_window(
                GridKey::ArrowDown,
                ActiveCell::Data(CellRef::new(10, 0)),
                window,
                100,
                10
            ),
            Some(4)
        );
        assert_eq!(
            requested_window(
                GridKey::CtrlEnd,
                ActiveCell::Data(CellRef::new(99, 1)),
                window,
                100,
                10
            ),
            Some(90)
        );
        assert_eq!(
            requested_window(
                GridKey::CtrlHome,
                ActiveCell::Header { col: 0 },
                Window::new(40, 10),
                100,
                10
            ),
            Some(0)
        );
    }

    /// The visible row is derived from the scroll offset and the row height.
    #[test]
    fn scroll_offset_maps_to_the_visible_row() {
        assert_eq!(visible_start(0, DEFAULT_ROW_HEIGHT), 0);
        assert_eq!(visible_start(31, DEFAULT_ROW_HEIGHT), 0);
        assert_eq!(visible_start(32, DEFAULT_ROW_HEIGHT), 1);
        assert_eq!(visible_start(1_000_000, DEFAULT_ROW_HEIGHT), 31_250);
        // A configured row height scales the same math.
        assert_eq!(visible_start(95, 48), 1);
        assert_eq!(visible_start(96, 48), 2);
        assert_eq!(visible_start(480, 48), 10);
    }

    /// The window keeps the overscan above the visible row and clamps at the end.
    #[test]
    fn window_offset_clamps_to_the_result() {
        assert_eq!(window_offset(0, 100, 10), 0);
        assert_eq!(window_offset(50, 100, 10), 44);
        assert_eq!(window_offset(99, 100, 10), 90);
        assert_eq!(window_offset_for_row(0, 100, 10), 0);
        assert_eq!(window_offset_for_row(99, 100, 10), 90);
        assert_eq!(window_rows(90, 100, 10), (90..100).collect::<Vec<_>>());
        // A pool shorter than the overscan still contains its row.
        assert_eq!(window_offset_for_row(2, 5, 2), 1);
    }

    /// A fresh assignment fills the free slots in order.
    #[test]
    fn assignment_fills_free_slots() {
        let slots = assign_pool(&[None; 3], None, &[0, 1, 2], 3);
        assert_eq!(slots, [Some(0), Some(1), Some(2)]);
    }

    /// Existing assignments are kept when the row is still in the window.
    #[test]
    fn assignment_keeps_existing_rows() {
        let old = [Some(0), Some(1), Some(2)];
        let slots = assign_pool(&old, None, &[0, 1, 2, 3], 3);
        assert_eq!(slots, [Some(0), Some(1), Some(2)]);
    }

    /// The focused row is pinned even when it left the window.
    #[test]
    fn assignment_pins_the_focused_row() {
        let old = [Some(6), Some(20), Some(21), Some(22)];
        let slots = assign_pool(&old, Some(6), &[20, 21, 22, 23], 4);
        assert_eq!(slots[0], Some(6));
        assert!(!slots.contains(&Some(23)));
    }

    /// When nothing is pinned the window fills the pool, dropping the far edge.
    #[test]
    fn assignment_scrolls_without_focus() {
        let old = [Some(0), Some(1), Some(2), Some(3)];
        let slots = assign_pool(&old, None, &[10, 11, 12, 13], 4);
        assert_eq!(slots, [Some(10), Some(11), Some(12), Some(13)]);
    }
}
