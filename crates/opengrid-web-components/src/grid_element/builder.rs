//! The nested filter in the element, and its builder (issue #324).
//!
//! The filter row holds one condition per column, and-ed. A filter with an
//! `or` across columns lives beside it, as **one** tree in the wire form
//! ([`opengrid_grid::filter_tree`]): `GridRuntime::nested`. It has two
//! editors, and both write that tree — the search expression
//! ([`crate::search`]) and the dialog here.
//!
//! The dialog is a native modal `<dialog>`: the page behind it is inert, `Tab`
//! stays inside and `Escape` closes it, all by the platform. It is built when
//! it opens and removed when it closes. Its fields are the quick dialog's
//! (`fill_conditions`), once per condition.
//!
//! Its events bubble on to the grid's listeners, and mean nothing there: a
//! key drives the grid only inside its table, and the dialog's buttons carry
//! names of their own (`data-builder`).

use super::*;

use opengrid_grid::filter_tree;
use web_sys::HtmlDialogElement;

const DIALOG: &str = "[part=\"filter-builder-dialog\"]";

/// One condition in words: "country is DE" — what a filter's chip says.
pub(super) fn condition_words(texts: &crate::texts::GridTexts, entry: &FilterEntry) -> String {
    let token = entry.op.as_str();
    let index = crate::shared::FILTER_OPERATORS
        .iter()
        .position(|op| *op == token)
        .unwrap_or(0);
    let operator = texts.operator(index, token);
    if grid::takes_value(token) {
        format!(
            "{} {operator} {}",
            texts.column(&entry.column),
            entry.value.trim()
        )
    } else {
        format!("{} {operator}", texts.column(&entry.column))
    }
}

/// The nested filter in words: "country is DE or amount ≥ 1000".
pub(super) fn words(host: &HtmlElement, filter: &FilterExpr) -> String {
    let texts = texts(host);
    filter_tree::words(
        filter,
        &|entry| condition_words(&texts, entry),
        &texts.query_and,
        &texts.query_or,
    )
}

/// Sets the nested filter — or with `None` removes it — asks again and says
/// it once, with the result. Setting what is there changes nothing.
pub(super) fn set(host: &HtmlElement, filter: Option<FilterExpr>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if runtime.borrow().nested == filter {
        return;
    }
    let before = std::mem::replace(&mut runtime.borrow_mut().nested, filter.clone());
    apply_facets(host);
    // After the query, so the sentence rides with its result only (phase E (j)).
    let texts = texts(host);
    let said = match (&filter, &before) {
        (Some(filter), _) => texts.filter_applied(&words(host, filter)),
        (None, Some(before)) => texts.filter_removed(&words(host, before)),
        (None, None) => return,
    };
    runtime.borrow_mut().state.set_notice(said);
    if let Some(root) = host.shadow_root() {
        draw_chips(host, &root);
    }
}

/// A new element with `html` inside. The markup is ours; every word a page
/// or a reader wrote goes in as text afterwards ([`say`]).
fn element(host: &HtmlElement, tag: &str, html: &str) -> Option<Element> {
    let node = host.owner_document()?.create_element(tag).ok()?;
    node.set_inner_html(html);
    Some(node)
}

/// Puts `text` into each node `selector` finds under `scope`, in order.
fn say(scope: &Element, selector: &str, texts: &[&str]) {
    let Ok(nodes) = scope.query_selector_all(selector) else {
        return;
    };
    for (index, text) in texts.iter().enumerate() {
        if let Some(node) = nodes.item(index as u32) {
            node.set_text_content(Some(text));
        }
    }
}

fn option(select: &Element, value: &str, text: &str) {
    if let Some(option) = select
        .owner_document()
        .and_then(|document| document.create_element("option").ok())
    {
        let _ = option.set_attribute("value", value);
        option.set_text_content(Some(text));
        let _ = select.append_child(&option);
    }
}

fn select_value(scope: &Element, selector: &str, value: &str) {
    if let Ok(Some(select)) = scope.query_selector(selector)
        && let Ok(select) = select.dyn_into::<HtmlSelectElement>()
    {
        select.set_value(value);
    }
}

