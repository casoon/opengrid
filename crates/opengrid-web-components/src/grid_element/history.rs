//! Undo and redo of what the reader did (issue #186).
//!
//! `Ctrl`/`Cmd`+`Z` walks the grid's own history back, `Shift`+`Ctrl`/`Cmd`+`Z`
//! and `Ctrl`+`Y` forward. Each step fires the events the original edit fired —
//! `opengrid-cell-change`, `opengrid-row-add`, `opengrid-paste` — with
//! `undo: true` (or `redo: true`), so the page saves a step as it saves an edit.
//! A paste is one step. The history is cleared when the source is replaced.

use super::*;

use opengrid_grid::{AddedEntry, CellEdit, Step};

pub(super) fn undo(host: &HtmlElement) {
    run(host, true);
}

pub(super) fn redo(host: &HtmlElement) {
    run(host, false);
}

fn run(host: &HtmlElement, undoing: bool) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let texts = texts(host);
    let step = {
        let mut borrowed = runtime.borrow_mut();
        let history = borrowed.state.history_mut();
        if undoing {
            history.take_undo()
        } else {
            history.take_redo()
        }
    };
    let Some(step) = step else {
        let nothing = if undoing {
            &texts.undo_nothing
        } else {
            &texts.redo_nothing
        };
        runtime.borrow_mut().state.set_notice(nothing.clone());
        render(host, false);
        return;
    };
    let flag = if undoing { "undo" } else { "redo" };
    let (step, said, at) = match step {
        Step::Cell(edit) => {
            let (cell, column) = apply_cell(host, &runtime, &edit, undoing, flag);
            let said = texts.history_cell(undoing, &column, cell.row + 1);
            (Step::Cell(edit), said, Some(cell))
        }
        Step::Added(entry) => {
            let entry = apply_added(host, &runtime, entry, undoing, flag);
            let said = texts.history_added(undoing);
            (Step::Added(entry), said, None)
        }
        Step::Paste { edits, added } => {
            let (added, at) = apply_paste(host, &runtime, &edits, added, undoing, flag);
            let said = texts.history_paste(undoing, edits.len());
            (Step::Paste { edits, added }, said, at)
        }
    };
    {
        let mut borrowed = runtime.borrow_mut();
        let history = borrowed.state.history_mut();
        if undoing {
            history.done_undo(step);
        } else {
            history.done_redo(step);
        }
        borrowed.state.set_notice(said);
    }
    focus_after(host, &runtime, at);
}

/// Sets one cell back or forward and tells the page. Answers where the cell is
/// now — the record by its key if it moved — and its column's name.
fn apply_cell(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    edit: &CellEdit,
    undoing: bool,
    flag: &str,
) -> (CellRef, String) {
    let (value, other) = if undoing {
        (&edit.before, &edit.after)
    } else {
        (&edit.after, &edit.before)
    };
    let (cell, column, key) = place(runtime, edit);
    runtime.borrow_mut().state.set_cell(cell, value.clone());
    dispatch_cell_change(
        host,
        cell,
        key.as_ref(),
        &column,
        &crate::formats::event_text(value),
        &crate::formats::event_text(other),
        Some(flag),
    );
    (cell, column)
}

/// Where an edit's cell is now, its column's name and its record's key as the
/// page gave it.
fn place(
    runtime: &Rc<RefCell<GridRuntime>>,
    edit: &CellEdit,
) -> (CellRef, String, Option<JsValue>) {
    let borrowed = runtime.borrow();
    let row = edit
        .key
        .as_ref()
        .and_then(|key| borrowed.state.row_of_key(key))
        .unwrap_or(edit.row);
    let column = borrowed
        .state
        .schema()
        .fields()
        .get(edit.col)
        .map(|field| field.name.as_str().to_owned())
        .unwrap_or_default();
    let key = edit
        .key
        .as_ref()
        .map(|key| to_js(&opengrid_json::ToJson::to_json(key)));
    (CellRef::new(row, edit.col), column, key)
}

fn by_name(runtime: &Rc<RefCell<GridRuntime>>, values: &[Value]) -> Vec<(String, Value)> {
    runtime
        .borrow()
        .state
        .schema()
        .fields()
        .iter()
        .zip(values)
        .map(|(field, value)| (field.name.as_str().to_owned(), value.clone()))
        .collect()
}

/// Takes an added row out, or puts it back, and tells the page.
fn apply_added(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    entry: AddedEntry,
    undoing: bool,
    flag: &str,
) -> AddedEntry {
    let entry = toggle_row(runtime, entry, undoing);
    dispatch_row_add(
        host,
        &entry.provisional,
        &by_name(runtime, &entry.values),
        Some(flag),
    );
    entry
}

