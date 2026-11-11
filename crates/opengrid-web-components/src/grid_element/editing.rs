//! Editing: the cell editor and the row the reader adds — opening it, committing or dropping what was typed, and the record a new row becomes.

use super::*;

/// After `Enter` committed a cell of the new row: on to the next column, and
/// from the last one — or with `Ctrl`/`Cmd` — the record is added (issue #183).
pub(super) fn after_new_row_enter(host: &HtmlElement, cell: CellRef, add: bool) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if runtime.borrow().state.row_kind(cell.row) != RowKind::Draft {
        return;
    }
    let last = runtime.borrow().state.schema().len().saturating_sub(1);
    if add || cell.col >= last {
        add_record(host);
        return;
    }
    runtime
        .borrow_mut()
        .set_active(ActiveCell::Data(CellRef::new(cell.row, cell.col + 1)));
    render(host, true);
}

/// Adds the record typed into the new row (issue #183): refused, with the cell
/// that lacks a value named, while a required one is empty; otherwise fired as
/// `opengrid-row-add`, shown at once as an unsaved row, with a fresh new row
/// under it holding the focus in its first column.
pub(super) fn add_record(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let lacking = {
        let borrowed = runtime.borrow();
        borrowed.state.added().lacking(borrowed.state.schema())
    };
    let draft_row = {
        let borrowed = runtime.borrow();
        borrowed.state.row_count().saturating_sub(1)
    };
    if let Some(&first) = lacking.first() {
        let message = {
            let borrowed = runtime.borrow();
            let name = borrowed.state.schema().fields()[first]
                .name
                .as_str()
                .to_owned();
            texts(host).cell_required(&name)
        };
        {
            let mut borrowed = runtime.borrow_mut();
            borrowed.state.added_mut().mark_missing(lacking.clone());
            borrowed.state.set_notice(message);
            borrowed.set_active(ActiveCell::Data(CellRef::new(draft_row, first)));
        }
        render(host, true);
        // The new row holds the focus, and the focused row is the one the
        // renderer leaves alone: the marks are written here.
        for col in lacking {
            redraw_cell(host, CellRef::new(draft_row, col));
        }
        return;
    }
    let (provisional, values, waiting) = {
        let mut borrowed = runtime.borrow_mut();
        let Some(row) = borrowed.state.added_mut().fire().cloned() else {
            return;
        };
        let fields = borrowed.state.schema().fields().to_vec();
        let values = fields
            .iter()
            .zip(&row.values)
            .map(|(field, value)| (field.name.as_str().to_owned(), value.clone()))
            .collect::<Vec<_>>();
        (row.provisional, values, borrowed.state.added().unsaved())
    };
    dispatch_row_add(host, &provisional, &values, None);
    runtime
        .borrow_mut()
        .state
        .history_mut()
        .record(opengrid_grid::Step::Added(opengrid_grid::AddedEntry {
            provisional: provisional.clone(),
            values: values.iter().map(|(_, value)| value.clone()).collect(),
        }));

    let (new_draft, pool) = {
        let mut borrowed = runtime.borrow_mut();
        let new_draft = borrowed.state.row_count().saturating_sub(1);
        borrowed.state.set_notice(texts(host).row_added(waiting));
        borrowed.set_active(ActiveCell::Data(CellRef::new(new_draft, 0)));
        let pool = borrowed
            .view
            .as_ref()
            .map_or_else(|| pool_of(host), |view| view.pool() as u64);
        (new_draft, pool)
    };
    // The new row may be below the window: the window follows it.
    let (offset, rows) = {
        let borrowed = runtime.borrow();
        (borrowed.state.window().offset, borrowed.state.row_count())
    };
    let wanted = grid::window_offset_for_row(new_draft, rows, pool);
    if new_draft >= offset + pool || new_draft < offset {
        scroll_to_far_row(&runtime, new_draft);
        runtime
            .borrow_mut()
            .state
            .set_window(Window::new(wanted, pool));
        run_query(host, QueryKind::Window, true);
    } else {
        render(host, true);
    }
}

