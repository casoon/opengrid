//! The column menu: what a header offers — sort, filter, pin, group, hide, aggregate — built when it opens, so it shows the current state.

use super::*;

/// Opens the menu of the column at `col`, focus on its first entry.
///
/// Built when it opens rather than kept in the skeleton: it shows *current*
/// state — which sort is checked, which aggregate, whether the column is
/// grouped — and a menu that is rebuilt cannot be stale.
pub(super) fn open_column_menu(host: &HtmlElement, col: usize) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    close_column_menu(host, false);

    let texts = texts(host);
    let (entries, name) = {
        let borrowed = runtime.borrow();
        let Some(field) = borrowed.state.schema().fields().get(col).cloned() else {
            return;
        };
        let name = field.name.as_str().to_owned();
        let sort = borrowed
            .state
            .sort_keys()
            .into_iter()
            .find(|(column, _)| column == &name)
            .map(|(_, direction)| direction);
        let aggregate = borrowed.aggregate_choice.get(&name).copied().or_else(|| {
            borrowed
                .grouping
                .as_ref()
                .and_then(|grouping| {
                    grouping
                        .aggregates()
                        .iter()
                        .find(|(column, _)| column == &name)
                        .map(|(_, function)| *function)
                })
                .or_else(|| {
                    presentation::styles(host)
                        .get(&name)
                        .and_then(|column| column.aggregate)
                })
        });
        let group_by = borrowed
            .grouping
            .as_ref()
            .map(|grouping| grouping.by().to_vec())
            .unwrap_or_default();
        let column = crate::column_menu::Column {
            name: &name,
            data_type: field.data_type,
            sort,
            aggregate,
            group_by: &group_by,
            can_group: !is_paging(host),
            visible: borrowed.state.schema().fields().len(),
            bulk: bulk::available(host, &borrowed, col),
            pin: pin_of(host, &name),
            lookup: presentation::styles(host).lookup(&name).is_some(),
        };
        (crate::column_menu::entries(&column, &texts), name)
    };

    show_menu(host, &root, col, &name, entries);
}

/// Opens the menu of the band whose cell is at `col` (issue #352).
pub(super) fn open_band_menu(host: &HtmlElement, col: usize) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some((band, _, count)) = bands_of(host)
        .into_iter()
        .find(|(_, first, _)| *first == col)
    else {
        return;
    };
    close_column_menu(host, false);
    // A band that is all there is has nothing to hide.
    if count >= columns_of(host).len() {
        return;
    }
    show_menu(
        host,
        &root,
        col,
        &band,
        crate::column_menu::band_entries(&texts(host)),
    );
}

/// Builds the popup for `entries` under the header cell at `col` and puts the
/// focus on its first entry.
fn show_menu(
    host: &HtmlElement,
    root: &ShadowRoot,
    col: usize,
    name: &str,
    entries: Vec<crate::column_menu::Entry>,
) {
    let texts = texts(host);
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    for (attribute, value) in [
        ("part", "column-menu"),
        ("role", "menu"),
        ("popover", "auto"),
        ("data-col", &col.to_string()),
    ] {
        let _ = menu.set_attribute(attribute, value);
    }
    let _ = menu.set_attribute("aria-label", &texts.column_menu(name));
    // Our words throughout — the menu names no data of the page's except the
    // column in its label, and that one is also the page's word for it.
    if !texts.lang.trim().is_empty() {
        let _ = menu.set_attribute("lang", &texts.lang);
    }
    append_entries(&document, &menu, &entries);
    let _ = root.append_child(&menu);

    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = menu.show_popover();
    place_menu(root, &menu, col);
    if let Ok(Some(first)) = menu.query_selector("[role^=\"menuitem\"]")
        && let Ok(first) = first.dyn_into::<HtmlElement>()
    {
        let _ = first.focus();
    }
}

