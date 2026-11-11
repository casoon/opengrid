//! The quick doors: "+ Filter" and "+ Group", the filter dialog and the operator
//! menu — second doors to what the column menu and the filter row already do.

use super::*;

//
// Two second doors, like the column menu: a filter lands in the filter row's
// own fields and is applied by its code, a grouping level in `group-by`. The
// keyboard protocol is the one written into issue #34 before building.

/// The toolbar button a popup belongs to.
pub(super) fn quick_button(root: &ShadowRoot, key: &str) -> Option<HtmlElement> {
    root.query_selector(&format!("[data-toolbar=\"{key}\"]"))
        .ok()
        .flatten()
        .and_then(|button| button.dyn_into::<HtmlElement>().ok())
}

/// Shows a popup built for `key`'s button, and keeps `aria-expanded` true to
/// what is open — also when the browser closes it (a click outside).
fn show_quick(root: &ShadowRoot, popup: &HtmlElement, key: &str) {
    let _ = root.append_child(popup);
    let _ = popup.show_popover();
    let Some(button) = quick_button(root, key) else {
        return;
    };
    let _ = button.set_attribute("aria-expanded", "true");
    place_under(popup, &button);
    let key = key.to_owned();
    let closed = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let open = js_sys::Reflect::get(&event, &JsValue::from_str("newState"))
            .ok()
            .and_then(|state| state.as_string())
            .is_some_and(|state| state == "open");
        if !open {
            if let Some(root) = root_of_target(&event)
                && let Some(button) = quick_button(&root, &key)
            {
                let _ = button.set_attribute("aria-expanded", "false");
            }
            if let Some(popup) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            {
                popup.remove();
            }
        }
    })
    .into_js_value();
    let _ = popup.add_event_listener_with_callback("toggle", closed.unchecked_ref());
}

/// Closes an open quick popup; with `refocus`, the focus goes back to its button.
pub(super) fn close_quick(host: &HtmlElement, part: &str, key: &str, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    if let Ok(Some(popup)) = root.query_selector(&format!("[part=\"{part}\"]"))
        && let Ok(popup) = popup.dyn_into::<HtmlElement>()
    {
        let _ = popup.hide_popover();
        popup.remove();
    }
    if let Some(button) = quick_button(&root, key) {
        let _ = button.set_attribute("aria-expanded", "false");
        if refocus {
            let _ = button.focus();
        }
    }
}

/// The shown columns with their types, in order.
pub(super) fn shown_fields(host: &HtmlElement) -> Vec<opengrid_types::Field> {
    runtime(host)
        .map(|runtime| runtime.borrow().state.schema().fields().to_vec())
        .unwrap_or_default()
}

