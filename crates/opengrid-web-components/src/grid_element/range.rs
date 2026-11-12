//! A block of cells, marked and copied (issue #327).
//!
//! `Shift`+arrows on a data cell — or `Shift`+click, or dragging — mark the
//! rectangle between where it began and where the focus is, announced as a
//! size ("3 by 4 cells") and drawn with `data-range` on its cells. `Ctrl`/`Cmd`
//! +`C` puts it on the clipboard as values, tab-separated
//! ([`opengrid_grid::range`]); the row selection (`Shift`+`Space`) is another
//! thing and stays as it was, so no cell says `aria-selected` for it.
//!
//! A range is positions, so it ends with any key that moves or edits, with a
//! click elsewhere and with another result for the whole query. It is off while
//! grouped and in a tree, where a position is not a place. A copy covers the
//! rows that are loaded; a range that reaches past them says so and copies
//! nothing.

use super::*;

use opengrid_grid::{CellRange, range};

/// Whether ranges are allowed at all: plain rows, as a paste is.
fn allowed(host: &HtmlElement) -> bool {
    !is_grouped(host) && !is_tree(host)
}

/// Forgets the range and takes its marks off.
fn clear(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    if runtime.borrow_mut().range.take().is_some() {
        paint(host);
    }
}

/// The focus moved by something other than the range's own keys and clicks — a
/// click elsewhere, a script, a `Tab` back in: the block ends. A focus on the
/// cell the block's edge is on is the block's own.
pub(super) fn on_focus(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, active: ActiveCell) {
    let edge = runtime.borrow().range.map(|range| range.focus);
    if edge.is_some() && active.data() != edge {
        clear(host, runtime);
    }
}

/// Draws the range on the cells in view: `data-range` on each, and `data-ranging`
/// on the table, which keeps the browser from selecting text while one is marked.
pub(super) fn paint(host: &HtmlElement) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let range = runtime.borrow().range;
    if let Ok(Some(table)) = root.query_selector("table") {
        if range.is_some() {
            let _ = table.set_attribute("data-ranging", "");
        } else {
            let _ = table.remove_attribute("data-ranging");
        }
    }
    let Ok(cells) = root.query_selector_all("td[data-row][data-col]") else {
        return;
    };
    for index in 0..cells.length() {
        let Some(td) = cells
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let inside =
            cell_of(&td).is_some_and(|cell| range.is_some_and(|range| range.contains(cell)));
        match (inside, td.has_attribute("data-range")) {
            (true, false) => {
                let _ = td.set_attribute("data-range", "");
            }
            (false, true) => {
                let _ = td.remove_attribute("data-range");
            }
            _ => {}
        }
    }
}

fn cell_of(td: &Element) -> Option<CellRef> {
    Some(CellRef::new(
        td.get_attribute("data-row")?.parse().ok()?,
        td.get_attribute("data-col")?.parse().ok()?,
    ))
}

/// Says the size of the range in the status line.
fn announce_size(host: &HtmlElement, range: &CellRange) {
    if range.is_single() {
        return;
    }
    let (rows, cols) = range.size();
    announce(host, &texts(host).range_size(rows, cols));
}

/// Takes the range keys: `Shift`+arrow extends, anything else that moves or
/// edits ends it. `true` when the key is spent.
pub(super) fn on_key(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    event: &KeyboardEvent,
    active: ActiveCell,
) -> bool {
    let key = event.key();
    // A modifier on its own, and the copy itself, leave the range alone.
    if matches!(
        key.as_str(),
        "Shift" | "Control" | "Meta" | "Alt" | "ContextMenu"
    ) || ((event.ctrl_key() || event.meta_key()) && key.eq_ignore_ascii_case("c"))
    {
        return false;
    }
    let (rows, cols) = match key.as_str() {
        "ArrowUp" => (-1i64, 0i64),
        "ArrowDown" => (1, 0),
        "ArrowLeft" => (0, -1),
        "ArrowRight" => (0, 1),
        _ => (0, 0),
    };
    let extends = event.shift_key()
        && !event.ctrl_key()
        && !event.meta_key()
        && !event.alt_key()
        && (rows, cols) != (0, 0);
    let ActiveCell::Data(cell) = active else {
        clear(host, runtime);
        return false;
    };
    if !extends || !allowed(host) {
        clear(host, runtime);
        return false;
    }
    event.prevent_default();
    let (anchor, from, row_count, col_count) = {
        let borrowed = runtime.borrow();
        let from = borrowed.range.map_or(cell, |range| range.focus);
        let anchor = borrowed.range.map_or(cell, |range| range.anchor);
        (
            anchor,
            from,
            borrowed.state.row_count(),
            borrowed.state.schema().len(),
        )
    };
    if row_count == 0 || col_count == 0 {
        return true;
    }
    let row = (from.row as i64 + rows).clamp(0, row_count as i64 - 1) as u64;
    let col = (from.col as i64 + cols).clamp(0, col_count as i64 - 1) as usize;
    let target = CellRef::new(row, col);
    if matches!(runtime.borrow().state.row_kind(row), RowKind::Draft) {
        return true;
    }
    let next = CellRange::new(anchor, target);
    runtime.borrow_mut().range = Some(next);
    history::focus_after(host, runtime, Some(target));
    // `focus_after` may have redrawn or asked for another window; the marks
    // follow either way.
    paint(host);
    announce_size(host, &next);
    true
}

