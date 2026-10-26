//! The type-ahead combobox that edits a column with choices (issue #185).
//!
//! A `<select>` cannot filter a long list, and a list column holds several
//! picks, so the editor of a column the page gave `set_choices` is the ARIA
//! combobox with a list that filters while typing: one pick for a plain column,
//! chips for a list column (`Backspace` takes the last one back).
//!
//! The editor is a group — `part="editor"`, as every editor — holding the
//! chips, the text field (`role="combobox"`) and the listbox it controls. Focus
//! stays in the text field, and `aria-activedescendant` names the option the
//! arrow keys are on. What it holds is in the DOM: `data-value` for one pick,
//! the chips for several; [`text`] reads it back as the text a commit parses.

use super::*;

use crate::texts::GridTexts;
use opengrid_grid::{Finding, Level};

/// One choice: what is stored, and what is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    pub value: String,
    pub label: String,
}

const LISTBOX_ID: &str = "og-listbox";

/// The choices the page gave column `col`: strings, numbers or
/// `{ value, label }` pairs.
pub(super) fn choices_of(host: &HtmlElement, col: usize) -> Option<Vec<Choice>> {
    let name = columns_of(host).get(col)?.clone();
    let all = js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(CHOICES_KEY)).ok()?;
    let list = js_sys::Reflect::get(&all, &JsValue::from_str(&name)).ok()?;
    let array = list.dyn_into::<js_sys::Array>().ok()?;
    let text = |item: &JsValue| {
        item.as_string()
            .or_else(|| item.as_f64().map(|number| number.to_string()))
    };
    let options: Vec<Choice> = array
        .iter()
        .filter_map(|item| {
            if let Some(value) = text(&item) {
                return Some(Choice {
                    label: value.clone(),
                    value,
                });
            }
            let value = text(&js_sys::Reflect::get(&item, &JsValue::from_str("value")).ok()?)?;
            let label = js_sys::Reflect::get(&item, &JsValue::from_str("label"))
                .ok()
                .and_then(|label| label.as_string())
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| value.clone());
            Some(Choice { value, label })
        })
        .collect();
    (!options.is_empty()).then_some(options)
}

/// Whether this element is the editor group of a combobox.
pub(super) fn is_combo(editor: &Element) -> bool {
    editor.has_attribute("data-combo")
}

/// The combobox editor a target sits in, if any.
fn editor_of(target: &Element) -> Option<Element> {
    target
        .closest("[part=\"editor\"][data-combo]")
        .ok()
        .flatten()
}

fn is_list(editor: &Element) -> bool {
    editor.get_attribute("data-combo").as_deref() == Some("list")
}

fn input_of(editor: &Element) -> Option<HtmlInputElement> {
    editor
        .query_selector("input")
        .ok()
        .flatten()
        .and_then(|input| input.dyn_into::<HtmlInputElement>().ok())
}

/// The values a list column holds, as the JSON array text says them.
fn picks_of(current: &str) -> Vec<String> {
    let Ok(opengrid_json::Json::Array(items)) = opengrid_json::Json::parse(current) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            item.as_str()
                .map(str::to_owned)
                .or_else(|| item.as_i64().map(|number| number.to_string()))
        })
        .collect()
}