/// Opens *Add filter*: a non-modal dialog with a column, a condition and a value.
pub(super) fn open_filter_dialog(host: &HtmlElement) {
    let (Some(root), Some(document)) = (
        host.shadow_root(),
        web_sys::window().and_then(|window| window.document()),
    ) else {
        return;
    };
    close_quick(host, "filter-dialog", "add-filter", false);
    let fields = shown_fields(host);
    let Some(first) = fields.first() else {
        return;
    };
    let texts = texts(host);
    let Ok(dialog) = document.create_element("div") else {
        return;
    };
    for (name, value) in [
        ("part", "filter-dialog"),
        ("role", "dialog"),
        ("popover", "auto"),
        // Tab does not leave it (`on_dialog_key`): it says so.
        ("aria-modal", "true"),
        ("aria-labelledby", "og-filter-dialog-title"),
    ] {
        let _ = dialog.set_attribute(name, value);
    }
    if !texts.lang.trim().is_empty() {
        let _ = dialog.set_attribute("lang", &texts.lang);
    }
    dialog.set_inner_html(
        "<div data-dialog-title id=\"og-filter-dialog-title\"></div>\
         <label><span></span><select data-dialog=\"column\"></select></label>\
         <label><span></span><select data-dialog=\"op\"></select></label>\
         <label data-dialog-value><span></span><input data-dialog=\"value\"></label>\
         <p data-dialog-problem hidden></p>\
         <div data-dialog-actions>\
           <button type=\"button\" data-dialog-action=\"cancel\"></button>\
           <button type=\"button\" data-dialog-action=\"apply\"></button>\
         </div>",
    );
    let set = |selector: &str, text: &str| {
        if let Ok(Some(node)) = dialog.query_selector(selector) {
            node.set_text_content(Some(text));
        }
    };
    set("#og-filter-dialog-title", &texts.add_filter_title);
    set("label:nth-of-type(1) > span", &texts.filter_column_label);
    set("label:nth-of-type(2) > span", &texts.filter_condition_label);
    set("label:nth-of-type(3) > span", &texts.filter_value_label);
    set("[data-dialog-action=\"cancel\"]", &texts.cancel);
    set("[data-dialog-action=\"apply\"]", &texts.apply);
    if let Ok(Some(select)) = dialog.query_selector("[data-dialog=\"column\"]") {
        for field in &fields {
            if let Ok(option) = document.create_element("option") {
                let name = field.name.as_str();
                let _ = option.set_attribute("value", name);
                option.set_text_content(Some(texts.column(name)));
                // The column's name is the page's word, not ours.
                let _ = option.set_attribute("lang", "");
                let _ = select.append_child(&option);
            }
        }
    }
    let Ok(dialog) = dialog.dyn_into::<HtmlElement>() else {
        return;
    };
    fill_conditions(host, &dialog, first.name.as_str());
    show_quick(&root, &dialog, "add-filter");
    if let Ok(Some(column)) = dialog.query_selector("[data-dialog=\"column\"]")
        && let Ok(column) = column.dyn_into::<HtmlElement>()
    {
        let _ = column.focus();
    }
    let changed = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<HtmlSelectElement>().ok())
        else {
            return;
        };
        let Some(dialog) = target
            .closest("[part=\"filter-dialog\"]")
            .ok()
            .flatten()
            .and_then(|dialog| dialog.dyn_into::<HtmlElement>().ok())
        else {
            return;
        };
        match target.get_attribute("data-dialog").as_deref() {
            Some("column") => {
                if let Some(root) = root_of_target(&event)
                    && let Ok(owner) = root.host().dyn_into::<HtmlElement>()
                {
                    fill_conditions(&owner, &dialog, &target.value());
                }
            }
            Some("op") => show_value_field(&dialog, &target.value()),
            _ => {}
        }
    })
    .into_js_value();
    let _ = dialog.add_event_listener_with_callback("change", changed.unchecked_ref());
}

/// The conditions `column`'s type allows — the filter row's own list — and the
/// value field's type.
pub(super) fn fill_conditions(host: &HtmlElement, dialog: &HtmlElement, column: &str) {
    let Some(field) = shown_fields(host)
        .into_iter()
        .find(|field| field.name.as_str() == column)
    else {
        return;
    };
    let texts = texts(host);
    let Ok(Some(select)) = dialog.query_selector("[data-dialog=\"op\"]") else {
        return;
    };
    select.set_inner_html("");
    let document = select.owner_document();
    for op in grid::operators_for(field.data_type, field.nullable) {
        let index = crate::shared::FILTER_OPERATORS
            .iter()
            .position(|token| *token == op)
            .unwrap_or(0);
        if let Some(option) = document
            .as_ref()
            .and_then(|document| document.create_element("option").ok())
        {
            let _ = option.set_attribute("value", op);
            option.set_text_content(Some(&texts.operator(index, op)));
            let _ = select.append_child(&option);
        }
    }
    if let Ok(Some(input)) = dialog.query_selector("[data-dialog=\"value\"]")
        && let Ok(input) = input.dyn_into::<HtmlInputElement>()
    {
        input.set_type(grid::input_type(field.data_type));
        match grid::input_step(field.data_type) {
            Some(step) => {
                let _ = input.set_attribute("step", &step);
            }
            None => {
                let _ = input.remove_attribute("step");
            }
        }
        input.set_value("");
    }
    let first = select
        .dyn_ref::<HtmlSelectElement>()
        .map(|select| select.value())
        .unwrap_or_default();
    show_value_field(dialog, &first);
}

/// Hides the value field for the two conditions that take none.
pub(super) fn show_value_field(dialog: &HtmlElement, op: &str) {
    if let Ok(Some(label)) = dialog.query_selector("[data-dialog-value]") {
        if grid::takes_value(op) {
            let _ = label.remove_attribute("hidden");
        } else {
            let _ = label.set_attribute("hidden", "");
        }
    }
}