/// The cell under a pointer event, if it is a data cell of the grid and not a
/// field in it.
fn data_cell(event: &Event) -> Option<(HtmlElement, CellRef, Element)> {
    let root = current_shadow_root(event)?;
    let host = root.host().dyn_into::<HtmlElement>().ok()?;
    let target = event.target()?.dyn_into::<Element>().ok()?;
    if target
        .closest("input, select, textarea, button, [part=\"editor\"]")
        .ok()
        .flatten()
        .is_some()
    {
        return None;
    }
    let td = target.closest("td[data-row][data-col]").ok().flatten()?;
    let cell = cell_of(&td)?;
    Some((host, cell, td))
}

pub(super) fn on_mouse_down(event: Event) {
    let Some((host, cell, _)) = data_cell(&event) else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() else {
        return;
    };
    if mouse.button() != 0 || !allowed(&host) {
        return;
    }
    if mouse.shift_key() {
        // From where the focus stands (or the range began) to the cell.
        let anchor = {
            let borrowed = runtime.borrow();
            borrowed
                .range
                .map(|range| range.anchor)
                .or_else(|| borrowed.active.data())
        };
        if let Some(anchor) = anchor {
            let next = CellRange::new(anchor, cell);
            runtime.borrow_mut().range = Some(next);
            runtime.borrow_mut().drag_from = None;
            paint(&host);
            announce_size(&host, &next);
        }
        return;
    }
    runtime.borrow_mut().drag_from = Some(cell);
    clear(&host, &runtime);
}

pub(super) fn on_mouse_over(event: Event) {
    let Some((host, cell, _)) = data_cell(&event) else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() else {
        return;
    };
    let from = runtime.borrow().drag_from;
    let Some(from) = from else {
        return;
    };
    // A button let go outside the grid never reached `mouseup`.
    if mouse.buttons() & 1 == 0 {
        runtime.borrow_mut().drag_from = None;
        return;
    }
    let next = CellRange::new(from, cell);
    if runtime.borrow().range == Some(next)
        || (next.is_single() && runtime.borrow().range.is_none())
    {
        return;
    }
    runtime.borrow_mut().range = Some(next);
    if let Some(selection) =
        web_sys::window().and_then(|window| window.get_selection().ok().flatten())
    {
        let _ = selection.remove_all_ranges();
    }
    paint(&host);
}

pub(super) fn on_mouse_up(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let dragged = runtime.borrow_mut().drag_from.take().is_some();
    let range = runtime.borrow().range;
    if dragged && let Some(range) = range {
        announce_size(&host, &range);
    }
}

/// `Ctrl`/`Cmd`+`C` with a range: the cells as tab-separated values, or — when
/// the range reaches rows that are not loaded — a sentence and nothing copied.
pub(super) fn on_copy(event: Event) {
    // Chromium aims the event at the cell, through the shadow root; WebKit and
    // Firefox aim it at the host. The first listener to act keeps the second.
    if event.default_prevented() {
        return;
    }
    let Some(current) = event.current_target() else {
        return;
    };
    let (root, host) = match current.dyn_into::<ShadowRoot>() {
        Ok(root) => {
            let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
                return;
            };
            (root, host)
        }
        Err(current) => {
            let Ok(host) = current.dyn_into::<HtmlElement>() else {
                return;
            };
            let Some(root) = host.shadow_root() else {
                return;
            };
            (root, host)
        }
    };
    // From a field — the filter row, the search, an open editor — the browser
    // copies as it always did. The focus says which, as the target of the
    // event is the host in some engines.
    if root.active_element().is_some_and(|active| {
        active
            .closest("input, select, textarea, [part=\"editor\"]")
            .ok()
            .flatten()
            .is_some()
    }) {
        return;
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let Some(block) = runtime.borrow().range.filter(|range| !range.is_single()) else {
        return;
    };
    let texts = texts(&host);
    let text = {
        let borrowed = runtime.borrow();
        let (first_row, last_row) = block.rows();
        let (first_col, last_col) = block.cols();
        let lone = first_col == last_col;
        let mut lines: Vec<Vec<String>> = Vec::new();
        for row in first_row..=last_row {
            let mut line = Vec::new();
            for col in first_col..=last_col {
                let Some(value) = borrowed.state.cell(CellRef::new(row, col)) else {
                    drop(borrowed);
                    runtime
                        .borrow_mut()
                        .state
                        .set_notice(texts.range_unloaded.clone());
                    render(&host, false);
                    event.prevent_default();
                    return;
                };
                let text_column = borrowed
                    .state
                    .schema()
                    .fields()
                    .get(col)
                    .is_some_and(|field| field.data_type == DataType::Utf8);
                line.push(range::cell_text(
                    &crate::formats::plain_text(value),
                    text_column,
                    lone,
                ));
            }
            lines.push(line);
        }
        range::block_text(&lines)
    };
    let Some(data) = event
        .dyn_ref::<web_sys::ClipboardEvent>()
        .and_then(|event| event.clipboard_data())
    else {
        return;
    };
    if data.set_data("text/plain", &text).is_err() {
        return;
    }
    event.prevent_default();
    let (rows, cols) = block.size();
    runtime
        .borrow_mut()
        .state
        .set_notice(texts.range_copied(rows, cols));
    render(&host, false);
}