/// Builds the editor of column `col`. `current` is the value as text: the
/// choice's value for one pick, the JSON array for a list (empty for NULL).
pub(super) fn build(
    host: &HtmlElement,
    document: &web_sys::Document,
    col: usize,
    name: &str,
    data_type: DataType,
    current: &str,
    typed: Option<&str>,
) -> Option<Element> {
    let choices = choices_of(host, col)?;
    let list = data_type.is_list();
    let texts = texts(host);
    let make = |tag: &str| document.create_element(tag).ok();
    let group = make("div")?;
    let _ = group.set_attribute("part", "editor");
    let _ = group.set_attribute("data-combo", if list { "list" } else { "single" });
    let _ = group.set_attribute("role", "group");
    let _ = group.set_attribute("aria-label", name);
    if data_type == DataType::ListInt64 {
        // The picks are numbers: the commit writes them as such.
        let _ = group.set_attribute("data-numbers", "");
    }

    let label_of = |value: &str| {
        choices
            .iter()
            .find(|choice| choice.value == value)
            .map_or_else(|| value.to_owned(), |choice| choice.label.clone())
    };

    let input = make("input")?;
    for (attribute, value) in [
        ("type", "text"),
        ("role", "combobox"),
        ("aria-autocomplete", "list"),
        ("aria-expanded", "false"),
        ("aria-controls", LISTBOX_ID),
        ("aria-label", name),
        ("autocomplete", "off"),
    ] {
        let _ = input.set_attribute(attribute, value);
    }
    if list {
        let chips = make("span")?;
        let _ = chips.set_attribute("part", grid::LIST_PARTS[0]);
        let _ = chips.set_attribute("role", "list");
        let _ = chips.set_attribute("aria-label", name);
        let _ = group.append_child(&chips);
        if current.is_empty() {
            let _ = group.set_attribute("data-was-null", "");
        }
        for value in picks_of(current) {
            add_chip(&texts, document, &group, &value, &label_of(&value));
        }
    } else if !current.is_empty() {
        let _ = group.set_attribute("data-value", current);
        if let Ok(field) = input.clone().dyn_into::<HtmlInputElement>() {
            field.set_value(&label_of(current));
        }
    }
    if let Some(typed) = typed
        && let Ok(field) = input.clone().dyn_into::<HtmlInputElement>()
    {
        field.set_value(typed);
    }
    let _ = group.append_child(&input);

    let listbox = make("ul")?;
    let _ = listbox.set_attribute("part", grid::LIST_PARTS[3]);
    let _ = listbox.set_attribute("role", "listbox");
    let _ = listbox.set_attribute("id", LISTBOX_ID);
    let _ = listbox.set_attribute("aria-label", name);
    let _ = listbox.set_attribute("hidden", "");
    for (index, choice) in choices.iter().enumerate() {
        let option = make("li")?;
        let _ = option.set_attribute("role", "option");
        let _ = option.set_attribute("id", &format!("og-option-{index}"));
        let _ = option.set_attribute("aria-selected", "false");
        let _ = option.set_attribute("data-value", &choice.value);
        option.set_text_content(Some(&choice.label));
        let _ = listbox.append_child(&option);
    }
    let _ = group.append_child(&listbox);
    if typed.is_some() {
        filter(&group);
    }
    Some(group)
}

/// Adds a chip for `value`, shown as `label`, with its remove button.
fn add_chip(
    texts: &GridTexts,
    document: &web_sys::Document,
    group: &Element,
    value: &str,
    label: &str,
) {
    let Ok(Some(chips)) = group.query_selector("[part~=\"list\"]") else {
        return;
    };
    let (Ok(chip), Ok(remove)) = (
        document.create_element("span"),
        document.create_element("button"),
    ) else {
        return;
    };
    let _ = chip.set_attribute("part", grid::LIST_PARTS[1]);
    let _ = chip.set_attribute("role", "listitem");
    let _ = chip.set_attribute("data-value", value);
    chip.set_text_content(Some(label));
    for (attribute, text) in [
        ("type", "button"),
        ("part", grid::LIST_PARTS[2]),
        ("tabindex", "-1"),
    ] {
        let _ = remove.set_attribute(attribute, text);
    }
    let _ = remove.set_attribute("aria-label", &texts.list_item_remove(label));
    remove.set_text_content(Some("\u{00D7}"));
    let _ = chip.append_child(&remove);
    let _ = chips.append_child(&chip);
}

/// The options, in order.
fn options(editor: &Element) -> Vec<Element> {
    let Ok(found) = editor.query_selector_all("[role=\"option\"]") else {
        return Vec::new();
    };
    (0..found.length())
        .filter_map(|index| found.item(index))
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .collect()
}

/// The values already picked in a list.
fn picked(editor: &Element) -> Vec<String> {
    let Ok(found) = editor.query_selector_all("[part~=\"list-item\"]") else {
        return Vec::new();
    };
    (0..found.length())
        .filter_map(|index| found.item(index))
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .filter_map(|chip| chip.get_attribute("data-value"))
        .collect()
}