/// *Apply*: the filter row's entry for the column, applied by its own code.
/// A value the column cannot take is named in the dialog, which stays open.
pub(super) fn apply_filter_dialog(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(Some(dialog)) = root.query_selector("[part=\"filter-dialog\"]") else {
        return;
    };
    let read = |selector: &str| -> String {
        dialog
            .query_selector(selector)
            .ok()
            .flatten()
            .and_then(|node| {
                node.clone()
                    .dyn_into::<HtmlSelectElement>()
                    .map(|select| select.value())
                    .or_else(|node| {
                        // A boolean's value in the dialog is a checkbox, whose
                        // `value` is `on` either way (issue #60).
                        node.dyn_into::<HtmlInputElement>().map(|input| {
                            if input.type_() == "checkbox" {
                                input.checked().to_string()
                            } else {
                                input.value()
                            }
                        })
                    })
                    .ok()
            })
            .unwrap_or_default()
    };
    let (column, op, value) = (
        read("[data-dialog=\"column\"]"),
        read("[data-dialog=\"op\"]"),
        read("[data-dialog=\"value\"]"),
    );
    let Some(filter_op) = grid::FilterOp::parse(&op) else {
        return;
    };
    let texts = texts(host);
    let problem = |message: &str| {
        if let Ok(Some(line)) = dialog.query_selector("[data-dialog-problem]") {
            line.set_text_content(Some(message));
            let _ = line.remove_attribute("hidden");
        }
        // Said through the status line, the grid's one live region.
        announce(host, message);
    };
    if grid::takes_value(&op) && value.trim().is_empty() {
        problem(&texts.query_missing_value.replace("{column}", &column));
        return;
    }
    let Some(schema) = runtime(host).map(|runtime| runtime.borrow().state.schema().clone()) else {
        return;
    };
    let entry = FilterEntry {
        column: column.clone(),
        op: filter_op,
        value: value.clone(),
    };
    if let Err(problems) = grid::filter_expr(std::slice::from_ref(&entry), &schema) {
        if let Some(first) = problems.first() {
            problem(&texts.filter_invalid(&first.column, &first.value));
        }
        return;
    }
    let columns = columns_of(host);
    let Some(col) = columns.iter().position(|name| name == &column) else {
        return;
    };
    if let Some(operator) = OperatorControl::of(&root, col) {
        operator.set_value(&op);
    }
    if let Some(field) = ValueControl::of(&root, col) {
        field.set_disabled(!grid::takes_value(&op));
        field.set_value(&value);
    }
    close_quick(host, "filter-dialog", "add-filter", true);
    apply_filters(host);
}

/// The keys inside *Add filter*: `Tab` cycles, `Escape` closes, `Enter` in the
/// value applies.
pub(super) fn on_dialog_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(dialog)) = target.closest("[part=\"filter-dialog\"]") else {
        return;
    };
    match event.key().as_str() {
        "Escape" => {
            event.prevent_default();
            close_quick(host, "filter-dialog", "add-filter", true);
        }
        "Enter" if target.get_attribute("data-dialog").as_deref() == Some("value") => {
            event.prevent_default();
            apply_filter_dialog(host);
        }
        "Tab" => {
            let Ok(controls) =
                dialog.query_selector_all("select, label:not([hidden]) > input, button")
            else {
                return;
            };
            let controls: Vec<HtmlElement> = (0..controls.length())
                .filter_map(|index| controls.item(index))
                .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
                .collect();
            let (Some(first), Some(last)) = (controls.first(), controls.last()) else {
                return;
            };
            if event.shift_key() && first.is_same_node(Some(target)) {
                event.prevent_default();
                let _ = last.focus();
            } else if !event.shift_key() && last.is_same_node(Some(target)) {
                event.prevent_default();
                let _ = first.focus();
            }
        }
        _ => {}
    }
}

/// Opens *Group*: a menu of the groupable columns not grouped yet.
pub(super) fn open_group_menu(host: &HtmlElement) {
    let (Some(root), Some(document), Some(runtime)) = (
        host.shadow_root(),
        web_sys::window().and_then(|window| window.document()),
        runtime(host),
    ) else {
        return;
    };
    close_quick(host, "grouping-menu", "add-grouping", false);
    let current = runtime
        .borrow()
        .grouping
        .as_ref()
        .map(|grouping| grouping.by().to_vec())
        .unwrap_or_default();
    let offered: Vec<String> = shown_fields(host)
        .into_iter()
        .filter(|field| grouping::groupable(field.data_type))
        .map(|field| field.name.as_str().to_owned())
        .filter(|name| !current.contains(name))
        .collect();
    if current.len() >= grouping::MAX_LEVELS || offered.is_empty() {
        return;
    }
    let texts = texts(host);
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    for (name, value) in [
        ("part", "grouping-menu"),
        ("role", "menu"),
        ("popover", "auto"),
    ] {
        let _ = menu.set_attribute(name, value);
    }
    let _ = menu.set_attribute("aria-label", &texts.add_grouping);
    if !texts.lang.trim().is_empty() {
        let _ = menu.set_attribute("lang", &texts.lang);
    }
    for name in &offered {
        if let Ok(item) = document.create_element("div") {
            let _ = item.set_attribute("role", "menuitem");
            let _ = item.set_attribute("tabindex", "-1");
            let _ = item.set_attribute("data-group-column", name);
            // The column's name is the page's word, not ours.
            let _ = item.set_attribute("lang", "");
            item.set_text_content(Some(texts.column(name)));
            let _ = menu.append_child(&item);
        }
    }
    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    show_quick(&root, &menu, "add-grouping");
    if let Ok(Some(first)) = menu.query_selector("[role=\"menuitem\"]")
        && let Ok(first) = first.dyn_into::<HtmlElement>()
    {
        let _ = first.focus();
    }
}

