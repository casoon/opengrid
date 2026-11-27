//! Editing: the cell editor and the row the reader adds — opening it, committing or dropping what was typed, and the record a new row becomes.

use super::*;

/// Whether the page locked editing for the whole grid (issue #494). The one
/// question every way into an edit asks — the editor, paste, the bulk panel,
/// the record form, the new row, undo and redo — so no path keeps its own.
pub(super) fn edit_locked(host: &HtmlElement) -> bool {
    host.has_attribute(grid::EDIT_LOCKED_ATTRIBUTE)
}

/// Whether the page keeps column `field` from being edited (issues #153, #523,
/// #527): it fills it, it is the `row-key` column the page did not open, or the
/// page marked other columns for editing and not this one. What the reader may
/// lock is what this lets through.
pub(super) fn page_readonly_column(host: &HtmlElement, field: &opengrid_types::Field) -> bool {
    let key = row_key_of(host);
    let styles = presentation::styles(host);
    let name = field.name.as_str();
    derived_column(host, name)
        || styles.readonly(name, key.as_ref().map(|key| key.as_str()))
        || styles.unmarked(name)
}

/// Whether the source's schema says `name` is derived (issue #528): a year or a
/// month of a date, with nothing to write back to. Read-only with no opt-in —
/// not even `readonly: false` opens it.
pub(super) fn derived_column(host: &HtmlElement, name: &str) -> bool {
    runtime(host).is_some_and(|runtime| runtime.borrow().derived.contains(name))
}

/// Asks the provider once what the source's schema is, to learn which fields are
/// derived (issue #528): a result's schema never says. A provider that cannot say
/// leaves every field as it is — no derived field is known. The answer lands
/// later; one that is out of date by then, or was refused, is dropped.
pub(super) fn describe_source(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let (Some(provider), Some(source)) = (provider(host), host.get_attribute(DATASOURCE_ATTRIBUTE))
    else {
        return;
    };
    let Some(promise) = provider.describe(&source) else {
        set_derived(host, std::collections::BTreeSet::new());
        return;
    };
    let ask = {
        let mut runtime = runtime.borrow_mut();
        runtime.describing += 1;
        runtime.describing
    };
    let host = host.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let derived = match JsFuture::from(promise).await {
            Ok(answer) => derived_fields(&answer),
            Err(_) => std::collections::BTreeSet::new(),
        };
        let current =
            super::runtime(&host).is_some_and(|runtime| runtime.borrow().describing == ask);
        if current {
            set_derived(&host, derived);
        }
    });
}

/// The names of the fields with a `from` in a provider's `{ schema: { fields } }`.
fn derived_fields(answer: &JsValue) -> std::collections::BTreeSet<String> {
    let get =
        |object: &JsValue, key: &str| js_sys::Reflect::get(object, &JsValue::from_str(key)).ok();
    let fields = get(answer, "schema")
        .and_then(|schema| get(&schema, "fields"))
        .map(|fields| js_sys::Array::from(&fields))
        .unwrap_or_default();
    fields
        .iter()
        .filter(|field| {
            get(field, "from").is_some_and(|from| !from.is_null() && !from.is_undefined())
        })
        .filter_map(|field| get(&field, "name")?.as_string())
        .collect()
}

/// Takes what the source says is derived. A change closes what the reader had
/// open on a column that is read-only now and draws again; a derived field the
/// page marked `editable` is reported.
fn set_derived(host: &HtmlElement, derived: std::collections::BTreeSet<String>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if runtime.borrow().derived == derived {
        return;
    }
    runtime.borrow_mut().derived = derived;
    edit_state_changed(host, true);
    // Checks the page's columns against what is known now, and says what
    // contradicts it.
    recolumn(host);
}

/// Whether column `field` never opens an editor, whatever the lock (issues #153,
/// #523, #527): the page keeps it from being edited, or the reader locked it.
/// The one answer the editor, paste, the bulk panel, the record form, the new
/// row and the marks of the cells all take.
pub(super) fn readonly_column(host: &HtmlElement, field: &opengrid_types::Field) -> bool {
    page_readonly_column(host, field)
        || columns::layout(host)
            .borrow()
            .is_locked(field.name.as_str())
}

/// Applies a change of what may be edited — the attribute `edit-locked`, or a
/// column the reader locked (issue #527). With `closing` what the reader had
/// open is dropped unconfirmed; the new row goes or comes back. No event, no
/// query: the data, the sort, the filter, the scroll position and the focus stay.
pub(super) fn edit_state_changed(host: &HtmlElement, closing: bool) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if closing {
        end_edit(host, false);
        paste::discard(host);
        if let Some(root) = host.shadow_root() {
            bulk::close(&root);
            record::close_dialog(&root);
        }
    }
    sync_add_row(host, &runtime);
    render(host, false);
    sync_edit_toggle(host);
}