fn focus(scope: &Element, selector: &str) {
    if let Ok(Some(node)) = scope.query_selector(selector)
        && let Ok(node) = node.dyn_into::<HtmlElement>()
    {
        let _ = node.focus();
    }
}

/// One condition: *Column*, *Condition*, *Value* and its remove button —
/// filled from `filter` when it is one, empty on the first column otherwise.
fn condition_node(host: &HtmlElement, filter: Option<&FilterExpr>) -> Option<Element> {
    let texts = texts(host);
    let fields = shown_fields(host);
    // The columns' names are the page's words, not ours: `lang=""`.
    let row: HtmlElement = element(
        host,
        "div",
        "<label><span></span><select data-dialog=\"column\" lang=\"\"></select></label>\
         <label><span></span><select data-dialog=\"op\"></select></label>\
         <label data-dialog-value><span></span><input data-dialog=\"value\"></label>\
         <button type=\"button\" data-builder=\"remove\">\u{00D7}</button>",
    )?
    .dyn_into()
    .ok()?;
    let _ = row.set_attribute("data-cond", "");
    let _ = row.set_attribute("role", "group");
    say(
        &row,
        "span",
        &[
            &texts.filter_column_label,
            &texts.filter_condition_label,
            &texts.filter_value_label,
        ],
    );
    let column = row.query_selector("[data-dialog=\"column\"]").ok()??;
    for field in &fields {
        let name = field.name.as_str();
        option(&column, name, texts.column(name));
    }

    let entry = filter.and_then(filter_tree::entry);
    let shown = |name: &str| fields.iter().any(|field| field.name.as_str() == name);
    match entry {
        Some(entry) if shown(&entry.column) => {
            select_value(&row, "[data-dialog=\"column\"]", &entry.column);
            fill_conditions(host, &row, &entry.column);
            let token = entry.op.as_str();
            select_value(&row, "[data-dialog=\"op\"]", token);
            show_value_field(&row, token);
            if let Ok(Some(input)) = row.query_selector("[data-dialog=\"value\"]")
                && let Ok(input) = input.dyn_into::<HtmlInputElement>()
            {
                if input.type_() == "checkbox" {
                    input.set_checked(entry.value == "true");
                } else {
                    input.set_value(&entry.value);
                }
            }
        }
        // A condition on a column the grid does not show cannot be typed
        // here: it is kept as it is, to see and to remove.
        Some(entry) => {
            let token = entry.op.as_str();
            let _ = row.set_attribute(
                "data-kept",
                &filter.map(opengrid_json::to_string).unwrap_or_default(),
            );
            option(&column, &entry.column, texts.column(&entry.column));
            select_value(&row, "[data-dialog=\"column\"]", &entry.column);
            let index = crate::shared::FILTER_OPERATORS
                .iter()
                .position(|known| *known == token)
                .unwrap_or(0);
            if let Ok(Some(op)) = row.query_selector("[data-dialog=\"op\"]") {
                option(&op, token, &texts.operator(index, token));
            }
            show_value_field(&row, token);
            if let Ok(controls) = row.query_selector_all("[data-dialog]") {
                for index in 0..controls.length() {
                    if let Some(control) = controls
                        .item(index)
                        .and_then(|node| node.dyn_into::<Element>().ok())
                    {
                        let _ = control.set_attribute("disabled", "");
                        // The input's; on the two selects it says nothing.
                        let _ = control.set_attribute("value", &entry.value);
                    }
                }
            }
        }
        None => {
            if let Some(first) = fields.first() {
                fill_conditions(host, &row, first.name.as_str());
            }
        }
    }
    Some(row.into())
}

