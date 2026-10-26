//! One value for every selected row (issue #187).
//!
//! With rows selected, `F2` — or "Set for selected rows" in a column's menu —
//! opens that column's editor **once**, in a small panel above the rows.
//! Committing fires one `opengrid-bulk-change` with the keys of the selected
//! records, the column and the value, and marks the cells unsaved. A selection
//! names positions, the page names records, so this needs a `row-key`; a
//! column the page fills (`readonly`) refuses it.

use super::*;

use opengrid_grid::paste::{Notation, parse_cell};

/// Whether rows are selected on a grid that can name them: it has a `row-key`
/// and shows plain rows. `F2` then means "set for the selected rows".
pub(super) fn applicable(host: &HtmlElement, runtime: &GridRuntime) -> bool {
    host.has_attribute(grid::ROW_KEY_ATTRIBUTE)
        && !runtime.state.selection().is_empty()
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
        (field, borrowed.state.selection().len() as u64)
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
    if let Ok(editor) = editor.dyn_into::<HtmlElement>() {
        let _ = editor.focus();
    }
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

/// Reads the editor, finds the keys of the selected records and sets the value.
fn apply(host: &HtmlElement) {
    let host = host.clone();
    spawn_local(async move {
        let (Some(runtime), Some(root)) = (runtime(&host), host.shadow_root()) else {
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
            .map(
                |editor| match editor.clone().dyn_into::<HtmlInputElement>() {
                    Ok(input) if input.type_() == "checkbox" => input.checked().to_string(),
                    Ok(input) => input.value(),
                    Err(_) => editor
                        .dyn_into::<HtmlSelectElement>()
                        .map(|select| select.value())
                        .unwrap_or_default(),
                },
            )
            .unwrap_or_default();
        let texts = texts(&host);
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
            render(&host, false);
            return;
        };

        let Some(keys) = selected_keys(&host, &runtime).await else {
            return;
        };
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
            list.push(&to_js(&opengrid_json::ToJson::to_json(key)));
        }
        for (property, item) in [
            ("keys", list.into()),
            ("column", JsValue::from_str(&name)),
            (
                "value",
                JsValue::from_str(&crate::formats::plain_text(&value)),
            ),
        ] {
            let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(property), &item);
        }
        let init = web_sys::CustomEventInit::new();
        init.set_bubbles(true);
        init.set_composed(true);
        init.set_cancelable(false);
        init.set_detail(&detail);
        if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(BULK_EVENT, &init) {
            let _ = host.dispatch_event(&event);
        }

        runtime
            .borrow_mut()
            .state
            .set_notice(texts.bulk_done(&name, rows.len() as u64));
        render(&host, true);
        // The focused row is the one the renderer leaves alone.
        for row in rows {
            redraw_cell(&host, CellRef::new(row, col));
        }
    });
}

/// The `row-key` values of the selected rows: the loaded ones are read, the
/// rest are asked of the source in pieces of the sorted, filtered result.
async fn selected_keys(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
) -> Option<Vec<Value>> {
    const PIECE: u64 = 500;
    let provider = provider(host)?;
    let source = host.get_attribute(DATASOURCE_ATTRIBUTE)?;
    let key_name = row_key_of(host)?;
    let (selected, schema, sorts, filter) = {
        let borrowed = runtime.borrow();
        (
            borrowed.state.selection().to_vec(),
            borrowed.state.schema().clone(),
            borrowed.state.sort_keys(),
            effective_filter(host, &borrowed, None).ok()?,
        )
    };
    let key_col = schema
        .fields()
        .iter()
        .position(|field| field.name == key_name)?;
    let mut keys: Vec<Option<Value>> = vec![None; selected.len()];
    {
        let borrowed = runtime.borrow();
        for (slot, row) in keys.iter_mut().zip(&selected) {
            *slot = borrowed.state.row_key(*row).cloned();
        }
    }
    let columns = columns_of(host);
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let mut index = 0;
    while index < selected.len() {
        if keys[index].is_some() {
            index += 1;
            continue;
        }
        let piece_start = selected[index] / PIECE * PIECE;
        let query = grid::query_json(
            &source,
            &columns,
            &sorts,
            filter.as_ref(),
            piece_start,
            PIECE,
        );
        let result = match ask(host, &provider, runtime, &query, &mode).await {
            Ok(result) => result,
            Err(message) => {
                runtime
                    .borrow_mut()
                    .state
                    .set_notice(texts(host).error(&message));
                render(host, false);
                return None;
            }
        };
        let column = result.columns.get(key_col)?;
        while index < selected.len() && selected[index] < piece_start + PIECE {
            if keys[index].is_none() {
                keys[index] = column
                    .get((selected[index] - piece_start) as usize)
                    .cloned();
            }
            index += 1;
        }
    }
    keys.into_iter().collect()
}
