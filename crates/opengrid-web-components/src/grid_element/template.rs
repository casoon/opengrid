//! Cell templates: a node the page builds instead of a cell's text (issue #317).
//!
//! The page gives a function per column (`set_cell_templates`) that returns a
//! DOM node. The frame still writes the cell's text, as it does for a list
//! column's chips ([`combo::draw_cells`]), and this replaces it once per frame,
//! only where the node is not already the cell's: a cell the frame did not touch
//! keeps it. Nothing is ever parsed from a string, and the source's value stays
//! what search, export, copy and the events read.
//!
//! The cell is one stop of the grid pattern: the controls inside it are out of
//! the tab order, `Enter` on the cell goes in and `Escape` comes back.

use super::*;

/// Where `set_cell_templates` keeps what the page supplied.
const KEY: &str = "__opengridCellTemplates";

/// What can take the focus inside a node.
const FOCUSABLE: &str =
    "a[href], button, input, select, textarea, summary, [tabindex], [contenteditable=\"true\"]";

/// Stores the page's templates and draws every cell again.
pub(super) fn store(host: &HtmlElement, templates: &JsValue) {
    let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(KEY), templates);
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().templates_round += 1;
        render(host, false);
    }
}

fn function_for(host: &HtmlElement, column: &str) -> Option<js_sys::Function> {
    let all = js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(KEY)).ok()?;
    js_sys::Reflect::get(&all, &JsValue::from_str(column))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()
}

fn has_any(host: &HtmlElement) -> bool {
    js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(KEY))
        .ok()
        .is_some_and(|all| all.is_object())
}

/// Takes the controls of a drawn node out of the tab order.
fn untab(td: &Element) {
    if let Ok(controls) = td.query_selector_all(FOCUSABLE) {
        for index in 0..controls.length() {
            if let Some(control) = controls
                .item(index)
                .and_then(|n| n.dyn_into::<Element>().ok())
            {
                let _ = control.set_attribute("tabindex", "-1");
            }
        }
    }
}

/// Draws the templates of every templated column in view.
pub(super) fn draw_cells(host: &HtmlElement, root: &ShadowRoot) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if !has_any(host) && !matches!(root.query_selector("td[data-template]"), Ok(Some(_))) {
        return;
    }
    let columns: Vec<(usize, String, bool)> = {
        let borrowed = runtime.borrow();
        if borrowed.grouping.is_some() || borrowed.tree.is_some() {
            return;
        }
        borrowed
            .state
            .schema()
            .fields()
            .iter()
            .enumerate()
            .map(|(col, field)| {
                (
                    col,
                    field.name.as_str().to_owned(),
                    field.data_type.is_list(),
                )
            })
            .collect()
    };
    for (col, name, is_list) in columns {
        let function = (!is_list).then(|| function_for(host, &name)).flatten();
        let Ok(cells) = root.query_selector_all(&format!("tbody td[data-col=\"{col}\"][data-row]"))
        else {
            continue;
        };
        for index in 0..cells.length() {
            let Some(td) = cells
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(row) = td
                .get_attribute("data-row")
                .and_then(|row| row.parse::<u64>().ok())
            else {
                continue;
            };
            draw_cell(
                host,
                &runtime,
                &td,
                CellRef::new(row, col),
                function.as_ref(),
            );
        }
    }
}