/// A group: its choice between *all* and *any*, its members and its buttons.
/// The outermost one is a plain block the dialog names; one inside is a
/// `<fieldset>` with a legend and a button that removes it.
fn group_node(host: &HtmlElement, filter: Option<&FilterExpr>, outermost: bool) -> Option<Element> {
    let texts = texts(host);
    let group = element(
        host,
        if outermost { "div" } else { "fieldset" },
        "<legend></legend>\
         <label data-join><span></span><select data-builder=\"join\">\
         <option value=\"and\"></option><option value=\"or\"></option></select></label>\
         <div data-members></div>\
         <div data-group-actions>\
         <button type=\"button\" data-builder=\"add-condition\"></button>\
         <button type=\"button\" data-builder=\"add-group\"></button>\
         <button type=\"button\" data-builder=\"remove\"></button></div>",
    )?;
    let _ = group.set_attribute("data-group", "");
    if outermost {
        // Nothing names or removes the outermost group but the dialog.
        if let Ok(extra) = group.query_selector_all("legend, [data-builder=\"remove\"]") {
            for index in 0..extra.length() {
                if let Some(node) = extra.item(index)
                    && let Some(parent) = node.parent_node()
                {
                    let _ = parent.remove_child(&node);
                }
            }
        }
    }
    say(
        &group,
        "span, option, button",
        &[
            &texts.filter_builder_join,
            &texts.filter_builder_all,
            &texts.filter_builder_any,
            &texts.filter_builder_add_condition,
            &texts.filter_builder_add_group,
        ],
    );
    let members = group.query_selector("[data-members]").ok()??;

    let (any, items): (bool, &[FilterExpr]) = match filter {
        Some(FilterExpr::Or(items)) => (true, items),
        Some(FilterExpr::And(items)) => (false, items),
        Some(single) => (false, std::slice::from_ref(single)),
        None => (false, &[]),
    };
    if any {
        select_value(&group, "[data-builder=\"join\"]", "or");
    }
    // A group nobody filled yet starts with one condition to fill.
    let mut wanted: Vec<Option<&FilterExpr>> = items.iter().map(Some).collect();
    if wanted.is_empty() {
        wanted.push(None);
    }
    for item in wanted {
        let member = match item {
            Some(FilterExpr::And(_) | FilterExpr::Or(_)) => group_node(host, item, false)?,
            _ => condition_node(host, item)?,
        };
        let _ = members.append_child(&member);
    }
    Some(group)
}

/// Names the conditions and the inner groups by their order on the screen,
/// and their remove buttons after them: ten buttons called "Remove" are ten
/// buttons nobody can tell apart.
fn number(host: &HtmlElement, dialog: &Element) {
    let texts = texts(host);
    for (selector, group) in [("[data-cond]", false), ("fieldset", true)] {
        let Ok(nodes) = dialog.query_selector_all(selector) else {
            continue;
        };
        for index in 0..nodes.length() {
            let Some(node) = nodes
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let number = index as usize + 1;
            if group {
                let name = texts.filter_builder_group(number);
                say(&node, ":scope > legend", &[&name]);
                say(
                    &node,
                    ":scope > [data-group-actions] > [data-builder=\"remove\"]",
                    &[&texts.chip_remove(&name)],
                );
            } else {
                let name = texts.filter_builder_condition(number);
                let _ = node.set_attribute("aria-label", &name);
                if let Ok(Some(remove)) = node.query_selector("[data-builder=\"remove\"]") {
                    let _ = remove.set_attribute("aria-label", &texts.chip_remove(&name));
                }
            }
        }
    }
}

/// Opens the builder on the nested filter as it stands. `from_chip` says
/// where the focus goes back to.
pub(super) fn open(host: &HtmlElement, from_chip: bool) {
    let (Some(root), Some(runtime)) = (host.shadow_root(), runtime(host)) else {
        return;
    };
    if shown_fields(host).is_empty() || matches!(root.query_selector(DIALOG), Ok(Some(_))) {
        return;
    }
    let texts = texts(host);
    let nested = runtime.borrow().nested.clone();
    // `Enter` in a value applies: the form's own submit.
    let Some(dialog) = element(
        host,
        "dialog",
        "<form method=\"dialog\" novalidate><h2 id=\"og-builder-title\"></h2>\
         <p id=\"og-builder-problem\" data-dialog-problem hidden></p>\
         <div data-dialog-actions>\
         <button type=\"button\" data-builder=\"cancel\"></button>\
         <button type=\"submit\"></button></div></form>",
    ) else {
        return;
    };
    say(
        &dialog,
        "h2, [data-dialog-actions] > button",
        &[&texts.filter_builder, &texts.cancel, &texts.apply],
    );
    for (name, value) in [
        ("part", "filter-builder-dialog"),
        ("aria-labelledby", "og-builder-title"),
        ("lang", texts.lang.trim()),
        ("data-from-chip", if from_chip { "1" } else { "" }),
    ] {
        if !value.is_empty() {
            let _ = dialog.set_attribute(name, value);
        }
    }
    if let (Some(group), Ok(Some(problem))) = (
        group_node(host, nested.as_ref(), true),
        dialog.query_selector("[data-dialog-problem]"),
    ) {
        let _ = problem.before_with_node_1(&group);
    }
    number(host, &dialog);
    let Ok(dialog) = dialog.dyn_into::<HtmlDialogElement>() else {
        return;
    };
    let owner = host.clone();
    let listener = Closure::<dyn FnMut(Event)>::new(move |event: Event| on_event(&owner, &event))
        .into_js_value();
    for kind in ["click", "change", "submit", "close"] {
        let _ = dialog.add_event_listener_with_callback(kind, listener.unchecked_ref());
    }
    let _ = root.append_child(&dialog);
    let _ = dialog.show_modal();
}

