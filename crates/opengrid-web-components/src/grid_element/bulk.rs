//! One value for every selected row (issue #187).
//!
//! With rows selected, `F2` — or "Set for selected rows" in a column's menu —
//! opens that column's editor **once**, in a small panel above the rows.
//! Committing fires one `opengrid-bulk-change` with the keys of the selected
//! records, the column and the value, and marks the cells unsaved. The page
//! names records, so this needs a `row-key` — with one the selection is the
//! records' keys already (issue #322); a column the page fills (`readonly`)
//! refuses it.

use super::*;

use opengrid_grid::paste::{Notation, parse_cell};

/// Whether rows are selected on a grid that can name them: it has a `row-key`
/// and shows plain rows. `F2` then means "set for the selected rows".
pub(super) fn applicable(host: &HtmlElement, runtime: &GridRuntime) -> bool {
    has_row_key(host)
        && !runtime.picked.is_empty()
        && runtime.grouping.is_none()
        && runtime.tree.is_none()
}

/// Whether column `col` offers it in its menu: only what does something is
/// shown, so a column the page fills does not.
pub(super) fn available(host: &HtmlElement, runtime: &GridRuntime, col: usize) -> bool {
    let Some(field) = runtime.state.schema().fields().get(col) else {
        return false;
    };
    applicable(host, runtime) && !presentation::styles(host).readonly(field.name.as_str())
}

/// Opens the panel for column `col`. A read-only column is said to refuse.
pub(super) fn open(host: &HtmlElement, col: usize) {
    let (Some(runtime), Some(root), Some(document)) =
        (runtime(host), host.shadow_root(), host.owner_document())
    else {
        return;
    };
    let texts = texts(host);
    let (field, count) = {
        let borrowed = runtime.borrow();
        let Some(field) = borrowed.state.schema().fields().get(col).cloned() else {
            return;
        };
        (field, borrowed.picked.len() as u64)
    };
    if presentation::styles(host).readonly(field.name.as_str()) {
        runtime
            .borrow_mut()
            .state
            .set_notice(texts.bulk_readonly(field.name.as_str()));
        render(host, false);
        return;
    }
    close(&root);
    let Ok(Some(viewport)) = root.query_selector("[part=\"viewport\"]") else {
        return;
    };
    let Some(layout) = viewport.parent_element() else {
        return;
    };
    let make = |tag: &str| document.create_element(tag).ok();
    let (Some(panel), Some(caption)) = (make("div"), make("p")) else {
        return;
    };
    let _ = panel.set_attribute("part", grid::BULK_PARTS[0]);
    let _ = panel.set_attribute("role", "group");
    let _ = panel.set_attribute("data-col", &col.to_string());
    let sentence = texts.bulk_caption(field.name.as_str(), count);
    let _ = panel.set_attribute("aria-label", &sentence);
    if !texts.lang.trim().is_empty() {
        let _ = panel.set_attribute("lang", &texts.lang);
    }
    caption.set_text_content(Some(&sentence));
    let _ = panel.append_child(&caption);
    let Some(editor) = build_editor(
        host,
        &document,
        col,
        field.name.as_str(),
        field.data_type,
        "",
        None,
    ) else {
        return;
    };
    let _ = panel.append_child(&editor);
    for (part, label) in [
        (grid::BULK_PARTS[1], &texts.bulk_apply),
        (grid::BULK_PARTS[2], &texts.cancel),
    ] {
        if let Some(button) = make("button") {
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("part", part);
            button.set_text_content(Some(label));
            let _ = panel.append_child(&button);
        }
    }
    let _ = layout.insert_before(&panel, Some(&viewport));
    focus_editor(&editor);
}

/// Removes the panel, if there is one.
fn close(root: &ShadowRoot) {
    if let Ok(Some(panel)) = root.query_selector("[part=\"bulk\"]") {
        panel.remove();
    }
}

pub(super) fn in_panel(target: &Element) -> bool {
    target.closest("[part=\"bulk\"]").ok().flatten().is_some()
}

/// `Enter` sets, `Escape` drops; a button does what a button does.
pub(super) fn on_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    match event.key().as_str() {
        "Enter" if target.tag_name().eq_ignore_ascii_case("button") => {}
        "Enter" => {
            event.prevent_default();
            apply(host);
        }
        "Escape" => {
            event.prevent_default();
            cancel(host);
        }
        _ => {}
    }
}

pub(super) fn on_click(host: &HtmlElement, target: &Element) -> bool {
    let hit = |part: &str| {
        target
            .closest(&format!("[part=\"{part}\"]"))
            .ok()
            .flatten()
            .is_some()
    };
    if hit(grid::BULK_PARTS[1]) {
        apply(host);
        return true;
    }
    if hit(grid::BULK_PARTS[2]) {
        cancel(host);
        return true;
    }
    false
}

fn cancel(host: &HtmlElement) {
    if let Some(root) = host.shadow_root() {
        close(&root);
    }
    render(host, true);
}

/// Reads the editor and sets the value for the selected records.
pub(super) fn apply(host: &HtmlElement) {
    {
        let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
            return;
        };
        let Ok(Some(panel)) = root.query_selector("[part=\"bulk\"]") else {
            return;
        };
        let Some(col) = panel
            .get_attribute("data-col")
            .and_then(|col| col.parse::<usize>().ok())
        else {
            return;
        };
        let typed = panel
            .query_selector("[part=\"editor\"]")
            .ok()
            .flatten()
            .map(|editor| editor_text(&editor))
            .unwrap_or_default();
        let texts = texts(host);
        let Some(field) = runtime.borrow().state.schema().fields().get(col).cloned() else {
            return;
        };
        let name = field.name.as_str().to_owned();
        // What the editor holds is in the wire notation already (a typed input
        // gives `1234.5`), so the page's number notation is not asked.
        let Some(value) = parse_cell(
            &typed,
            field.data_type,
            field.nullable,
            &Notation::default(),
        ) else {
            let message = if typed.trim().is_empty() {
                texts.cell_required(&name)
            } else {
                texts.filter_invalid(&name, &typed)
            };
            runtime.borrow_mut().state.set_notice(message);
            render(host, false);
            return;
        };

        let keys = runtime.borrow().picked.keys();
        close(&root);
        // A bulk change is not a step of the history (one event cannot carry a
        // different old value per record): it ends the history instead.
        runtime.borrow_mut().state.history_mut().clear();
        let rows: Vec<u64> = runtime.borrow().state.selection().to_vec();
        for row in &rows {
            runtime
                .borrow_mut()
                .state
                .set_cell(CellRef::new(*row, col), value.clone());
        }

        let detail = js_sys::Object::new();
        let list = js_sys::Array::new();
        for key in &keys {
            list.push(&to_js(key));
        }
        for (property, item) in [
            ("keys", list.into()),
            ("column", JsValue::from_str(&name)),
            (
                "value",
                JsValue::from_str(&crate::formats::event_text(&value)),
            ),
        ] {
            let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(property), &item);
        }
        crate::element::dispatch(host, BULK_EVENT, &detail);

        runtime
            .borrow_mut()
            .state
            .set_notice(texts.bulk_done(&name, keys.len() as u64));
        render(host, true);
        // The focused row is the one the renderer leaves alone.
        for row in rows {
            redraw_cell(host, CellRef::new(row, col));
        }
    }
}