/// Writes the entries into the menu element.
fn append_entries(
    document: &web_sys::Document,
    parent: &Element,
    entries: &[crate::column_menu::Entry],
) {
    use crate::column_menu::Entry;
    for entry in entries {
        match entry {
            Entry::Separator => {
                if let Ok(line) = document.create_element("div") {
                    let _ = line.set_attribute("role", "separator");
                    let _ = parent.append_child(&line);
                }
            }
            Entry::Group { label, entries } => {
                let Ok(group) = document.create_element("div") else {
                    continue;
                };
                let _ = group.set_attribute("role", "group");
                // The group's name, visible and referenced: `aria-labelledby`
                // points inside the same shadow root, which E8/R6 allows.
                if let Ok(caption) = document.create_element("div") {
                    let id = format!("og-menu-{}", label.len());
                    let _ = caption.set_attribute("part", "menu-label");
                    let _ = caption.set_attribute("id", &id);
                    caption.set_text_content(Some(label));
                    let _ = group.append_child(&caption);
                    let _ = group.set_attribute("aria-labelledby", &id);
                }
                append_entries(document, &group, entries);
                let _ = parent.append_child(&group);
            }
            Entry::Item {
                action,
                label,
                checked,
            } => {
                let Ok(item) = document.create_element("div") else {
                    continue;
                };
                match checked {
                    Some(checked) => {
                        let _ = item.set_attribute("role", "menuitemradio");
                        let _ = item.set_attribute("aria-checked", &checked.to_string());
                    }
                    None => {
                        let _ = item.set_attribute("role", "menuitem");
                    }
                }
                let _ = item.set_attribute("tabindex", "-1");
                let _ = item.set_attribute("data-action", action);
                item.set_text_content(Some(label));
                let _ = parent.append_child(&item);
            }
        }
    }
}

/// Puts the menu under its header — never over it (WCAG 2.2 §2.4.11): the
/// focus returns there when the menu closes, and a menu that covered it would
/// have hidden where the reader is going back to. Right-aligned when it would
/// run off the right edge, above the header when it would run off the bottom.
fn place_menu(root: &ShadowRoot, menu: &HtmlElement, col: usize) {
    let Ok(Some(header)) = root.query_selector(&format!("th[data-col=\"{col}\"]")) else {
        return;
    };
    let cell = header.get_bounding_client_rect();
    let own = menu.get_bounding_client_rect();
    let height = web_sys::window()
        .and_then(|window| window.inner_height().ok())
        .and_then(|value| value.as_f64())
        .unwrap_or(768.0);
    const GAP: f64 = 4.0;
    let left = crate::element::popup_left(&cell, &own, crate::element::is_rtl(&header));
    let mut top = cell.bottom() + GAP;
    if top + own.height() > height - GAP && cell.top() - own.height() - GAP >= GAP {
        top = cell.top() - own.height() - GAP;
    }
    let _ = menu.style().set_property("left", &format!("{left}px"));
    let _ = menu.style().set_property("top", &format!("{top}px"));
}

/// Closes an open menu; with `refocus`, the focus goes back to its header.
fn close_column_menu(host: &HtmlElement, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(menus) = root.query_selector_all("[part=\"column-menu\"]") else {
        return;
    };
    let mut col = None;
    for index in 0..menus.length() {
        let Some(menu) = menus
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        else {
            continue;
        };
        col = col.or_else(|| {
            menu.get_attribute("data-col")
                .and_then(|col| col.parse().ok())
        });
        let _ = menu.hide_popover();
        menu.remove();
    }
    if refocus
        && let Some(col) = col
        && let Some(runtime) = runtime(host)
    {
        runtime.borrow_mut().set_active(ActiveCell::Header { col });
        focus_active(host);
    }
}

/// The keys inside the menu: the arrows, `Home`/`End`, `Enter`/`Space`,
/// `Escape` and `Tab` — the protocol written down in point 64 before building.
pub(super) fn on_menu_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(menu)) = target.closest("[part=\"column-menu\"]") else {
        return;
    };
    let items = menu_items(&menu, "[role^=\"menuitem\"]");
    if items.is_empty() {
        return;
    }
    match menu_key(event, &items, target) {
        Some(MenuAct::Pick(at)) => activate_menu_item(host, &items[at]),
        Some(MenuAct::Escape | MenuAct::Tab) => close_column_menu(host, true),
        None => {}
    }
}