fn on_event(host: &HtmlElement, event: &Event) {
    let Some(dialog) = event
        .current_target()
        .and_then(|target| target.dyn_into::<HtmlDialogElement>().ok())
    else {
        return;
    };
    let target = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok());
    match (event.type_().as_str(), target) {
        ("submit", _) => {
            event.prevent_default();
            apply(host, &dialog);
        }
        ("close", _) => closed(host, &dialog),
        ("click", Some(target)) => on_click(host, &dialog, &target),
        ("change", Some(target)) => {
            let Ok(Some(row)) = target.closest("[data-cond]") else {
                return;
            };
            let (Ok(row), Ok(select)) = (
                row.dyn_into::<HtmlElement>(),
                target.dyn_into::<HtmlSelectElement>(),
            ) else {
                return;
            };
            match select.get_attribute("data-dialog").as_deref() {
                Some("column") => fill_conditions(host, &row, &select.value()),
                Some("op") => show_value_field(&row, &select.value()),
                _ => {}
            }
        }
        _ => {}
    }
}

fn on_click(host: &HtmlElement, dialog: &HtmlDialogElement, target: &Element) {
    let Ok(Some(button)) = target.closest("button[data-builder]") else {
        return;
    };
    let action = button.get_attribute("data-builder");
    let Ok(Some(group)) = button.closest("[data-group]") else {
        // *Cancel* stands outside the groups.
        if action.as_deref() == Some("cancel") {
            dialog.close();
        }
        return;
    };
    let members = group
        .query_selector(":scope > [data-members]")
        .ok()
        .flatten();
    match (action.as_deref(), members) {
        (Some("add-condition"), Some(members)) => {
            if let Some(row) = condition_node(host, None) {
                let _ = members.append_child(&row);
                number(host, dialog);
                focus(&row, "[data-dialog=\"column\"]");
            }
        }
        (Some("add-group"), Some(members)) => {
            if let Some(inner) = group_node(host, None, false) {
                let _ = members.append_child(&inner);
                number(host, dialog);
                focus(&inner, "[data-builder=\"join\"]");
            }
        }
        (Some("remove"), _) => {
            // A condition's button removes the condition, a group's the group;
            // the focus goes to the group that held it, never to the document.
            let member = button
                .closest("[data-cond]")
                .ok()
                .flatten()
                .unwrap_or(group);
            let Some(holder) = member
                .parent_element()
                .and_then(|members| members.closest("[data-group]").ok().flatten())
            else {
                return;
            };
            member.remove();
            number(host, dialog);
            focus(
                &holder,
                ":scope > [data-group-actions] > [data-builder=\"add-condition\"]",
            );
        }
        _ => {}
    }
}

/// What a field holds that cannot be applied: the sentence, and the field.
type Refusal = (String, Option<Element>);

