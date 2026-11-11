//! The search field: the field, its hint and its list of suggestions, and what typing in it does.

use super::*;

/// The field, its hint and its list, if the grid has them.
fn search_parts(root: &ShadowRoot) -> Option<(HtmlInputElement, Element, Element)> {
    let input = root
        .query_selector("[part=\"search-input\"]")
        .ok()
        .flatten()?
        .dyn_into::<HtmlInputElement>()
        .ok()?;
    let hint = root
        .query_selector("[part=\"search-hint\"]")
        .ok()
        .flatten()?;
    let list = root
        .query_selector("[part=\"search-list\"]")
        .ok()
        .flatten()?;
    Some((input, hint, list))
}

/// Typing: the hint says whether the input reads as a filter, the list offers
/// the columns the last word could be.
pub(super) fn on_search_input(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
    else {
        return;
    };
    if target.get_attribute("part").as_deref() != Some("search-input") {
        return;
    }
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    update_search(&host, &root);
}

fn update_search(host: &HtmlElement, root: &ShadowRoot) {
    let Some((input, hint, list)) = search_parts(root) else {
        return;
    };
    let value = input.value();
    let columns = columns_of(host);
    let texts = texts(host);
    let query = crate::search::looks_like_query(&value);
    let _ = if query {
        hint.remove_attribute("hidden")
            .and(input.set_attribute("data-query", ""))
    } else {
        hint.set_attribute("hidden", "")
            .and(input.remove_attribute("data-query"))
    };

    list.set_inner_html("");
    let _ = input.remove_attribute("aria-activedescendant");
    let offered = crate::search::suggestions(&value, &texts.query_and, &texts.query_or, &columns);
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let schema = runtime(host).map(|runtime| runtime.borrow().state.schema().clone());
    for (index, column) in offered.iter().enumerate() {
        let Ok(option) = document.create_element("li") else {
            continue;
        };
        let _ = option.set_attribute("role", "option");
        let _ = option.set_attribute("id", &format!("og-search-option-{index}"));
        let _ = option.set_attribute("aria-selected", "false");
        let _ = option.set_attribute("data-column", column);
        // The title reads; the field name is what the expression takes, so a
        // titled column shows both (issue #66).
        let title = texts.column(column);
        if title == column {
            option.set_text_content(Some(column));
        } else {
            option.set_text_content(Some(&format!("{title} \u{b7} {column}")));
        }
        // The type beside the name, as the prototype shows it. The option is
        // the page's column name and claims no language; the type is our word
        // (F8), so it carries ours.
        if let Some(field) = schema.as_ref().and_then(|schema| {
            schema
                .fields()
                .iter()
                .find(|field| field.name.as_str() == column)
        }) && let Ok(kind) = document.create_element("span")
        {
            let _ = kind.set_attribute("aria-hidden", "true");
            if !texts.lang.trim().is_empty() {
                let _ = kind.set_attribute("lang", &texts.lang);
            }
            let name = match field.data_type {
                DataType::Utf8 => &texts.type_text,
                DataType::Bool => &texts.type_bool,
                DataType::Int64 => &texts.type_integer,
                DataType::Float64 | DataType::Decimal { .. } => &texts.type_number,
                DataType::Date => &texts.type_date,
                DataType::Timestamp => &texts.type_time,
                DataType::ListUtf8 | DataType::ListInt64 => &texts.type_list,
            };
            kind.set_text_content(Some(name));
            let _ = option.append_child(&kind);
        }
        let _ = list.append_child(&option);
    }
    let open = !offered.is_empty();
    let _ = if open {
        list.remove_attribute("hidden")
    } else {
        list.set_attribute("hidden", "")
    };
    let _ = input.set_attribute("aria-expanded", &open.to_string());
}