/// One cell: its node, or — without a function now — its own text again.
fn draw_cell(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    td: &Element,
    at: CellRef,
    function: Option<&js_sys::Function>,
) {
    let (signature, argument, text) = {
        let borrowed = runtime.borrow();
        if borrowed.state.editing() == Some(at) {
            return;
        }
        let Some(value) = borrowed.state.cell(at) else {
            return;
        };
        let fields = borrowed.state.schema().fields();
        let formatter = formatter(host, borrowed.state.schema());
        let text = formatter.text(at.col, value);
        let mut signature = format!("{}\u{1f}{text}", borrowed.templates_round);
        let row = js_sys::Object::new();
        for (col, field) in fields.iter().enumerate() {
            if let Some(cell) = borrowed.state.cell(CellRef::new(at.row, col)) {
                signature.push('\u{1f}');
                signature.push_str(&crate::formats::plain_text(cell));
                let _ = js_sys::Reflect::set(
                    &row,
                    &JsValue::from_str(field.name.as_str()),
                    &to_js(&opengrid_json::ToJson::to_json(cell)),
                );
            }
        }
        let argument = js_sys::Object::new();
        for (name, item) in [
            ("value", to_js(&opengrid_json::ToJson::to_json(value))),
            ("text", JsValue::from_str(&text)),
            ("row", row.into()),
            (
                "key",
                row_key_js(host, &borrowed.state, at.row).unwrap_or(JsValue::UNDEFINED),
            ),
        ] {
            let _ = js_sys::Reflect::set(&argument, &JsValue::from_str(name), &item);
        }
        (signature, argument, text)
    };

    let Some(function) = function else {
        // The template was taken away: the cell says its text again.
        if td.has_attribute("data-template") {
            let _ = td.remove_attribute("data-template");
            td.set_text_content(Some(&text));
        }
        return;
    };
    // Drawn already: its node is the cell's only child element still.
    if td.get_attribute("data-template").as_deref() == Some(signature.as_str())
        && td.child_element_count() > 0
    {
        return;
    }
    let answer = function
        .call1(&JsValue::NULL, &argument)
        .unwrap_or(JsValue::NULL);
    let Some(document) = host.owner_document() else {
        return;
    };
    let node: Option<Node> = if let Some(node) = answer.dyn_ref::<Node>() {
        Some(node.clone())
    } else if let Some(string) = answer.as_string() {
        // A string is text: a wrapper keeps it a node of its own, so the cell
        // is not drawn again every frame.
        document.create_element("span").ok().map(|span| {
            span.set_text_content(Some(&string));
            span.into()
        })
    } else {
        None
    };
    let Some(node) = node else {
        if td.has_attribute("data-template") {
            let _ = td.remove_attribute("data-template");
            td.set_text_content(Some(&text));
        }
        return;
    };
    td.set_text_content(None);
    let _ = td.append_child(&node);
    untab(td);
    let _ = td.set_attribute("data-template", &signature);
}

/// The cell an element inside a drawn node belongs to, when it is not the cell.
fn cell_around(target: &Element) -> Option<Element> {
    let td = target.closest("td[data-template]").ok().flatten()?;
    (!td.is_same_node(Some(target))).then_some(td)
}

/// A key pressed inside a drawn node: `Escape` comes back to the cell, the rest
/// is the control's. `true` when the grid's matrix is to stay out of it.
pub(super) fn on_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) -> bool {
    let Some(td) = cell_around(target) else {
        return false;
    };
    if event.key() == "Escape" {
        event.prevent_default();
        untab(&td);
        if let Ok(td) = td.dyn_into::<HtmlElement>() {
            let _ = td.focus();
        } else {
            focus_active(host);
        }
    }
    true
}

/// `Enter` on a templated cell goes into its first control. `false` — nothing
/// to go into — leaves the key to what it meant before.
pub(super) fn enter(host: &HtmlElement, cell: CellRef) -> bool {
    let Some(root) = host.shadow_root() else {
        return false;
    };
    let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-template][data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) else {
        return false;
    };
    let Ok(Some(first)) = td.query_selector(FOCUSABLE) else {
        return false;
    };
    // Inside, `Tab` walks the controls as the page made them.
    if let Ok(controls) = td.query_selector_all(FOCUSABLE) {
        for index in 0..controls.length() {
            if let Some(control) = controls
                .item(index)
                .and_then(|n| n.dyn_into::<Element>().ok())
            {
                let _ = control.remove_attribute("tabindex");
            }
        }
    }
    match first.dyn_into::<HtmlElement>() {
        Ok(first) => first.focus().is_ok(),
        Err(_) => false,
    }
}

/// The focus left a templated cell for somewhere else: its controls go out of
/// the tab order again.
pub(super) fn on_focus_out(event: Event) {
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return;
    };
    let Some(td) = cell_around(&target) else {
        return;
    };
    let into = event
        .dyn_ref::<web_sys::FocusEvent>()
        .and_then(|event| event.related_target())
        .and_then(|node| node.dyn_into::<Node>().ok());
    if into.is_some_and(|node| td.contains(Some(&node))) {
        return;
    }
    untab(&td);
}