/// Reads one condition. A value that is missing, or one the column cannot
/// take, is a refusal that names the column.
fn read_condition(host: &HtmlElement, row: &Element) -> Result<Option<FilterExpr>, Refusal> {
    if let Some(kept) = row.get_attribute("data-kept") {
        return Ok(opengrid_json::from_str::<FilterExpr>(&kept).ok());
    }
    let control = |name: &str| {
        row.query_selector(&format!("[data-dialog=\"{name}\"]"))
            .ok()
            .flatten()
    };
    let chosen = |name: &str| {
        control(name)
            .and_then(|node| node.dyn_into::<HtmlSelectElement>().ok())
            .map(|select| select.value())
            .unwrap_or_default()
    };
    let field = control("value");
    let value = field
        .clone()
        .and_then(|node| node.dyn_into::<HtmlInputElement>().ok())
        .map(|input| {
            // A boolean's value is a checkbox, whose `value` is `on` either
            // way (issue #60).
            if input.type_() == "checkbox" {
                input.checked().to_string()
            } else {
                input.value()
            }
        })
        .unwrap_or_default();
    let (column, token) = (chosen("column"), chosen("op"));
    let Some(op) = grid::FilterOp::parse(&token) else {
        return Ok(None);
    };
    let texts = texts(host);
    if grid::takes_value(&token) && value.trim().is_empty() {
        return Err((
            texts
                .query_missing_value
                .replace("{column}", texts.column(&column)),
            field,
        ));
    }
    let schema = runtime(host)
        .map(|runtime| runtime.borrow().state.schema().clone())
        .unwrap_or_default();
    let entry = FilterEntry { column, op, value };
    filter_tree::condition(&entry, &schema)
        .map_err(|problem| (texts.filter_invalid(&problem.column, &problem.value), field))
}

/// Reads a group, with the groups inside it, into one tidy tree.
fn read_group(host: &HtmlElement, group: &Element) -> Result<Option<FilterExpr>, Refusal> {
    let any = group
        .query_selector(":scope > [data-join] select")
        .ok()
        .flatten()
        .and_then(|select| select.dyn_into::<HtmlSelectElement>().ok())
        .is_some_and(|select| select.value() == "or");
    let mut members = Vec::new();
    if let Ok(nodes) = group.query_selector_all(":scope > [data-members] > *") {
        for index in 0..nodes.length() {
            let Some(node) = nodes
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let member = if node.has_attribute("data-cond") {
                read_condition(host, &node)?
            } else {
                read_group(host, &node)?
            };
            members.extend(member);
        }
    }
    Ok(filter_tree::group(any, members))
}

/// *Apply*: the tree the dialog shows becomes the nested filter — or, with
/// no condition left, there is none — and the dialog closes. What cannot be
/// applied is named above the buttons and its field takes the focus, marked
/// invalid and described by the sentence: read once, with the field.
fn apply(host: &HtmlElement, dialog: &HtmlDialogElement) {
    let Ok(Some(group)) = dialog.query_selector("[data-group]") else {
        return;
    };
    if let Ok(marked) = dialog.query_selector_all("[aria-invalid]") {
        for index in 0..marked.length() {
            if let Some(node) = marked
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                let _ = node.remove_attribute("aria-invalid");
                let _ = node.remove_attribute("aria-describedby");
            }
        }
    }
    match read_group(host, &group) {
        Ok(filter) => {
            set(host, filter);
            dialog.close();
        }
        Err((message, field)) => {
            if let Ok(Some(line)) = dialog.query_selector("[data-dialog-problem]") {
                line.set_text_content(Some(&message));
                let _ = line.remove_attribute("hidden");
            }
            if let Some(field) = field {
                let _ = field.set_attribute("aria-invalid", "true");
                let _ = field.set_attribute("aria-describedby", "og-builder-problem");
                if let Ok(field) = field.dyn_into::<HtmlElement>() {
                    let _ = field.focus();
                }
            }
        }
    }
}

/// The dialog closed — applied, cancelled or by `Escape`: it leaves the
/// tree, and the focus goes back to where the reader came from, or to the
/// toolbar's button when the chip is gone.
fn closed(host: &HtmlElement, dialog: &HtmlDialogElement) {
    let from_chip = dialog.has_attribute("data-from-chip");
    dialog.remove();
    let Some(root) = host.shadow_root() else {
        return;
    };
    let chip = root.query_selector("[data-chip-edit]").ok().flatten();
    let back = chip
        .filter(|_| from_chip)
        .or_else(|| quick_button(&root, "filter-builder").map(Element::from));
    if let Some(back) = back.and_then(|node| node.dyn_into::<HtmlElement>().ok()) {
        let _ = back.focus();
    }
}
