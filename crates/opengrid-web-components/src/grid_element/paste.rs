//! Pasting a block of cells from a spreadsheet (issue #184).
//!
//! `Ctrl`/`Cmd`+`V` on a data cell reads the clipboard's text, anchors it at
//! the focused cell and **shows what would happen** before anything changes:
//! each value read against its column, the ones that cannot be read marked with
//! the reason, a sentence in the live region. `Enter` applies — one
//! `opengrid-paste` event for the whole block — and `Escape` drops it.

use super::*;

use crate::texts::GridTexts;
use opengrid_grid::paste::{Notation, parse_cell, split_block};

/// What a paste would change in one existing row.
pub(super) struct Change {
    row: u64,
    key: Option<Value>,
    col: usize,
    column: String,
    value: Value,
    before: Value,
    previous: String,
}

/// A paste shown and not yet applied.
pub(super) struct Pending {
    changes: Vec<Change>,
    /// The rows past the end that become added rows, in schema order.
    added: Vec<Vec<Value>>,
    unreadable: usize,
    cut_off: usize,
}

/// The paste event: read the text, work out the block, show it.
pub(super) fn on_paste(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    // Into a field — the filter row, the search, an open editor — the browser
    // pastes as it always did.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target
            .closest("input, select, textarea, [part=\"editor\"]")
            .ok()
            .flatten()
            .is_some()
    {
        return;
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    // Grouped or in a tree a position is not a place to paste at.
    if is_grouped(&host) || is_tree(&host) {
        return;
    }
    let ActiveCell::Data(anchor) = runtime.borrow().active else {
        return;
    };
    if !matches!(
        runtime.borrow().state.row_kind(anchor.row),
        RowKind::Source | RowKind::Draft
    ) {
        return;
    }
    let text = event
        .dyn_ref::<web_sys::ClipboardEvent>()
        .and_then(|event| event.clipboard_data())
        .and_then(|data| data.get_data("text/plain").ok())
        .unwrap_or_default();
    let rows = split_block(&text);
    if rows.is_empty() {
        return;
    }
    event.prevent_default();
    spawn_local(async move {
        prepare(&host, anchor, rows).await;
    });
}

/// How the page writes numbers: `1.234,5` or `1,234.5`, from the language of the
/// page — the data is the page's, in the page's language, whatever the
/// component's own sentences are written in — and the browser's where the page
/// names none.
fn notation_of(host: &HtmlElement) -> Notation {
    let lang = host
        .closest("[lang]")
        .ok()
        .flatten()
        .and_then(|owner| owner.get_attribute("lang"))
        .unwrap_or_default();
    let lang = lang.trim();
    let sample = js_sys::Number::from(1_234_567.5_f64);
    let shown: String = sample
        .to_locale_string(if lang.is_empty() { "default" } else { lang })
        .into();
    let marks: Vec<char> = shown.chars().filter(|c| !c.is_ascii_digit()).collect();
    match marks.as_slice() {
        [] => Notation::default(),
        [decimal] => Notation {
            decimal: *decimal,
            group: None,
        },
        [group, .., decimal] => Notation {
            decimal: *decimal,
            group: Some(*group),
        },
    }
}

/// The name of a column's type, for the reason a value is refused.
fn type_name(texts: &GridTexts, data_type: DataType) -> String {
    match data_type {
        DataType::Utf8 => &texts.type_text,
        DataType::Bool => &texts.type_bool,
        DataType::Int64 => &texts.type_integer,
        DataType::Float64 | DataType::Decimal { .. } => &texts.type_number,
        DataType::Date => &texts.type_date,
        DataType::Timestamp => &texts.type_time,
        DataType::ListUtf8 | DataType::ListInt64 => &texts.type_list,
    }
    .clone()
}

/// Reads the rows the block lands on, parses the block against the columns it
/// lands in and shows the preview.
async fn prepare(host: &HtmlElement, anchor: CellRef, rows: Vec<Vec<String>>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(provider) = provider(host) else {
        return;
    };
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let (source_rows, add_row, schema, sorts, filter) = {
        let borrowed = runtime.borrow();
        (
            borrowed.state.total_count(),
            borrowed.state.added().enabled(),
            borrowed.state.schema().clone(),
            borrowed.state.sort_keys(),
            effective_filter(host, &borrowed, None),
        )
    };
    let Ok(filter) = filter else {
        return;
    };
    let width = schema.len();
    let notation = notation_of(host);
    let texts = texts(host);

    // The existing rows the block covers: their keys and what they hold now are
    // asked of the source, so a block longer than the window is the same as one
    // inside it.
    let first = anchor.row.min(source_rows);
    let covered = (anchor.row + rows.len() as u64).min(source_rows) - first;
    let mut existing: Option<opengrid_datasource::QueryResult> = None;
    if covered > 0 && anchor.row < source_rows {
        let columns = columns_of(host);
        let query = grid::query_json(&source, &columns, &sorts, filter.as_ref(), first, covered);
        let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
        match ask(host, &provider, &runtime, &query, &mode).await {
            Ok(result) => existing = Some(result),
            Err(message) => {
                runtime.borrow_mut().state.set_notice(texts.error(&message));
                render(host, false);
                return;
            }
        }
    }
    let key_column = row_key_of(host)
        .and_then(|name| schema.fields().iter().position(|field| field.name == name));

    let mut pending = Pending {
        changes: Vec::new(),
        added: Vec::new(),
        unreadable: 0,
        cut_off: 0,
    };
    // What the preview shows: per block row, per cell, the text and whether it read.
    let mut shown: Vec<(bool, Vec<(String, Option<String>)>)> = Vec::new();
    for (index, cells) in rows.iter().enumerate() {
        let row = anchor.row + index as u64;
        let is_new = row >= source_rows;
        let mut line: Vec<(String, Option<String>)> = Vec::new();
        let mut values: Vec<Value> = vec![Value::Null; width];
        let mut cut = 0;
        if is_new && !add_row {
            pending.cut_off += cells.len();
            continue;
        }
        if is_new
            && !matches!(
                runtime.borrow().state.row_kind(row),
                RowKind::Draft | RowKind::Source
            )
            && row < source_rows + runtime.borrow().state.added().rows().len() as u64
        {
            // A row added earlier waits for the page; it is not pasted over.
            pending.cut_off += cells.len();
            continue;
        }
        for (offset, text) in cells.iter().enumerate() {
            let col = anchor.col + offset;
            let Some(field) = schema.fields().get(col) else {
                cut += 1;
                continue;
            };
            match parse_cell(text, field.data_type, field.nullable, &notation) {
                Some(value) => {
                    if is_new {
                        values[col] = value;
                    } else {
                        let at = (row - first) as usize;
                        let before = existing
                            .as_ref()
                            .and_then(|result| result.columns.get(col)?.get(at))
                            .cloned()
                            .unwrap_or(Value::Null);
                        let previous = crate::formats::event_text(&before);
                        let key = key_column.and_then(|key_col| {
                            existing.as_ref()?.columns.get(key_col)?.get(at).cloned()
                        });
                        pending.changes.push(Change {
                            row,
                            key,
                            col,
                            column: field.name.as_str().to_owned(),
                            value,
                            before,
                            previous,
                        });
                    }
                    line.push((text.clone(), None));
                }
                None => {
                    pending.unreadable += 1;
                    let reason = if text.trim().is_empty() {
                        texts.cell_required(field.name.as_str())
                    } else {
                        texts.paste_unreadable(text, &type_name(&texts, field.data_type))
                    };
                    line.push((text.clone(), Some(reason)));
                }
            }
        }
        pending.cut_off += cut;
        if is_new {
            // A required column the block did not reach is missing too.
            for (col, field) in schema.fields().iter().enumerate() {
                if values[col] == Value::Null && !field.nullable {
                    let reached = col >= anchor.col && col < anchor.col + cells.len();
                    if !reached {
                        pending.unreadable += 1;
                    }
                }
            }
            pending.added.push(values);
        }
        shown.push((is_new, line));
    }

    show_preview(host, &runtime, &pending, &schema, anchor.col, &shown);
    runtime.borrow_mut().paste = Some(pending);
}

/// The sentence the live region says, and the preview's caption.
fn summary(host: &HtmlElement, pending: &Pending) -> String {
    let texts = texts(host);
    let rows = pending
        .changes
        .iter()
        .map(|change| change.row)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        + pending.added.len();
    let mut parts = vec![texts.rows(rows as u64)];
    if pending.unreadable > 0 {
        parts.push(texts.paste_bad(pending.unreadable));
    }
    if pending.cut_off > 0 {
        parts.push(texts.paste_cut_off(pending.cut_off));
    }
    parts.join(", ")
}

/// Draws the preview above the rows and moves the focus into it.
fn show_preview(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    pending: &Pending,
    schema: &opengrid_types::Schema,
    first_col: usize,
    shown: &[(bool, Vec<(String, Option<String>)>)],
) {
    let (Some(root), Some(document)) = (host.shadow_root(), host.owner_document()) else {
        return;
    };
    close_preview(&root);
    let Ok(Some(viewport)) = root.query_selector("[part=\"viewport\"]") else {
        return;
    };
    let Some(layout) = viewport.parent_element() else {
        return;
    };
    let texts = texts(host);
    let make = |tag: &str| document.create_element(tag).ok();
    let (Some(panel), Some(caption), Some(scroller), Some(table)) =
        (make("div"), make("p"), make("div"), make("table"))
    else {
        return;
    };
    let _ = panel.set_attribute("part", grid::PASTE_PARTS[0]);
    let _ = panel.set_attribute("role", "region");
    let _ = panel.set_attribute("aria-label", &texts.paste_preview);
    let _ = panel.set_attribute("tabindex", "-1");
    if !texts.lang.trim().is_empty() {
        let _ = panel.set_attribute("lang", &texts.lang);
    }
    let sentence = summary(host, pending);
    let _ = caption.set_attribute("part", grid::PASTE_PARTS[1]);
    caption.set_text_content(Some(&sentence));
    let _ = panel.append_child(&caption);

    // The block scrolls when it is long, so the keys can reach it.
    let _ = scroller.set_attribute("data-paste-scroll", "");
    let _ = scroller.set_attribute("tabindex", "0");
    let _ = scroller.set_attribute("role", "group");
    let _ = scroller.set_attribute("aria-label", &texts.paste_preview);
    let _ = table.set_attribute("aria-label", &texts.paste_preview);
    // The columns the block reaches, by name.
    let width = shown.iter().map(|(_, line)| line.len()).max().unwrap_or(0);
    if let Some(head) = make("thead")
        && let Some(row) = make("tr")
    {
        for offset in 0..width {
            if let Some(th) = make("th") {
                let _ = th.set_attribute("scope", "col");
                let name = schema
                    .fields()
                    .get(first_col + offset)
                    .map_or("", |field| field.name.as_str());
                th.set_text_content(Some(texts.column(name)));
                let _ = row.append_child(&th);
            }
        }
        let _ = head.append_child(&row);
        let _ = table.append_child(&head);
    }
    if let Some(body) = make("tbody") {
        // A long block is shown in part; the sentence above counts all of it.
        for (is_new, line) in shown.iter().take(200) {
            let Some(row) = make("tr") else { continue };
            if *is_new {
                let _ = row.set_attribute("data-new", "");
            }
            for (text, problem) in line {
                let Some(td) = make("td") else { continue };
                td.set_text_content(Some(text));
                if let Some(reason) = problem {
                    let _ = td.set_attribute("aria-invalid", "true");
                    if let Some(note) = make("span") {
                        let _ = note.set_attribute("data-reason", "");
                        note.set_text_content(Some(&format!(" — {reason}")));
                        let _ = td.append_child(&note);
                    }
                }
                let _ = row.append_child(&td);
            }
            let _ = body.append_child(&row);
        }
        let _ = table.append_child(&body);
    }
    let _ = scroller.append_child(&table);
    let _ = panel.append_child(&scroller);

    if let Some(buttons) = make("div") {
        for (part, label) in [
            (grid::PASTE_PARTS[2], &texts.paste_apply),
            (grid::PASTE_PARTS[3], &texts.paste_discard),
        ] {
            if let Some(button) = make("button") {
                let _ = button.set_attribute("type", "button");
                let _ = button.set_attribute("part", part);
                button.set_text_content(Some(label));
                let _ = buttons.append_child(&button);
            }
        }
        let _ = panel.append_child(&buttons);
    }
    let _ = layout.insert_before(&panel, Some(&viewport));

    // One live region (point 41): the sentence rides with the status line.
    runtime.borrow_mut().state.set_notice(sentence);
    render(host, false);
    if let Ok(panel) = panel.dyn_into::<HtmlElement>() {
        let _ = panel.focus();
    }
}

/// Removes the preview, if there is one.
fn close_preview(root: &ShadowRoot) {
    if let Ok(Some(panel)) = root.query_selector("[part=\"paste-preview\"]") {
        panel.remove();
    }
}

/// Whether a key or click target is inside the preview.
pub(super) fn in_preview(target: &Element) -> bool {
    target
        .closest("[part=\"paste-preview\"]")
        .ok()
        .flatten()
        .is_some()
}

/// `Enter` applies and `Escape` drops, from anywhere in the preview; a button
/// does what a button does.
pub(super) fn on_preview_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    match event.key().as_str() {
        "Enter" if target.tag_name().eq_ignore_ascii_case("button") => {}
        "Enter" => {
            event.prevent_default();
            apply(host);
        }
        "Escape" => {
            event.prevent_default();
            discard(host);
        }
        _ => {}
    }
}