/// Locks or unlocks a column for the reader, from the column menu (issue #527).
/// The lock narrows what the page allows and never widens it: a column the page
/// keeps from editing offers neither.
pub(super) fn set_column_locked(host: &HtmlElement, name: &str, locked: bool) {
    if !columns::update(host, |layout| layout.set_locked(name, locked)) {
        return;
    }
    edit_state_changed(host, locked);
    let texts = texts(host);
    let said = if locked {
        texts.column_locked(name)
    } else {
        texts.column_unlocked(name)
    };
    announce(host, &said);
    dispatch_view(host);
}

/// Whether the toolbar's switch offers editing: it is the inverse of the lock.
pub(super) fn sync_edit_toggle(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    if let Ok(Some(button)) = root.query_selector("[data-toolbar=\"edit\"]") {
        let _ = button.set_attribute("aria-pressed", &(!edit_locked(host)).to_string());
    }
}

/// The toolbar's "Edit" switch (issue #527): it sets and removes `edit-locked`,
/// which is the whole state. What it says is said with the change.
pub(super) fn toggle_editing(host: &HtmlElement) {
    let locking = !edit_locked(host);
    let texts = texts(host);
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().state.set_notice(if locking {
            texts.edit_locked.clone()
        } else {
            texts.edit_unlocked.clone()
        });
    }
    let _ = if locking {
        host.set_attribute(grid::EDIT_LOCKED_ATTRIBUTE, "")
    } else {
        host.remove_attribute(grid::EDIT_LOCKED_ATTRIBUTE)
    };
}

/// Writes the footer's edit hint (issue #523): that editing is locked, for as
/// long as it is, or how the focused cell opens when its column is one the
/// page means to be edited. Empty otherwise — a hint on every cell of a grid
/// that is only read would be noise.
pub(super) fn show_edit_hint(host: &HtmlElement) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let Ok(Some(node)) = root.query_selector("[part=\"edit-hint\"]") else {
        return;
    };
    let texts = texts(host);
    let text = if edit_locked(host) {
        texts.edit_locked_hint.clone()
    } else {
        let active = runtime.borrow().active;
        let marked = match active {
            ActiveCell::Data(cell) => runtime
                .borrow()
                .decorated
                .as_ref()
                .is_some_and(|decorated| decorated.mark(cell.col) == Some("editable")),
            _ => false,
        };
        if marked && (!is_grouped(host) || editable_group_row(host, active)) {
            texts.edit_hint.clone()
        } else {
            String::new()
        }
    };
    if node.text_content().unwrap_or_default() != text {
        node.set_text_content(Some(&text));
    }
}

/// Says so in the status line and answers `true` when editing is locked: the
/// attempt is refused out loud, not dropped.
pub(super) fn refuse_locked(host: &HtmlElement) -> bool {
    if !edit_locked(host) {
        return false;
    }
    if let Some(runtime) = runtime(host) {
        runtime
            .borrow_mut()
            .state
            .set_notice(texts(host).edit_locked.clone());
        render(host, false);
    }
    true
}

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