/// The row of an add step leaves the added rows, or comes back under a new key.
fn toggle_row(runtime: &Rc<RefCell<GridRuntime>>, entry: AddedEntry, undoing: bool) -> AddedEntry {
    let mut borrowed = runtime.borrow_mut();
    if undoing {
        borrowed.state.added_mut().remove(&entry.provisional);
        return entry;
    }
    match borrowed.state.added_mut().push(entry.values.clone()) {
        Some(row) => AddedEntry {
            provisional: row.provisional.clone(),
            values: entry.values,
        },
        None => entry,
    }
}

/// A paste goes back or forward as it came: its cells and its rows, one event.
fn apply_paste(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    edits: &[CellEdit],
    added: Vec<AddedEntry>,
    undoing: bool,
    flag: &str,
) -> (Vec<AddedEntry>, Option<CellRef>) {
    let changes = js_sys::Array::new();
    let mut first = None;
    for edit in edits {
        let (value, other) = if undoing {
            (&edit.before, &edit.after)
        } else {
            (&edit.after, &edit.before)
        };
        let (cell, column, key) = place(runtime, edit);
        runtime.borrow_mut().state.set_cell(cell, value.clone());
        first.get_or_insert(cell);
        let item = js_sys::Object::new();
        if let Some(key) = &key {
            let _ = js_sys::Reflect::set(&item, &JsValue::from_str("key"), key);
        }
        for (name, value) in [
            ("row", JsValue::from_f64(cell.row as f64)),
            ("column", JsValue::from_str(&column)),
            (
                "value",
                JsValue::from_str(&crate::formats::event_text(value)),
            ),
            (
                "previous",
                JsValue::from_str(&crate::formats::event_text(other)),
            ),
        ] {
            let _ = js_sys::Reflect::set(&item, &JsValue::from_str(name), &value);
        }
        changes.push(&item);
    }
    let rows = js_sys::Array::new();
    let mut now = Vec::with_capacity(added.len());
    for entry in added {
        let entry = toggle_row(runtime, entry, undoing);
        let values = js_sys::Object::new();
        for (name, value) in by_name(runtime, &entry.values) {
            let _ = js_sys::Reflect::set(
                &values,
                &JsValue::from_str(&name),
                &to_js(&opengrid_json::ToJson::to_json(&value)),
            );
        }
        let item = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&item, &JsValue::from_str("values"), &values);
        let _ = js_sys::Reflect::set(
            &item,
            &JsValue::from_str("provisionalKey"),
            &JsValue::from_str(&entry.provisional),
        );
        rows.push(&item);
        now.push(entry);
    }
    paste::dispatch(host, &changes, &rows, Some(flag));
    (now, first)
}

/// The focus goes to the cell that changed, the window with it; with no cell —
/// a row came or went — to the new row, or the last one.
fn focus_after(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, at: Option<CellRef>) {
    let (target, window, rows) = {
        let borrowed = runtime.borrow();
        let rows = borrowed.state.row_count();
        let target = at.unwrap_or_else(|| {
            let col = borrowed.active.data().map_or(0, |cell| cell.col);
            CellRef::new(rows.saturating_sub(1), col)
        });
        (target, borrowed.state.window(), rows)
    };
    if rows == 0 {
        render(host, false);
        return;
    }
    let target = CellRef::new(target.row.min(rows - 1), target.col);
    let from = runtime.borrow().active;
    {
        // The row the focus stood on is pinned and would keep showing what it
        // showed; a step changes cells and rows under it, so nothing is pinned.
        let mut borrowed = runtime.borrow_mut();
        borrowed.slots.iter_mut().for_each(|slot| *slot = None);
        borrowed.set_active(ActiveCell::Data(target));
    }
    if target.row < window.offset || target.row >= window.offset + window.count {
        let pool = pool_of(host);
        let offset = grid::window_offset_for_row(target.row, rows, pool);
        runtime
            .borrow_mut()
            .state
            .set_window(Window::new(offset, pool));
        run_query(host, QueryKind::Window, true);
        return;
    }
    render(host, false);
    if let Some(root) = host.shadow_root() {
        let viewport = runtime.borrow().viewport.clone();
        focus_from(&root, viewport.as_ref(), from, ActiveCell::Data(target));
    }
    // The focused row is the one the renderer leaves alone.
    redraw_cell(host, target);
}