/// The column menu's protocol, for *Group*.
pub(super) fn on_group_menu_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(menu)) = target.closest("[part=\"grouping-menu\"]") else {
        return;
    };
    let items = menu_items(&menu, "[role=\"menuitem\"]");
    if items.is_empty() {
        return;
    }
    match menu_key(event, &items, target) {
        Some(MenuAct::Pick(at)) => pick_grouping(host, &items[at]),
        Some(MenuAct::Escape | MenuAct::Tab) => {
            close_quick(host, "grouping-menu", "add-grouping", true)
        }
        None => {}
    }
}

/// Adds the item's column as the next grouping level.
pub(super) fn pick_grouping(host: &HtmlElement, item: &Element) {
    let Some(column) = item.get_attribute("data-group-column") else {
        return;
    };
    let mut next = runtime(host)
        .and_then(|runtime| {
            runtime
                .borrow()
                .grouping
                .as_ref()
                .map(|grouping| grouping.by().to_vec())
        })
        .unwrap_or_default();
    close_quick(host, "grouping-menu", "add-grouping", true);
    if next.len() >= grouping::MAX_LEVELS || next.contains(&column) {
        return;
    }
    next.push(column);
    let _ = host.set_attribute(grid::GROUP_BY_ATTRIBUTE, &next.join(","));
}

/// Opens a column's operator menu (issue #96): the comparisons its type
/// allows, the chosen one checked and focused — or the last one, with `last`
/// (`ArrowUp` on the button, as the menu-button pattern has it).
pub(super) fn open_operator_menu(host: &HtmlElement, col: usize, last: bool) {
    let (Some(root), Some(document)) = (
        host.shadow_root(),
        web_sys::window().and_then(|window| window.document()),
    ) else {
        return;
    };
    close_operator_menu(host, false);
    let Some(operator) = OperatorControl::of(&root, col) else {
        return;
    };
    let allowed = operator.allowed();
    if allowed.is_empty() {
        return;
    }
    let current = operator.value();
    let texts = texts(host);
    let column = columns_of(host).get(col).cloned().unwrap_or_default();
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    for (name, value) in [
        ("part", "operator-menu".to_owned()),
        ("role", "menu".to_owned()),
        ("popover", "auto".to_owned()),
        ("data-col", col.to_string()),
        ("aria-label", texts.operator_label(&column)),
    ] {
        let _ = menu.set_attribute(name, &value);
    }
    if !texts.lang.trim().is_empty() {
        let _ = menu.set_attribute("lang", &texts.lang);
    }
    for op in &allowed {
        let Ok(item) = document.create_element("div") else {
            continue;
        };
        let _ = item.set_attribute("role", "menuitemradio");
        let _ = item.set_attribute("tabindex", "-1");
        let _ = item.set_attribute("data-op", op);
        let _ = item.set_attribute(
            "aria-checked",
            if *op == current { "true" } else { "false" },
        );
        if let Ok(sign) = document.create_element("span") {
            let _ = sign.set_attribute("aria-hidden", "true");
            sign.set_text_content(Some(grid::operator_glyph(op)));
            let _ = item.append_child(&sign);
        }
        if let Ok(word) = document.create_element("span") {
            word.set_text_content(Some(&operator_word(&texts, op)));
            let _ = item.append_child(&word);
        }
        let _ = menu.append_child(&item);
    }
    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = root.append_child(&menu);
    let _ = menu.show_popover();
    let button = operator.element().clone();
    let _ = button.set_attribute("aria-expanded", "true");
    place_under(&menu, &button);
    // A click outside closes the popover by itself; the button must say so.
    let closed = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let open = js_sys::Reflect::get(&event, &JsValue::from_str("newState"))
            .ok()
            .and_then(|state| state.as_string())
            .is_some_and(|state| state == "open");
        if !open {
            if let Some(root) = root_of_target(&event)
                && let Some(operator) = OperatorControl::of(&root, col)
            {
                let _ = operator.element().set_attribute("aria-expanded", "false");
            }
            if let Some(popup) = event
                .target()
                .and_then(|target| target.dyn_into::<Element>().ok())
            {
                popup.remove();
            }
        }
    })
    .into_js_value();
    let _ = menu.add_event_listener_with_callback("toggle", closed.unchecked_ref());

    let selector = if last {
        "[role=\"menuitemradio\"]:last-child"
    } else {
        "[role=\"menuitemradio\"][aria-checked=\"true\"]"
    };
    if let Ok(Some(item)) = menu.query_selector(selector)
        && let Ok(item) = item.dyn_into::<HtmlElement>()
    {
        let _ = item.focus();
    }
}