/// A click on one of the preview's buttons.
pub(super) fn on_preview_click(host: &HtmlElement, target: &Element) -> bool {
    if target
        .closest("[part=\"paste-apply\"]")
        .ok()
        .flatten()
        .is_some()
    {
        apply(host);
        return true;
    }
    if target
        .closest("[part=\"paste-discard\"]")
        .ok()
        .flatten()
        .is_some()
    {
        discard(host);
        return true;
    }
    false
}

/// Drops the preview and puts the focus back on the cell it started from.
pub(super) fn discard(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if runtime.borrow_mut().paste.take().is_none() {
        return;
    }
    if let Some(root) = host.shadow_root() {
        close_preview(&root);
    }
    runtime
        .borrow_mut()
        .state
        .set_notice(texts(host).paste_discarded.clone());
    render(host, true);
}

/// Applies the previewed paste: the cells of existing rows change as an edit
/// would — marked unsaved — and the rows past the end are added; the page hears
/// of all of it in one event.
fn apply(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    {
        let borrowed = runtime.borrow();
        let Some(pending) = borrowed.paste.as_ref() else {
            return;
        };
        if pending.unreadable > 0 {
            let message = texts(host).paste_blocked(&texts(host).paste_bad(pending.unreadable));
            drop(borrowed);
            runtime.borrow_mut().state.set_notice(message);
            render(host, false);
            return;
        }
    }
    let Some(pending) = runtime.borrow_mut().paste.take() else {
        return;
    };
    if let Some(root) = host.shadow_root() {
        close_preview(&root);
    }

    let changes = js_sys::Array::new();
    for change in &pending.changes {
        runtime
            .borrow_mut()
            .state
            .set_cell(CellRef::new(change.row, change.col), change.value.clone());
        let item = js_sys::Object::new();
        if let Some(key) = &change.key {
            let _ = js_sys::Reflect::set(
                &item,
                &JsValue::from_str("key"),
                &to_js(&opengrid_json::ToJson::to_json(key)),
            );
        }
        for (name, value) in [
            ("row", JsValue::from_f64(change.row as f64)),
            ("column", JsValue::from_str(&change.column)),
            (
                "value",
                JsValue::from_str(&crate::formats::event_text(&change.value)),
            ),
            ("previous", JsValue::from_str(&change.previous)),
        ] {
            let _ = js_sys::Reflect::set(&item, &JsValue::from_str(name), &value);
        }
        changes.push(&item);
    }
    let added = js_sys::Array::new();
    let mut entries: Vec<opengrid_grid::AddedEntry> = Vec::new();
    let names: Vec<String> = runtime
        .borrow()
        .state
        .schema()
        .fields()
        .iter()
        .map(|field| field.name.as_str().to_owned())
        .collect();
    for values in &pending.added {
        let Some(row) = runtime
            .borrow_mut()
            .state
            .added_mut()
            .push(values.clone())
            .cloned()
        else {
            continue;
        };
        let by_name = js_sys::Object::new();
        for (name, value) in names.iter().zip(&row.values) {
            let _ = js_sys::Reflect::set(
                &by_name,
                &JsValue::from_str(name),
                &to_js(&opengrid_json::ToJson::to_json(value)),
            );
        }
        let item = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&item, &JsValue::from_str("values"), &by_name);
        let _ = js_sys::Reflect::set(
            &item,
            &JsValue::from_str("provisionalKey"),
            &JsValue::from_str(&row.provisional),
        );
        added.push(&item);
        entries.push(opengrid_grid::AddedEntry {
            provisional: row.provisional.clone(),
            values: row.values.clone(),
        });
    }

    dispatch(host, &changes, &added, None);
    // One step: its cells and its rows go back together (issue #186).
    let edits = pending
        .changes
        .iter()
        .map(|change| opengrid_grid::CellEdit {
            row: change.row,
            col: change.col,
            key: change.key.clone(),
            before: change.before.clone(),
            after: change.value.clone(),
        })
        .collect();
    runtime
        .borrow_mut()
        .state
        .history_mut()
        .record(opengrid_grid::Step::Paste {
            edits,
            added: entries,
        });

    let cells = pending.changes.len()
        + pending
            .added
            .iter()
            .map(|row| row.iter().filter(|value| **value != Value::Null).count())
            .sum::<usize>();
    runtime
        .borrow_mut()
        .state
        .set_notice(texts(host).paste_applied(cells));
    render(host, true);
    // The focused row is the one the renderer leaves alone: the cells it holds
    // are redrawn where they stand.
    for change in &pending.changes {
        redraw_cell(host, CellRef::new(change.row, change.col));
    }
}

/// Fires `opengrid-paste` — `changes` and `added` as the event gives them. A paste
/// that is undone or redone says so with `undo` or `redo` (issue #186).
pub(super) fn dispatch(
    host: &HtmlElement,
    changes: &js_sys::Array,
    added: &js_sys::Array,
    step: Option<&str>,
) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("changes"), changes);
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("added"), added);
    if let Some(step) = step {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(step), &JsValue::TRUE);
    }
    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(false);
    init.set_detail(&detail);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(PASTE_EVENT, &init) {
        let _ = host.dispatch_event(&event);
    }
}