/// `opengrid-row-add`: `{ values, provisionalKey }` — the values by field name,
/// in the wire notation of a result.
pub(super) fn dispatch_row_add(
    host: &HtmlElement,
    provisional: &str,
    values: &[(String, Value)],
    step: Option<&str>,
) {
    let object = js_sys::Object::new();
    for (name, value) in values {
        let _ = js_sys::Reflect::set(
            &object,
            &JsValue::from_str(name),
            &to_js(&opengrid_json::ToJson::to_json(value)),
        );
    }
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("values"), &object);
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("provisionalKey"),
        &JsValue::from_str(provisional),
    );
    if let Some(step) = step {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(step), &JsValue::TRUE);
    }
    crate::element::dispatch(host, ROW_ADD_EVENT, &detail);
}

/// [`crate::element::set_row_state`] (issue #183): what became of a row the
/// reader added. `detail` is the record's key for `saved`, the message for `error`.
pub(crate) fn write_row_state(
    host: &HtmlElement,
    provisional: &str,
    state: &str,
    detail: &JsValue,
) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(state) = opengrid_grid::RowState::parse(state) else {
        return;
    };
    let key = (state == opengrid_grid::RowState::Saved)
        .then(|| key_from_js(&runtime.borrow().state, detail))
        .flatten();
    let placed = runtime
        .borrow_mut()
        .state
        .added_mut()
        .set_state(provisional, state, key);
    if placed.is_none() {
        return;
    }
    // One live region (point 41), as `set_cell_state` has it: a failure is
    // said in the status line, politely, and the focus stays where it is.
    if state == opengrid_grid::RowState::Error
        && let Some(message) = detail.as_string()
    {
        runtime.borrow_mut().state.set_notice(message);
    }
    render(host, false);
}

/// Where `set_choices` stores what the page supplied.
pub(crate) const CHOICES_KEY: &str = "__opengridChoices";

/// Opens the editor on the focused data cell (plan point 37).
///
/// The editor is an ordinary form control inside the cell, typed after the
/// column: `text`, `number` with the column's step, `date`, `checkbox`, or the
/// type-ahead combobox when the page supplied choices ([`combo`]). The same
/// derivation as the filter row (point 51) — one place decides what a column
/// can hold.
///
/// `typed` is the character that opened it, if a character did: it **replaces**
/// the value in a text or number field, as in a spreadsheet, and in a combobox
/// it is the start of what is looked for. A date or a checkbox has no place
/// for one character; it opens on the cell's value.
pub(super) fn begin_edit(host: &HtmlElement, cell: CellRef, typed: Option<&str>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if is_grouped(host) && !editable_group_row(host, ActiveCell::Data(cell)) {
        return;
    }
    let (data_type, current) = {
        let borrowed = runtime.borrow();
        let Some(field) = borrowed.state.schema().fields().get(cell.col) else {
            return;
        };
        // The page fills this column; an edit would be overwritten by the
        // next value it sets (issue #153).
        if presentation::styles(host).readonly(field.name.as_str()) {
            return;
        }
        // A list is edited in a combobox over the page's choices; with none there
        // is nothing to pick from, and no editor (issue #185).
        if field.data_type.is_list() && combo::choices_of(host, cell.col).is_none() {
            return;
        }
        let current = match borrowed.state.cell(cell) {
            // The JSON array, which the combobox reads its chips from.
            Some(list @ Value::List(_)) => opengrid_json::ToJson::to_json(list).to_string(),
            Some(value) => crate::formats::plain_text(value),
            None => String::new(),
        };
        (field.data_type, current)
    };
    if runtime.borrow_mut().state.begin_edit(cell).is_empty() {
        return;
    }
    runtime.borrow_mut().validation_said = None;
    render(host, false);

    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };

    td.set_text_content(None);
    let name = runtime
        .borrow()
        .state
        .schema()
        .fields()
        .get(cell.col)
        .map(|field| field.name.as_str().to_owned())
        .unwrap_or_default();
    let Some(editor) = build_editor(host, &document, cell.col, &name, data_type, &current, typed)
    else {
        return;
    };
    let _ = td.append_child(&editor);
    focus_editor(&editor);
}