/// The combobox keys: `↓`/`↑` through the suggestions, `Enter` takes one or
/// applies the field, `Escape` closes the list — or, when it is closed, empties
/// the field and the search. The focus stays in the field throughout.
pub(super) fn on_search_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    if target.get_attribute("part").as_deref() != Some("search-input") {
        return;
    }
    let Some((input, _, list)) = search_parts(&root) else {
        return;
    };
    let options: Vec<Element> = list
        .query_selector_all("[role=\"option\"]")
        .ok()
        .map(|all| {
            (0..all.length())
                .filter_map(|index| all.item(index))
                .filter_map(|node| node.dyn_into::<Element>().ok())
                .collect()
        })
        .unwrap_or_default();
    let open = !list.has_attribute("hidden") && !options.is_empty();
    let active = input
        .get_attribute("aria-activedescendant")
        .and_then(|id| options.iter().position(|option| option.id() == id));
    let point = |at: usize| {
        for (index, option) in options.iter().enumerate() {
            let _ = option.set_attribute("aria-selected", &(index == at).to_string());
        }
        let _ = input.set_attribute("aria-activedescendant", &options[at].id());
    };
    match event.key().as_str() {
        "ArrowDown" if open => {
            event.prevent_default();
            point(active.map_or(0, |at| (at + 1) % options.len()));
        }
        "ArrowUp" if open => {
            event.prevent_default();
            point(active.map_or(options.len() - 1, |at| {
                (at + options.len() - 1) % options.len()
            }));
        }
        "Enter" => {
            event.prevent_default();
            match active.filter(|_| open) {
                Some(at) => take_suggestion(host, &options[at]),
                None => apply_search(host),
            }
        }
        "Escape" => {
            event.prevent_default();
            if open {
                let _ = list.set_attribute("hidden", "");
                let _ = input.set_attribute("aria-expanded", "false");
                let _ = input.remove_attribute("aria-activedescendant");
            } else {
                input.set_value("");
                update_search(host, &root);
                let had =
                    runtime(host).is_some_and(|runtime| !runtime.borrow().search_text.is_empty());
                if had {
                    if let Some(runtime) = runtime(host) {
                        runtime.borrow_mut().search_text.clear();
                    }
                    apply_facets(host);
                }
            }
        }
        "Tab" => {
            let _ = list.set_attribute("hidden", "");
            let _ = input.set_attribute("aria-expanded", "false");
        }
        _ => {}
    }
}

/// Puts a suggested column into the field, followed by a space, and keeps the
/// focus there — the next thing to type is the operator.
pub(super) fn take_suggestion(host: &HtmlElement, option: &Element) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(column) = option.get_attribute("data-column") else {
        return;
    };
    let Some((input, _, _)) = search_parts(&root) else {
        return;
    };
    let texts = texts(host);
    input.set_value(&crate::search::complete(
        &input.value(),
        &texts.query_and,
        &texts.query_or,
        &column,
    ));
    update_search(host, &root);
    let _ = input.focus();
}

/// Applies the field: an expression becomes filter-row entries — or, with an
/// `or`, the nested filter — anything else a free-text search.
///
/// An expression that does not parse is **said**, and the field keeps it so
/// it can be corrected — falling back to free text would look like it worked
/// and show the wrong rows.
fn apply_search(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some((input, _, list)) = search_parts(&root) else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let value = input.value();
    let columns = columns_of(host);
    let texts = texts(host);
    let _ = list.set_attribute("hidden", "");
    let _ = input.set_attribute("aria-expanded", "false");

    if crate::search::looks_like_query(&value) {
        let schema = runtime.borrow().state.schema().clone();
        let styles = presentation::styles(host);
        let lookups: Vec<String> = columns
            .iter()
            .filter(|name| styles.lookup(name).is_some())
            .cloned()
            .collect();
        match crate::search::parse_for(
            &value,
            &texts.query_and,
            Some(&texts.query_or),
            &schema,
            &lookups,
        ) {
            // With an `or` it is the nested filter (issue #324): the one
            // place such a filter lives, and it replaces the one there was.
            Ok(crate::search::Parsed::Tree(tree)) => {
                input.set_value("");
                update_search(host, &root);
                builder::set(host, Some(tree));
            }
            Ok(crate::search::Parsed::Row(entries)) => {
                // Written into the filter row's own fields and applied by its
                // own code: one place a filter lives (F5).
                for entry in &entries {
                    let Some(col) = columns.iter().position(|name| name == &entry.column) else {
                        continue;
                    };
                    if let Some(operator) = OperatorControl::of(&root, col) {
                        operator.set_value(entry.op.as_str());
                    }
                    if let Some(field) = ValueControl::of(&root, col) {
                        field.set_disabled(false);
                        field.set_value(&entry.value);
                    }
                }
                input.set_value("");
                update_search(host, &root);
                apply_filters(host);
            }
            Err(problem) => {
                runtime
                    .borrow_mut()
                    .state
                    .set_status(GridStatus::Error(texts.query_problem(&problem)));
                render(host, false);
            }
        }
        return;
    }

    runtime.borrow_mut().search_text = value.trim().to_owned();
    apply_facets(host);
}