/// After `Enter` committed a cell of a record, with `enter-moves="down"`: the
/// focus goes to the same column of the next **data** row — `Shift`+`Enter` of
/// the one before — passing group headers and the total, and opens nothing; the
/// next `Enter` does. At the last row the focus stays, and so it does when the
/// commit did not happen: a validator held it back, the input was refused
/// (issue #536). The new row has its own way on, [`after_new_row_enter`].
pub(super) fn after_enter_commit(host: &HtmlElement, cell: CellRef, committed: bool, up: bool) {
    if !committed || host.get_attribute(grid::ENTER_MOVES_ATTRIBUTE).as_deref() != Some("down") {
        return;
    }
    let Some(runtime) = runtime(host) else {
        return;
    };
    let target = {
        let borrowed = runtime.borrow();
        if borrowed.state.row_kind(cell.row) == RowKind::Draft {
            return;
        }
        let total = borrowed.state.row_count();
        if up {
            (0..cell.row).rev().find(|row| is_record(&borrowed, *row))
        } else {
            (cell.row + 1..total).find(|row| is_record(&borrowed, *row))
        }
    };
    let Some(row) = target else {
        return;
    };
    let key = if up {
        GridKey::ArrowUp
    } else {
        GridKey::ArrowDown
    };
    go_to(
        host,
        &runtime,
        key,
        ActiveCell::Data(cell),
        ActiveCell::Data(CellRef::new(row, cell.col)),
    );
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

/// How this grid's cells become text: the page's formats, and the label of a
/// key where the column has a lookup (issue #326), and the scaled number where
/// it has a scale (issue #537).
pub(super) fn formatter(host: &HtmlElement, schema: &opengrid_types::Schema) -> Formatter {
    let styles = presentation::styles(host);
    Formatter::new(&formats(host), schema)
        .with_lookups(&styles, schema)
        .with_scales(&styles, schema)
}

/// Opens the editor on the focused data cell (plan point 37).
///
/// The editor is an ordinary form control inside the cell, typed after the
/// column: `text`, a text field for a number that reads the page's notation
/// ([`number`]), `date`, `checkbox`, or the type-ahead combobox when the page
/// supplied choices ([`combo`]). The same derivation as the filter row (point
/// 51) — one place decides what a column can hold.
///
/// `typed` is the character that opened it, if a character did: it **replaces**
/// the value in a text or number field, as in a spreadsheet, and in a combobox
/// it is the start of what is looked for. A date or a checkbox has no place
/// for one character; it opens on the cell's value.
pub(super) fn begin_edit(host: &HtmlElement, cell: CellRef, typed: Option<&str>) {
    if refuse_locked(host) {
        return;
    }
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
        // The page fills this column, it is derived, or it names the record
        // (issues #153, #523).
        if readonly_column(host, field) {
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
        // A number is a text field that reads the page's notation (issue #535),
        // not a `type="number"` the browser reads in its own.
        let numeric = number::is_number(data_type);
        let scale = number::scale_of(host, name);
        let kind = if numeric {
            number::set_up(&input, data_type, scale);
            "text"
        } else {
            let kind = grid::input_type(data_type);
            let _ = input.set_attribute("type", kind);
            kind
        };
        if kind == "checkbox" {
            if current == "true" {
                let _ = input.set_attribute("checked", "");
            }
        } else if let Ok(field) = input.clone().dyn_into::<HtmlInputElement>() {
            match typed {
                Some(typed) if kind != "date" => field.set_value(typed),
                _ if numeric => field.set_value(&number::opening_text(host, current, scale)),
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
/// document is a dead end for a keyboard. Answers `true` when the value was
/// taken — not discarded, not refused, not held back by a validator.
pub(super) fn end_edit(host: &HtmlElement, commit: bool) -> bool {
    let Some(runtime) = runtime(host) else {
        return false;
    };
    let Some(cell) = runtime.borrow().state.editing() else {
        return false;
    };
    let Some(root) = host.shadow_root() else {
        return false;
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
            None => return false,
        }
    };

    // A number the page's notation cannot read is refused with the reason, under
    // the editor, which stays open with what was typed — never committed as NULL
    // (issue #535). An empty field is the one way to clear a value, below.
    let numeric = number::applies(data_type, editor.as_ref());
    // A scaled column's editor holds the scaled number (`6,12`); it is read as,
    // and the page is told, the stored integer (`612`) (issue #537).
    let scale = number::scale_of(host, &field_name);
    if commit
        && numeric
        && let (Some(editor), Some(text)) = (&editor, typed.as_deref())
        && !text.is_empty()
        && let Err(message) = number::read(host, data_type, scale, text)
    {
        number::refuse(host, &runtime, editor, &message);
        return false;
    }

    // The page's validator speaks on commit too (issue #188). A column that asks
    // for it blocks on an error: the editor stays, with the sentence under it.
    let finding = if commit {
        let text = typed.as_deref().unwrap_or("");
        validate::run(
            host,
            &runtime,
            cell,
            &number::wire_text(host, data_type, scale, editor.as_ref(), text),
        )
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
        return false;
    }

    // Read before the commit: an edit of the key field itself still names the
    // record by the key it had.
    let key = row_key_js(host, &runtime.borrow().state, cell.row);
    // What is typed into the new row is not an edit of anything yet: the page
    // hears of it with the record (`opengrid-row-add`, issue #183).
    let in_draft = runtime.borrow().state.row_kind(cell.row) == RowKind::Draft;
    let mut rejected = None;
    // **An empty editor means NULL** — on a column that may hold one. It is the
    // only way to clear a value — a number that cannot be read is refused above,
    // not taken for it. On a required column it is a refusal, not an empty
    // string.
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
        } else if numeric {
            // Read above already: in the page's notation (issue #535).
            number::read(host, data_type, scale, text).ok()
        } else {
            grid::literal(text, data_type)
                .and_then(|json| Value::from_json_typed(&json, &data_type).ok())
        };
        match parsed {
            Some(value) => {
                // The page is told a number in the wire notation, as before.
                let told = if numeric {
                    crate::formats::event_text(&value)
                } else {
                    text.to_owned()
                };
                commit_cell(
                    host,
                    &runtime,
                    cell,
                    value,
                    key.as_ref(),
                    &field_name,
                    &told,
                    in_draft,
                );
            }
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
                .map(|value| formatter(host, borrowed.state.schema()).text(cell.col, value))
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
    commit && rejected.is_none()
}