/// Does what an entry says, closes the menu, and puts the focus back.
///
/// Nothing here announces: the result of each action does, through the one
/// live region (point 41) — a sort re-queries, a hidden column says so, a
/// grouping reloads.
pub(super) fn activate_menu_item(host: &HtmlElement, item: &Element) {
    let Some(action) = item.get_attribute("data-action") else {
        return;
    };
    let Some(col) = item
        .closest("[part=\"column-menu\"]")
        .ok()
        .flatten()
        .and_then(|menu| menu.get_attribute("data-col"))
        .and_then(|col| col.parse::<usize>().ok())
    else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(name) = runtime
        .borrow()
        .state
        .schema()
        .fields()
        .get(col)
        .map(|field| field.name.as_str().to_owned())
    else {
        return;
    };
    close_column_menu(host, true);

    match action.as_str() {
        "sort:asc" | "sort:desc" => {
            let direction = if action == "sort:asc" {
                SortDirection::Asc
            } else {
                SortDirection::Desc
            };
            let pool = pool_of(host);
            {
                let mut borrowed = runtime.borrow_mut();
                if let Ok(field) = opengrid_types::FieldName::new(&name) {
                    borrowed.state.set_sort(vec![Sort {
                        field,
                        direction,
                        nulls: Default::default(),
                        collation: Default::default(),
                    }]);
                }
                borrowed.state.set_window(Window::new(0, pool));
            }
            announce_if_cleared(host, &runtime);
            run_query(host, QueryKind::Data, true);
            dispatch_view(host);
        }
        "filter" => {
            // Into the filter row, where filtering already lives (point 48). A
            // column whose operator takes no value has its field disabled, so
            // the operator is where the focus can go.
            if let Some(root) = host.shadow_root() {
                let field = ValueControl::of(&root, col)
                    .filter(|control| !control.is_disabled())
                    .map(|control| Element::from(control.element().clone()))
                    .or_else(|| {
                        OperatorControl::of(&root, col)
                            .map(|operator| Element::from(operator.element().clone()))
                    });
                if let Some(field) = field.and_then(|field| field.dyn_into::<HtmlElement>().ok()) {
                    let _ = field.focus();
                }
            }
        }
        "hide" => set_column_hidden(host, &name, true),
        "hide-band" => hide_band(host, col),
        "pin:none" => pin_column(host, &name, None),
        "pin:start" => pin_column(host, &name, Some(Pin::Start)),
        "pin:end" => pin_column(host, &name, Some(Pin::End)),
        "bulk" => bulk::open(host, col),
        grouping if grouping.starts_with("group:") => {
            let current = runtime
                .borrow()
                .grouping
                .as_ref()
                .map(|grouping| grouping.by().to_vec())
                .unwrap_or_default();
            let next = crate::column_menu::regrouped(grouping, &name, &current);
            if next.is_empty() {
                let _ = host.remove_attribute(grid::GROUP_BY_ATTRIBUTE);
            } else {
                let _ = host.set_attribute(grid::GROUP_BY_ATTRIBUTE, &next.join(","));
            }
        }
        aggregate if aggregate.starts_with("aggregate:") => {
            let choice = presentation::aggregate_from(&aggregate["aggregate:".len()..]);
            {
                let mut borrowed = runtime.borrow_mut();
                match choice {
                    Some(function) => {
                        borrowed.aggregate_choice.insert(name.clone(), function);
                    }
                    None => {
                        borrowed.aggregate_choice.remove(&name);
                    }
                }
                // The counts were asked together with the old aggregates.
                if let Some(grouping) = borrowed.grouping.as_mut() {
                    grouping.invalidate();
                }
                borrowed.groups_filter = None;
            }
            run_query(host, QueryKind::Window, true);
            dispatch_view(host);
        }
        _ => {}
    }
}