/// Closes the open operator menu; with `refocus`, the focus goes back to its
/// button.
pub(super) fn close_operator_menu(host: &HtmlElement, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(Some(menu)) = root.query_selector("[part=\"operator-menu\"]") else {
        return;
    };
    let col = menu
        .get_attribute("data-col")
        .and_then(|col| col.parse::<usize>().ok());
    if let Ok(menu) = menu.dyn_into::<HtmlElement>() {
        let _ = menu.hide_popover();
        menu.remove();
    }
    if let Some(operator) = col.and_then(|col| OperatorControl::of(&root, col)) {
        let _ = operator.element().set_attribute("aria-expanded", "false");
        if refocus {
            let _ = operator.element().focus();
        }
    }
}

/// The column menu's keys, for the operator menu.
pub(super) fn on_operator_menu_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(menu)) = target.closest("[part=\"operator-menu\"]") else {
        return;
    };
    let items = menu_items(&menu, "[role=\"menuitemradio\"]");
    if items.is_empty() {
        return;
    }
    match menu_key(event, &items, target) {
        Some(MenuAct::Pick(at)) => pick_operator(host, &items[at]),
        Some(MenuAct::Escape | MenuAct::Tab) => close_operator_menu(host, true),
        None => {}
    }
}

/// Chooses the item's comparison for its column. A filter that can run —
/// a value in the field, or a comparison that takes none — runs at once; the
/// focus goes to the value, where the reader goes next, or back to the button
/// when there is no value to give.
pub(super) fn pick_operator(host: &HtmlElement, item: &Element) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let (Some(op), Some(col)) = (
        item.get_attribute("data-op"),
        item.closest("[part=\"operator-menu\"]")
            .ok()
            .flatten()
            .and_then(|menu| menu.get_attribute("data-col"))
            .and_then(|col| col.parse::<usize>().ok()),
    ) else {
        return;
    };
    close_operator_menu(host, false);
    let Some(operator) = OperatorControl::of(&root, col) else {
        return;
    };
    operator.set_value(&op);
    let takes = grid::takes_value(&op);
    let field = ValueControl::of(&root, col);
    if let Some(field) = &field {
        field.set_disabled(!takes);
    }
    let has_value = field
        .as_ref()
        .is_some_and(|field| !field.value().trim().is_empty());
    match (&field, takes) {
        (Some(field), true) => {
            let _ = field.element().focus();
        }
        _ => {
            let _ = operator.element().focus();
        }
    }
    if !takes || has_value {
        apply_filters(host);
    }
}

/// *Group* says when there is nothing more to add (issue #34).
pub(super) fn sync_quick_buttons(host: &HtmlElement, root: &ShadowRoot) {
    let Some(button) = quick_button(root, "add-grouping") else {
        return;
    };
    let full = runtime(host).is_some_and(|runtime| {
        runtime
            .borrow()
            .grouping
            .as_ref()
            .is_some_and(|grouping| grouping.levels() >= grouping::MAX_LEVELS)
    });
    let texts = texts(host);
    let visible = texts.add_grouping.as_str();
    if is_tree(host) {
        // A tree is its own hierarchy (issue #135): the button says why not.
        let _ = button.set_attribute("aria-disabled", "true");
        let _ = button.set_attribute(
            "aria-label",
            &crate::texts::reasoned(visible, &texts.tree_group_ignored),
        );
    } else if full {
        let _ = button.set_attribute("aria-disabled", "true");
        let _ = button.set_attribute(
            "aria-label",
            &crate::texts::reasoned(visible, &texts.grouping_full),
        );
    } else if button.has_attribute("aria-disabled") {
        let _ = button.remove_attribute("aria-disabled");
        let _ = button.remove_attribute("aria-label");
    }
}
