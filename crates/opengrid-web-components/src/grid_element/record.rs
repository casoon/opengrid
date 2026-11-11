//! A whole record as a form (issue #325).
//!
//! `Shift`+`F2` on a data cell opens the focused record in a native modal
//! `<dialog>` the grid builds from the schema: one labelled field per column,
//! each the editor its cell would open ([`build_editor`]). The page behind it is
//! inert, `Tab` stays inside and `Escape` closes it — all by the platform.
//!
//! Before it opens, a cancelable `opengrid-record-open` lets a page that has a
//! form of its own take over. **Saving is cell edits:** every field is read and
//! checked first, and nothing is saved until all are fine; then one
//! `opengrid-cell-change` fires per changed field, in column order, and the
//! fields are **one** step of the history ([`Step::Record`]).

use super::*;

use opengrid_grid::{CellEdit, Level, Step};
use web_sys::HtmlDialogElement;

const DIALOG: &str = "[part=\"record-dialog\"]";

/// Whether the record under `cell` can be opened as a form: the page names it
/// (`row-key`), it is a row of the result, and the grid shows plain rows.
pub(super) fn applicable(host: &HtmlElement, runtime: &GridRuntime, cell: CellRef) -> bool {
    has_row_key(host)
        && !is_grouped(host)
        && !is_tree(host)
        && runtime.state.row_kind(cell.row) == RowKind::Source
        && runtime.state.row_key(cell.row).is_some()
}

pub(super) fn in_dialog(target: &Element) -> bool {
    target.closest(DIALOG).ok().flatten().is_some()
}

/// What a field holds, as its editor shows it: the cell's text, or for a list
/// the JSON array the combobox reads its chips from.
fn current_text(runtime: &GridRuntime, cell: CellRef) -> String {
    match runtime.state.cell(cell) {
        Some(list @ Value::List(_)) => opengrid_json::ToJson::to_json(list).to_string(),
        Some(value) => crate::formats::plain_text(value),
        None => String::new(),
    }
}

