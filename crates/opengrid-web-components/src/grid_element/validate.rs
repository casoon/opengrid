//! Validation while typing, without blocking (issue #188).
//!
//! The page gives a function per column; it runs on every keystroke in the
//! editor and once more on commit. What it answers is shown under the editor,
//! stays on the cell afterwards (`data-validation`) and is said once, politely,
//! in the status line. A finding does not block the commit — the reader may
//! save a duplicate and fix it later — unless the column asks for it with
//! `block: true`; what the schema refuses always blocks.

use super::*;

use opengrid_grid::{Finding, Level};

/// Where `set_validators` keeps what the page supplied.
const KEY: &str = "__opengridValidators";

/// Stores the page's validators: `{ column: fn }` or `{ column: { validate: fn, block } }`.
pub(super) fn store(host: &HtmlElement, validators: &JsValue) {
    let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(KEY), validators);
}

/// One column's rule.
struct Rule {
    validate: js_sys::Function,
    block: bool,
}

fn rule_for(host: &HtmlElement, column: &str) -> Option<Rule> {
    let all = js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(KEY)).ok()?;
    let entry = js_sys::Reflect::get(&all, &JsValue::from_str(column)).ok()?;
    if let Some(function) = entry.dyn_ref::<js_sys::Function>() {
        return Some(Rule {
            validate: function.clone(),
            block: false,
        });
    }
    let validate = js_sys::Reflect::get(&entry, &JsValue::from_str("validate"))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let block = js_sys::Reflect::get(&entry, &JsValue::from_str("block"))
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Some(Rule { validate, block })
}

/// What the column's validator says about `text` typed into `cell`, and whether
/// the column blocks on an error. The validator gets the text as it stands in
/// the editor — as `opengrid-cell-change` gives it — and the record as loaded,
/// by field name.
pub(super) fn run(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    cell: CellRef,
    text: &str,
) -> Option<(Finding, bool)> {
    let (name, row) = {
        let borrowed = runtime.borrow();
        let fields = borrowed.state.schema().fields();
        let name = fields.get(cell.col)?.name.as_str().to_owned();
        let row = js_sys::Object::new();
        for (col, field) in fields.iter().enumerate() {
            if let Some(value) = borrowed.state.cell(CellRef::new(cell.row, col)) {
                let _ = js_sys::Reflect::set(
                    &row,
                    &JsValue::from_str(field.name.as_str()),
                    &to_js(&opengrid_json::ToJson::to_json(value)),
                );
            }
        }
        (name, row)
    };
    let rule = rule_for(host, &name)?;
    let answer = rule
        .validate
        .call2(&JsValue::NULL, &JsValue::from_str(text), &row)
        .ok()?;
    if answer.is_null() || answer.is_undefined() {
        return None;
    }
    let level = js_sys::Reflect::get(&answer, &JsValue::from_str("level"))
        .ok()?
        .as_string()
        .and_then(|level| Level::parse(&level))?;
    let message = js_sys::Reflect::get(&answer, &JsValue::from_str("message"))
        .ok()?
        .as_string()
        .filter(|message| !message.trim().is_empty())?;
    Some((Finding { level, message }, rule.block))
}

/// A keystroke (`input`) or a choice (`change`) in the editor of a cell.
pub(super) fn on_editor_event(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Some(editor) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        .and_then(|target| target.closest("[part=\"editor\"]").ok().flatten())
    else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let Some(cell) = runtime.borrow().state.editing() else {
        return;
    };
    let finding = run(&host, &runtime, cell, &editor_text(&editor)).map(|(finding, _)| finding);
    show(&editor, finding.as_ref());
    announce(&host, &runtime, finding.as_ref());
}

/// Writes the sentence under the editor, or takes it away.
pub(super) fn show(editor: &Element, finding: Option<&Finding>) {
    let Some(td) = editor.parent_element() else {
        return;
    };
    if let Ok(Some(old)) = td.query_selector("[part=\"validation\"]") {
        old.remove();
    }
    let _ = editor.remove_attribute("aria-describedby");
    let _ = td.remove_attribute("data-validating");
    let Some(finding) = finding else {
        let _ = editor.remove_attribute("aria-invalid");
        return;
    };
    let Some(document) = editor.owner_document() else {
        return;
    };
    let Ok(note) = document.create_element("span") else {
        return;
    };
    let _ = note.set_attribute("part", grid::VALIDATION_PART);
    let _ = note.set_attribute("id", "og-validation");
    let _ = note.set_attribute("data-level", finding.level.as_str());
    note.set_text_content(Some(&finding.message));
    let _ = td.append_child(&note);
    let _ = td.set_attribute("data-validating", "");
    let _ = editor.set_attribute("aria-describedby", "og-validation");
    if finding.level == Level::Error {
        let _ = editor.set_attribute("aria-invalid", "true");
    } else {
        let _ = editor.remove_attribute("aria-invalid");
    }
}

/// Says a finding once, politely, in the status line: not again while the same
/// words stand, and not at all when there is nothing to say.
pub(super) fn announce(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    finding: Option<&Finding>,
) {
    let said = finding.map(|finding| format!("{}:{}", finding.level.as_str(), finding.message));
    if runtime.borrow().validation_said == said {
        return;
    }
    runtime.borrow_mut().validation_said = said;
    if let Some(finding) = finding {
        let sentence = texts(host).validation_said(finding.level == Level::Error, &finding.message);
        runtime.borrow_mut().state.set_notice(sentence);
        render(host, false);
    }
}