/// Shows the options that fit what is typed — the label contains it, case
/// ignored — and are not picked yet; opens the list while any does.
fn filter(editor: &Element) -> usize {
    let typed = input_of(editor)
        .map(|input| input.value().to_lowercase())
        .unwrap_or_default();
    let taken = if is_list(editor) {
        picked(editor)
    } else {
        Vec::new()
    };
    let mut shown = 0;
    for option in options(editor) {
        let fits = option
            .text_content()
            .unwrap_or_default()
            .to_lowercase()
            .contains(&typed)
            && option
                .get_attribute("data-value")
                .is_none_or(|value| !taken.contains(&value));
        let _ = option.set_attribute("aria-selected", "false");
        if fits {
            let _ = option.remove_attribute("hidden");
            shown += 1;
        } else {
            let _ = option.set_attribute("hidden", "");
        }
    }
    set_open(editor, shown > 0);
    if let Some(input) = input_of(editor) {
        let _ = input.remove_attribute("aria-activedescendant");
    }
    shown
}

fn set_open(editor: &Element, open: bool) {
    if let Ok(Some(listbox)) = editor.query_selector("[role=\"listbox\"]") {
        let _ = if open {
            listbox.remove_attribute("hidden")
        } else {
            listbox.set_attribute("hidden", "")
        };
    }
    if let Some(input) = input_of(editor) {
        let _ = input.set_attribute("aria-expanded", if open { "true" } else { "false" });
    }
    // The list hangs under the cell and overlaps the rows below, as a
    // validation sentence does: the cell lets it out.
    if let Some(cell) = editor.parent_element() {
        let _ = if open {
            cell.set_attribute("data-validating", "")
        } else {
            cell.remove_attribute("data-validating")
        };
    }
}

fn is_open(editor: &Element) -> bool {
    input_of(editor)
        .is_some_and(|input| input.get_attribute("aria-expanded").as_deref() == Some("true"))
}

/// The shown options, in order.
fn visible(editor: &Element) -> Vec<Element> {
    options(editor)
        .into_iter()
        .filter(|option| !option.has_attribute("hidden"))
        .collect()
}

/// The option the arrow keys are on, if any.
fn active(editor: &Element) -> Option<Element> {
    visible(editor)
        .into_iter()
        .find(|option| option.get_attribute("aria-selected").as_deref() == Some("true"))
}

/// Moves the active option by `step` among the shown ones, wrapping.
fn move_active(editor: &Element, step: isize) {
    let shown = visible(editor);
    if shown.is_empty() {
        return;
    }
    let at = shown
        .iter()
        .position(|option| option.get_attribute("aria-selected").as_deref() == Some("true"));
    let next = match (at, step) {
        (None, step) if step > 0 => 0,
        (None, _) => shown.len() - 1,
        (Some(at), step) => (at as isize + step).rem_euclid(shown.len() as isize) as usize,
    };
    for (index, option) in shown.iter().enumerate() {
        let selected = index == next;
        let _ = option.set_attribute("aria-selected", if selected { "true" } else { "false" });
        if selected {
            if let Some(input) = input_of(editor)
                && let Some(id) = option.get_attribute("id")
            {
                let _ = input.set_attribute("aria-activedescendant", &id);
            }
            option.scroll_into_view();
        }
    }
}

/// Picks `option`: a chip in a list, the value of the cell otherwise. Said in
/// the status line, since focus never leaves the text field.
fn pick(host: &HtmlElement, editor: &Element, option: &Element) {
    let (Some(value), Some(label)) = (option.get_attribute("data-value"), option.text_content())
    else {
        return;
    };
    let texts = texts(host);
    if is_list(editor) {
        if let Some(document) = host.owner_document() {
            add_chip(&texts, &document, editor, &value, &label);
        }
        if let Some(input) = input_of(editor) {
            input.set_value("");
        }
        // The list closes with the pick; `↓` opens what is left.
        filter(editor);
        set_open(editor, false);
        say(host, texts.list_item_added(&label));
    } else {
        let _ = editor.set_attribute("data-value", &value);
        if let Some(input) = input_of(editor) {
            input.set_value(&label);
        }
        set_open(editor, false);
    }
    validate::show(editor, None);
}

/// Said in the status line (point 41): the one live region.
fn say(host: &HtmlElement, sentence: String) {
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().state.set_notice(sentence);
        render(host, false);
    }
}