/// Opens the form on the record at `cell`, unless the page keeps it from it.
pub(super) fn open(host: &HtmlElement, cell: CellRef) {
    let (Some(root), Some(runtime), Some(document)) =
        (host.shadow_root(), runtime(host), host.owner_document())
    else {
        return;
    };
    if matches!(root.query_selector(DIALOG), Ok(Some(_))) {
        return;
    }
    let detail = js_sys::Object::new();
    let key = row_key_js(host, &runtime.borrow().state, cell.row).unwrap_or(JsValue::NULL);
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("key"), &key);
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("row"),
        &JsValue::from_f64(cell.row as f64),
    );
    if !crate::element::dispatch_cancelable(host, RECORD_OPEN_EVENT, &detail) {
        return;
    }

    let texts = texts(host);
    let Some(dialog) = builder::element(
        host,
        "dialog",
        "<form method=\"dialog\" novalidate><h2 id=\"og-record-title\"></h2>\
         <p id=\"og-record-problem\" role=\"alert\" data-dialog-problem hidden></p>\
         <div data-record-fields></div>\
         <div data-dialog-actions>\
         <button type=\"button\" data-record=\"cancel\"></button>\
         <button type=\"submit\"></button></div></form>",
    ) else {
        return;
    };
    builder::say(
        &dialog,
        "h2, [data-dialog-actions] > button",
        &[
            &texts.record_title(cell.row + 1),
            &texts.cancel,
            &texts.record_save,
        ],
    );
    for (name, value) in [
        ("part", "record-dialog"),
        ("aria-labelledby", "og-record-title"),
        ("lang", texts.lang.trim()),
        ("data-row", &cell.row.to_string()),
        ("data-from-col", &cell.col.to_string()),
    ] {
        if !value.is_empty() {
            let _ = dialog.set_attribute(name, value);
        }
    }
    let Ok(Some(fields)) = dialog.query_selector("[data-record-fields]") else {
        return;
    };
    let schema: Vec<_> = runtime.borrow().state.schema().fields().to_vec();
    let mut first: Option<Element> = None;
    for (col, field) in schema.iter().enumerate() {
        let name = field.name.as_str();
        let field_cell = CellRef::new(cell.row, col);
        let current = current_text(&runtime.borrow(), field_cell);
        let Some(editor) =
            build_editor(host, &document, col, name, field.data_type, &current, None)
        else {
            continue;
        };
        let label = texts.column(name).to_owned();
        let _ = editor.set_attribute("aria-label", &label);
        let _ = editor.set_attribute("data-record-col", &col.to_string());
        let (Ok(wrap), Ok(caption), Ok(note)) = (
            document.create_element("div"),
            document.create_element("label"),
            document.create_element("span"),
        ) else {
            continue;
        };
        let _ = wrap.set_attribute("data-record-field", "");
        caption.set_text_content(Some(&label));
        let _ = note.set_attribute("data-record-error", "");
        let _ = note.set_attribute("id", &format!("og-record-error-{col}"));
        let _ = note.set_attribute("hidden", "");
        // A column the page fills, or a list with nothing to pick from, is
        // shown and not edited — as its cell is.
        let readonly = presentation::styles(host).readonly(name)
            || (field.data_type.is_list() && combo::choices_of(host, col).is_none());
        if readonly {
            let _ = editor.set_attribute("disabled", "");
            let _ = wrap.set_attribute("data-readonly", "");
        } else if first.is_none() || col == cell.col {
            // The focus goes to the field of the cell it came from.
            first = Some(editor.clone());
        }
        // `for` names the first control inside it: a combobox has the input.
        let target = if combo::is_combo(&editor) {
            editor.query_selector("input").ok().flatten()
        } else {
            Some(editor.clone())
        };
        let _ = wrap.append_child(&caption);
        let _ = wrap.append_child(&editor);
        let _ = wrap.append_child(&note);
        let _ = fields.append_child(&wrap);
        if let Some(target) = target {
            let id = format!("og-record-field-{col}");
            let _ = target.set_attribute("id", &id);
            let _ = caption.set_attribute("for", &id);
            // The label names the control; a second name would be read twice.
            if !combo::is_combo(&editor) {
                let _ = editor.remove_attribute("aria-label");
            }
        }
    }
    let Ok(dialog) = dialog.dyn_into::<HtmlDialogElement>() else {
        return;
    };
    let owner = host.clone();
    let listener = Closure::<dyn FnMut(Event)>::new(move |event: Event| on_event(&owner, &event))
        .into_js_value();
    for kind in ["click", "submit", "close"] {
        let _ = dialog.add_event_listener_with_callback(kind, listener.unchecked_ref());
    }
    let _ = root.append_child(&dialog);
    let _ = dialog.show_modal();
    if let Some(editor) = first {
        focus_editor(&editor);
    }
}

fn on_event(host: &HtmlElement, event: &Event) {
    let Some(dialog) = event
        .current_target()
        .and_then(|target| target.dyn_into::<HtmlDialogElement>().ok())
    else {
        return;
    };
    match event.type_().as_str() {
        "submit" => {
            event.prevent_default();
            apply(host, &dialog);
        }
        "close" => {
            dialog.remove();
            focus_active(host);
        }
        _ => {
            let on_cancel = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
                .is_some_and(|target| {
                    target
                        .closest("button[data-record=\"cancel\"]")
                        .ok()
                        .flatten()
                        .is_some()
                });
            if on_cancel {
                dialog.close();
            }
        }
    }
}

/// One field, read and checked.
struct Read {
    col: usize,
    value: Value,
    text: String,
    finding: Option<opengrid_grid::Finding>,
}

/// The value a field's text means in a column of `data_type`, read as a cell
/// editor reads it: empty is `NULL` where the column allows one.
fn read_value(data_type: DataType, nullable: bool, text: &str) -> Result<Value, String> {
    if text.is_empty() && !matches!(data_type, DataType::Utf8 | DataType::Bool) {
        return if nullable {
            Ok(Value::Null)
        } else {
            Err(String::new())
        };
    }
    let parsed = if data_type.is_list() {
        Value::from_wire_str(text, &data_type).ok()
    } else {
        grid::literal(text, data_type)
            .and_then(|json| Value::from_json_typed(&json, &data_type).ok())
    };
    parsed.ok_or_else(|| text.to_owned())
}