/// Puts the focus in an editor: in its text field, for a combobox.
pub(super) fn focus_editor(editor: &Element) {
    let target = if combo::is_combo(editor) {
        editor.query_selector("input").ok().flatten()
    } else {
        Some(editor.clone())
    };
    if let Some(element) = target.and_then(|target| target.dyn_into::<HtmlElement>().ok()) {
        let _ = element.focus();
        // A pick that is already there is replaced by typing, as in a select.
        if let Ok(field) = element.dyn_into::<HtmlInputElement>()
            && combo::is_combo(editor)
        {
            field.select();
        }
    }
}

/// The form control that edits a value of `data_type`: a `<select>` when the page
/// supplied choices, otherwise an input typed after the column — the same
/// derivation as the filter row (point 51), one place for the cell editor and
/// the bulk one. `typed` is the character that opened it, if one did.
pub(super) fn build_editor(
    host: &HtmlElement,
    document: &web_sys::Document,
    col: usize,
    name: &str,
    data_type: DataType,
    current: &str,
    typed: Option<&str>,
) -> Option<Element> {
    // A column with choices is edited in the type-ahead combobox (issue #185).
    if let Some(editor) = combo::build(host, document, col, name, data_type, current, typed) {
        return Some(editor);
    }
    let editor: Element = {
        let Ok(input) = document.create_element("input") else {
            return None;
        };
        let kind = grid::input_type(data_type);
        let _ = input.set_attribute("type", kind);
        if let Some(step) = grid::input_step(data_type) {
            let _ = input.set_attribute("step", &step);
        }
        if kind == "checkbox" {
            if current == "true" {
                let _ = input.set_attribute("checked", "");
            }
        } else if let Ok(field) = input.clone().dyn_into::<HtmlInputElement>() {
            match typed {
                Some(typed) if kind != "date" => field.set_value(typed),
                _ => field.set_value(current),
            }
        }
        input
    };
    let _ = editor.set_attribute("part", "editor");
    // The column name is the accessible name: the cell it sits in has none.
    let _ = editor.set_attribute("aria-label", name);
    Some(editor)
}

/// The text an editor holds: an input's value, a checkbox's state, the value of a
/// `<select>` — or what a combobox holds (issue #185).
pub(super) fn editor_text(editor: &Element) -> String {
    if combo::is_combo(editor) {
        return combo::text(editor);
    }
    match editor.clone().dyn_into::<HtmlInputElement>() {
        Ok(input) if input.type_() == "checkbox" => input.checked().to_string(),
        Ok(input) => input.value(),
        Err(_) => editor
            .clone()
            .dyn_into::<HtmlSelectElement>()
            .map(|select| select.value())
            .unwrap_or_default(),
    }
}