/// What the combobox holds, as the text a commit reads: the value of the one
/// pick; the JSON array of the chips, in the order they were picked.
pub(super) fn text(editor: &Element) -> String {
    if !is_list(editor) {
        // Emptied by hand — no text left — is no pick any more.
        return match input_of(editor) {
            Some(input) if input.value().trim().is_empty() => String::new(),
            _ => editor.get_attribute("data-value").unwrap_or_default(),
        };
    }
    let picks = picked(editor);
    if picks.is_empty() {
        // Nothing picked: the empty list — unless the cell was NULL, which stays.
        return if editor.has_attribute("data-was-null") {
            String::new()
        } else {
            "[]".to_owned()
        };
    }
    let numbers = picks.iter().all(|pick| pick.parse::<i64>().is_ok())
        && editor.get_attribute("data-numbers").is_some();
    let items: Vec<String> = picks
        .iter()
        .map(|pick| {
            if numbers {
                pick.clone()
            } else {
                opengrid_json::Json::from(pick.as_str()).to_string()
            }
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Shows a refusal under the editor, as a validator's error is shown.
fn refuse(host: &HtmlElement, editor: &Element, sentence: String) {
    let finding = Finding {
        level: Level::Error,
        message: sentence,
    };
    validate::show(editor, Some(&finding));
    validate::announce(host, &runtime_of(host), Some(&finding));
    if let Some(input) = input_of(editor) {
        let _ = input.focus();
    }
}

fn runtime_of(host: &HtmlElement) -> Rc<RefCell<GridRuntime>> {
    runtime(host).expect("a grid with an editor has its runtime")
}

/// The one option a typed text means: the label it equals (case ignored), or the
/// only option that is left.
fn resolve(editor: &Element) -> Option<Element> {
    let typed = input_of(editor)?.value().trim().to_lowercase();
    let shown = visible(editor);
    let label = |option: &Element| option.text_content().unwrap_or_default().to_lowercase();
    let only = |found: Vec<&Element>| (found.len() == 1).then(|| found[0].clone());
    // The label it is, else the one label it starts, else the one that is left:
    // `1` is a grade of its own beside `1+` and `1-`.
    only(
        shown
            .iter()
            .filter(|option| label(option) == typed)
            .collect(),
    )
    .or_else(|| {
        only(
            shown
                .iter()
                .filter(|option| label(option).starts_with(&typed))
                .collect(),
        )
    })
    .or_else(|| only(shown.iter().collect()))
}

/// A key in the combobox's text field. `true` when it was dealt with here; a
/// key left alone (`Enter` with nothing to pick, `Escape` with the list shut)
/// is the editor's: commit and cancel.
pub(super) fn on_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) -> bool {
    let Some(editor) = editor_of(target) else {
        return false;
    };
    if target.tag_name().eq_ignore_ascii_case("button") {
        return false;
    }
    match event.key().as_str() {
        "ArrowDown" | "ArrowUp" => {
            event.prevent_default();
            if !is_open(&editor) {
                filter(&editor);
            }
            move_active(&editor, if event.key() == "ArrowDown" { 1 } else { -1 });
            true
        }
        "Escape" if is_open(&editor) => {
            event.prevent_default();
            set_open(&editor, false);
            if let Some(input) = input_of(&editor) {
                let _ = input.remove_attribute("aria-activedescendant");
            }
            true
        }
        "Backspace" if is_list(&editor) => {
            let empty = input_of(&editor).is_some_and(|input| input.value().is_empty());
            if !empty {
                return false;
            }
            event.prevent_default();
            let Ok(Some(chip)) = editor.query_selector("[part~=\"list-item\"]:last-child") else {
                return true;
            };
            remove_chip(host, &editor, &chip);
            true
        }
        "Enter" => {
            if let Some(option) = active(&editor) {
                event.prevent_default();
                pick(host, &editor, &option);
                // One pick is the whole edit: it is committed with the pick.
                if !is_list(&editor) {
                    commit(host, &editor);
                }
                return true;
            }
            let typed = input_of(&editor)
                .map(|input| input.value())
                .unwrap_or_default();
            if typed.trim().is_empty() {
                return false;
            }
            event.prevent_default();
            if !is_open(&editor) {
                filter(&editor);
            }
            match resolve(&editor) {
                Some(option) => {
                    pick(host, &editor, &option);
                    if !is_list(&editor) {
                        commit(host, &editor);
                    }
                    true
                }
                None => {
                    refuse(host, &editor, texts(host).choice_refused(typed.trim()));
                    true
                }
            }
        }
        _ => false,
    }
}

/// Commits what the editor holds: the cell's edit — or, in the panel that sets
/// a value for the selected rows (issue #187), that.
fn commit(host: &HtmlElement, editor: &Element) {
    if editor.closest("[part=\"bulk\"]").ok().flatten().is_some() {
        bulk::apply(host);
    } else {
        end_edit(host, true);
    }
}

/// Takes a chip back.
fn remove_chip(host: &HtmlElement, editor: &Element, chip: &Element) {
    let label = chip
        .text_content()
        .unwrap_or_default()
        .trim_end_matches('\u{00D7}')
        .to_owned();
    chip.remove();
    filter(editor);
    set_open(editor, false);
    say(host, texts(host).list_item_removed(&label));
}

/// Typing in the text field filters the options, and says how many are left.
pub(super) fn on_input(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Some(editor) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        .and_then(|target| editor_of(&target))
    else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    validate::show(&editor, None);
    let shown = filter(&editor);
    say(&host, texts(&host).choices_count(shown));
}

/// A click on an option or on a chip's remove button. `true` when it was one.
pub(super) fn on_click(host: &HtmlElement, target: &Element) -> bool {
    let Some(editor) = editor_of(target) else {
        return false;
    };
    if let Ok(Some(option)) = target.closest("[role=\"option\"]") {
        pick(host, &editor, &option);
        // One pick is the whole edit.
        if !is_list(&editor) {
            commit(host, &editor);
        } else if let Some(input) = input_of(&editor) {
            let _ = input.focus();
        }
        return true;
    }
    if let Ok(Some(remove)) = target.closest("[part~=\"list-item-remove\"]") {
        if let Some(chip) = remove.parent_element() {
            remove_chip(host, &editor, &chip);
        }
        if let Some(input) = input_of(&editor) {
            let _ = input.focus();
        }
        return true;
    }
    false
}

/// Draws the lists of the shown cells as chips (issue #185).
///
/// The frame writes a cell's text — `a, b` — and this replaces it with the
/// chips, once per frame and only where the chips are not already the cell's:
/// a cell the frame did not touch (the focused row's) keeps them.
pub(super) fn draw_cells(host: &HtmlElement, root: &ShadowRoot) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let columns: Vec<usize> = {
        let borrowed = runtime.borrow();
        if borrowed.grouping.is_some() {
            return;
        }
        borrowed
            .state
            .schema()
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| field.data_type.is_list())
            .map(|(col, _)| col)
            .collect()
    };
    for col in columns {
        let Ok(cells) = root.query_selector_all(&format!("tbody td[data-col=\"{col}\"][data-row]"))
        else {
            continue;
        };
        for index in 0..cells.length() {
            let Some(cell) = cells
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(row) = cell
                .get_attribute("data-row")
                .and_then(|row| row.parse::<u64>().ok())
            else {
                continue;
            };
            draw_cell(host, &cell, CellRef::new(row, col));
        }
    }
}