/// Reads every field, saves the changed ones as cell edits — or, if a field is
/// not fine, keeps the form open with the reason under it and saves nothing.
fn apply(host: &HtmlElement, dialog: &HtmlDialogElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(row) = dialog
        .get_attribute("data-row")
        .and_then(|row| row.parse::<u64>().ok())
    else {
        return;
    };
    let texts = texts(host);
    let schema: Vec<_> = runtime.borrow().state.schema().fields().to_vec();

    let mut reads: Vec<Read> = Vec::new();
    let mut first_problem: Option<(Element, String)> = None;
    for (col, field) in schema.iter().enumerate() {
        let Ok(Some(editor)) = dialog.query_selector(&format!("[data-record-col=\"{col}\"]"))
        else {
            continue;
        };
        let Ok(Some(note)) = dialog.query_selector(&format!("#og-record-error-{col}")) else {
            continue;
        };
        let target = if combo::is_combo(&editor) {
            editor.query_selector("input").ok().flatten()
        } else {
            Some(editor.clone())
        };
        // A field the grid does not edit has nothing to read.
        if editor.has_attribute("disabled") {
            continue;
        }
        let name = field.name.as_str();
        let text = editor_text(&editor);
        let mut problem: Option<String> = None;
        let mut read = None;
        match read_value(field.data_type, field.nullable, &text) {
            Ok(value) => {
                let cell = CellRef::new(row, col);
                let finding = validate::run(host, &runtime, cell, &text);
                match &finding {
                    Some((found, true)) if found.level == Level::Error => {
                        problem = Some(found.message.clone());
                    }
                    _ => {}
                }
                read = Some(Read {
                    col,
                    value,
                    text,
                    finding: finding.map(|(found, _)| found),
                });
            }
            Err(bad) if bad.is_empty() => problem = Some(texts.cell_required(name)),
            Err(bad) => problem = Some(texts.filter_invalid(name, &bad)),
        }
        let parent = editor.parent_element();
        if let Some(problem) = &problem {
            note.set_text_content(Some(problem));
            let _ = note.remove_attribute("hidden");
            if let Some(target) = &target {
                let _ = target.set_attribute("aria-invalid", "true");
                let _ = target.set_attribute("aria-describedby", &note.id());
            }
            if let Some(parent) = &parent {
                let _ = parent.set_attribute("data-invalid", "");
            }
            if first_problem.is_none()
                && let Some(target) = &target
            {
                first_problem = Some((target.clone(), problem.clone()));
            }
        } else {
            note.set_text_content(None);
            let _ = note.set_attribute("hidden", "");
            if let Some(target) = &target {
                let _ = target.remove_attribute("aria-invalid");
                let _ = target.remove_attribute("aria-describedby");
            }
            if let Some(parent) = &parent {
                let _ = parent.remove_attribute("data-invalid");
            }
        }
        if let Some(read) = read {
            reads.push(read);
        }
    }

    if let Some((target, message)) = first_problem {
        if let Ok(Some(summary)) = dialog.query_selector("[data-dialog-problem]") {
            summary.set_text_content(Some(&message));
            let _ = summary.remove_attribute("hidden");
        }
        if let Ok(target) = target.dyn_into::<HtmlElement>() {
            let _ = target.focus();
        }
        return;
    }

    let mut edits: Vec<CellEdit> = Vec::new();
    let mut changed: Vec<CellRef> = Vec::new();
    for read in reads {
        let cell = CellRef::new(row, read.col);
        let name = schema[read.col].name.as_str().to_owned();
        let (before, record_key) = {
            let borrowed = runtime.borrow();
            (
                borrowed.state.cell(cell).cloned().unwrap_or(Value::Null),
                borrowed.state.row_key(row).cloned(),
            )
        };
        let key = row_key_js(host, &runtime.borrow().state, row);
        let moved = !runtime
            .borrow_mut()
            .state
            .set_cell(cell, read.value.clone())
            .is_empty();
        if moved {
            dispatch_cell_change(
                host,
                cell,
                key.as_ref(),
                &name,
                &read.text,
                &crate::formats::event_text(&before),
                None,
            );
            edits.push(CellEdit {
                row,
                col: read.col,
                key: record_key,
                before,
                after: read.value,
            });
            changed.push(cell);
        }
        runtime
            .borrow_mut()
            .state
            .set_validation(cell, read.finding);
    }

    let said = if edits.is_empty() {
        texts.record_nothing(row + 1)
    } else {
        texts.record_done(row + 1, edits.len())
    };
    if !edits.is_empty() {
        runtime
            .borrow_mut()
            .state
            .history_mut()
            .record(Step::Record(edits));
    }
    runtime.borrow_mut().state.set_notice(said);
    dialog.close();
    render(host, true);
    for cell in changed {
        redraw_cell(host, cell);
    }
}