/// Closes the editor, optionally taking what it holds (plan point 37).
///
/// An input the column cannot hold is **refused and announced** — never
/// silently rounded, never silently dropped. Whatever happens, the focus goes
/// back to the cell: an editor that vanishes and leaves the focus on the
/// document is a dead end for a keyboard.
pub(super) fn end_edit(host: &HtmlElement, commit: bool) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(cell) = runtime.borrow().state.editing() else {
        return;
    };
    let Some(root) = host.shadow_root() else {
        return;
    };

    let editor = root.query_selector("[part=\"editor\"]").ok().flatten();
    let typed = editor.as_ref().map(editor_text);

    let (data_type, field_name, nullable) = {
        let borrowed = runtime.borrow();
        let field = borrowed.state.schema().fields().get(cell.col).cloned();
        match field {
            Some(field) => (
                field.data_type,
                field.name.as_str().to_owned(),
                field.nullable,
            ),
            None => return,
        }
    };

    // The page's validator speaks on commit too (issue #188). A column that asks
    // for it blocks on an error: the editor stays, with the sentence under it.
    let finding = if commit {
        validate::run(host, &runtime, cell, typed.as_deref().unwrap_or(""))
    } else {
        None
    };
    if let Some((found, true)) = &finding
        && found.level == opengrid_grid::Level::Error
    {
        if let Ok(Some(editor)) = root.query_selector("[part=\"editor\"]") {
            validate::show(&editor, Some(found));
            if let Ok(editor) = editor.dyn_into::<HtmlElement>() {
                let _ = editor.focus();
            }
        }
        validate::announce(host, &runtime, Some(found));
        return;
    }

    // Read before the commit: an edit of the key field itself still names the
    // record by the key it had.
    let key = row_key_js(host, &runtime.borrow().state, cell.row);
    // What is typed into the new row is not an edit of anything yet: the page
    // hears of it with the record (`opengrid-row-add`, issue #183).
    let in_draft = runtime.borrow().state.row_kind(cell.row) == RowKind::Draft;
    let mut rejected = None;
    // **An empty editor means NULL** — on a column that may hold one. It is the
    // only way to clear a value, and a typed input sanitizes most other
    // nonsense away before it ever gets here. On a required column it is a
    // refusal, not an empty string.
    if commit
        && typed.as_deref().is_some_and(str::is_empty)
        && !matches!(data_type, DataType::Utf8 | DataType::Bool)
    {
        if nullable {
            commit_cell(
                host,
                &runtime,
                cell,
                Value::Null,
                key.as_ref(),
                &field_name,
                "",
                in_draft,
            );
        } else {
            rejected = Some(String::new());
        }
    } else if commit && let Some(text) = typed.as_deref() {
        // One notation: the same parser the filter row and the wire format use,
        // so a value typed into a cell means what it would have meant anywhere
        // else (E13, S8, S9).
        let parsed = if data_type.is_list() {
            // The JSON array the combobox wrote (issue #185).
            Value::from_wire_str(text, &data_type).ok()
        } else {
            grid::literal(text, data_type)
                .and_then(|json| Value::from_json_typed(&json, &data_type).ok())
        };
        match parsed {
            Some(value) => commit_cell(
                host,
                &runtime,
                cell,
                value,
                key.as_ref(),
                &field_name,
                text,
                in_draft,
            ),
            None => rejected = Some(text.to_owned()),
        }
    }

    if commit && rejected.is_none() {
        let finding = finding.map(|(found, _)| found);
        validate::announce(host, &runtime, finding.as_ref());
        runtime.borrow_mut().state.set_validation(cell, finding);
    }
    runtime.borrow_mut().validation_said = None;
    runtime.borrow_mut().state.end_edit();
    if let Some(value) = &rejected {
        let texts = texts(host);
        let message = if value.is_empty() {
            texts.cell_required(&field_name)
        } else {
            texts.filter_invalid(&field_name, value)
        };
        runtime.borrow_mut().state.set_notice(message);
    }

    // **The element owns the editor node, so the element removes it.** The
    // edited cell sits in the focused row, and the focused slot is the one the
    // renderer never touches (point 17) — waiting for a patch to clear it would
    // wait forever.
    if let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) {
        let text = {
            let borrowed = runtime.borrow();
            borrowed
                .state
                .cell(cell)
                .map(|value| {
                    Formatter::new(&formats(host), borrowed.state.schema()).text(cell.col, value)
                })
                .unwrap_or_default()
        };
        td.set_text_content(Some(&text));
        if runtime.borrow().state.is_changed(cell) {
            let _ = td.set_attribute("data-changed", "true");
        }
    }

    render(host, false);
    // The focused row is the one the renderer leaves alone: the cell's marks —
    // the new row's, a validator's — are written here.
    redraw_cell(host, cell);
    focus_active(host);
}