/// One cell's chips: nothing for NULL, nothing while the cell is being edited.
pub(super) fn draw_cell(host: &HtmlElement, cell: &Element, at: CellRef) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let items: Vec<String> = {
        let borrowed = runtime.borrow();
        if borrowed.state.editing() == Some(at) {
            return;
        }
        match borrowed.state.cell(at) {
            Some(Value::List(items)) => items.iter().map(crate::formats::plain_text).collect(),
            _ => return,
        }
    };
    let signature = items.join("\u{1f}");
    if let Ok(Some(drawn)) = cell.query_selector("[part~=\"list\"]")
        && drawn.get_attribute("data-signature").as_deref() == Some(signature.as_str())
    {
        return;
    }
    let Some(document) = host.owner_document() else {
        return;
    };
    let choices = choices_of(host, at.col).unwrap_or_default();
    let Ok(list) = document.create_element("span") else {
        return;
    };
    let _ = list.set_attribute("part", grid::LIST_PARTS[0]);
    let _ = list.set_attribute("role", "list");
    let _ = list.set_attribute("data-signature", &signature);
    for item in &items {
        let Ok(chip) = document.create_element("span") else {
            continue;
        };
        let _ = chip.set_attribute("part", grid::LIST_PARTS[1]);
        let _ = chip.set_attribute("role", "listitem");
        let label = choices
            .iter()
            .find(|choice| &choice.value == item)
            .map_or(item.as_str(), |choice| choice.label.as_str());
        chip.set_text_content(Some(label));
        let _ = list.append_child(&chip);
    }
    cell.set_text_content(None);
    let _ = cell.append_child(&list);
}
