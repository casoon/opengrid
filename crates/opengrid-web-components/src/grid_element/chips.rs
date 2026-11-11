//! The toolbar and the chips: what the grid shows of the reader's filter, the grouping and the search, kept in line with the state.

use super::*;

/// Brings the toolbar, the chips and the filter row in line with the state.
///
/// Runs after every render, so it has to be cheap and it must **not** touch
/// what did not change: the chips are rebuilt only when what they say changed.
/// A scroll frame that rebuilt them would take the focus off a chip the reader
/// is standing on.
pub(super) fn sync_chrome(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    draw_facets(host, &root);
    draw_empty(host, &root, &runtime);
    let shown = runtime.borrow().filter_row;
    if let Ok(Some(row)) = root.query_selector("[part=\"filter\"]") {
        let _ = if shown {
            row.remove_attribute("hidden")
        } else {
            row.set_attribute("hidden", "")
        };
    }
    sync_quick_buttons(host, &root);
    let Ok(Some(toolbar)) = root.query_selector("[part=\"toolbar\"]") else {
        return;
    };
    if let Ok(Some(toggle)) = toolbar.query_selector("[data-toolbar=\"filter-row\"]") {
        let _ = toggle.set_attribute("aria-pressed", &shown.to_string());
    }
    let density = grid::density_of(host.get_attribute(grid::DENSITY_ATTRIBUTE).as_deref()).0;
    if let Ok(buttons) = toolbar.query_selector_all("[data-density]") {
        for index in 0..buttons.length() {
            if let Some(button) = buttons
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                let pressed = button.get_attribute("data-density").as_deref() == Some(density);
                let _ = button.set_attribute("aria-pressed", &pressed.to_string());
            }
        }
    }
    if let Ok(Some(switch)) = toolbar.query_selector("[data-toolbar=\"facets\"]") {
        let _ = switch.set_attribute(
            "aria-pressed",
            &host.has_attribute(grid::FACETS_ATTRIBUTE).to_string(),
        );
    }
    draw_chips(host, &root);
}

/// One chip: what it says, and how it is removed.
pub(super) struct Chip {
    /// The filter or grouping in words: "country is DE".
    pub(super) text: String,
    /// `column:<name>` for a filter, `group` for the grouping.
    removes: String,
}

/// The chips the current view calls for.
pub(super) fn chips_of(host: &HtmlElement) -> Vec<Chip> {
    let Some(view) = current_view(host) else {
        return Vec::new();
    };
    let texts = texts(host);
    let mut out = Vec::new();
    if !view.group.is_empty() {
        out.push(Chip {
            text: texts.group_chip(
                &view
                    .group
                    .iter()
                    .map(|column| texts.column(column))
                    .collect::<Vec<_>>()
                    .join(" \u{203A} "),
            ),
            removes: "group".to_owned(),
        });
    }
    let facet_chips: Vec<Chip> = runtime(host)
        .map(|runtime| {
            let borrowed = runtime.borrow();
            let schema = borrowed.state.schema().clone();
            borrowed
                .facets
                .iter()
                .filter(|(_, selection)| selection.is_active())
                .map(|(column, selection)| Chip {
                    text: facet_chip_text(
                        host,
                        column,
                        selection,
                        &schema,
                        borrowed.facet_domain.get(column),
                    ),
                    removes: format!("facet:{column}"),
                })
                .collect()
        })
        .unwrap_or_default();
    out.extend(facet_chips);
    if let Some(runtime) = runtime(host) {
        let text = runtime.borrow().search_text.clone();
        if !text.trim().is_empty() {
            out.push(Chip {
                text: texts.search_chip(text.trim()),
                removes: "search".to_owned(),
            });
        }
    }
    for entry in &view.filters {
        out.push(Chip {
            text: builder::condition_words(&texts, entry),
            removes: format!("column:{}", entry.column),
        });
    }
    // The nested filter is one chip, in words, removed as a whole (issue #324).
    if let Some(nested) = runtime(host).and_then(|runtime| runtime.borrow().nested.clone()) {
        out.push(Chip {
            text: builder::words(host, &nested),
            removes: NESTED_CHIP.to_owned(),
        });
    }
    out
}

/// What the nested filter's chip removes.
const NESTED_CHIP: &str = "where";

/// Draws the chips, but only when they say something different from before.
pub(super) fn draw_chips(host: &HtmlElement, root: &ShadowRoot) {
    let Ok(Some(group)) = root.query_selector("[part=\"chips\"]") else {
        return;
    };
    let chips = chips_of(host);
    let signature = chips
        .iter()
        .map(|chip| format!("{}={}", chip.removes, chip.text))
        .collect::<Vec<_>>()
        .join("\u{1F}");
    if group.get_attribute("data-drawn").as_deref() == Some(signature.as_str()) {
        return;
    }
    let _ = group.set_attribute("data-drawn", &signature);
    group.set_inner_html("");
    if chips.is_empty() {
        let _ = group.set_attribute("hidden", "");
        return;
    }
    let _ = group.remove_attribute("hidden");

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let texts = texts(host);
    for chip in &chips {
        let Ok(span) = document.create_element("span") else {
            continue;
        };
        let _ = span.set_attribute("part", "chip");
        // The grouping chip looks different from a filter's (issue #33).
        if chip.removes == "group" {
            let _ = span.set_attribute("data-kind", "group");
        }
        // The chip's words are the page's column name and value mixed with our
        // operator word — no `lang` fits both halves (the open question from
        // phase E (k), for point 71).
        // The nested filter's words are a button that opens its builder.
        let edits = chip.removes == NESTED_CHIP;
        if let Ok(text) = document.create_element(if edits { "button" } else { "span" }) {
            text.set_text_content(Some(&chip.text));
            if edits {
                let _ = text.set_attribute("type", "button");
                let _ = text.set_attribute("part", "chip-edit");
                let _ = text.set_attribute("data-chip-edit", "");
                let _ = text.set_attribute("aria-haspopup", "dialog");
                let _ = text.set_attribute("aria-label", &texts.filter_builder_edit(&chip.text));
            }
            let _ = span.append_child(&text);
        }
        if let Ok(button) = document.create_element("button") {
            let _ = button.set_attribute("type", "button");
            let _ = button.set_attribute("part", "chip-remove");
            let _ = button.set_attribute("data-chip-remove", &chip.removes);
            // A name that says *which* filter: ten buttons called "Remove"
            // are ten buttons nobody can tell apart.
            let _ = button.set_attribute("aria-label", &texts.chip_remove(&chip.text));
            button.set_text_content(Some("\u{00D7}"));
            let _ = span.append_child(&button);
        }
        let _ = group.append_child(&span);
    }
    if let Ok(clear) = document.create_element("button") {
        let _ = clear.set_attribute("type", "button");
        let _ = clear.set_attribute("part", "chips-clear");
        let _ = clear.set_attribute("data-chips-clear", "");
        if !texts.lang.trim().is_empty() {
            let _ = clear.set_attribute("lang", &texts.lang);
        }
        clear.set_text_content(Some(&texts.chips_clear));
        let _ = group.append_child(&clear);
    }
}

/// Shows or hides the filter row.
///
/// The viewport grows by exactly the row's height when it hides — `display:
/// none`, not `visibility` — so `PageUp`/`PageDown` step by what is really
/// there. The window is asked again, because more rows may now fit.
pub(super) fn toggle_filter_row(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.filter_row = !borrowed.filter_row;
    }
    sync_chrome(host);
    // Shown, the filter row asks more width of every column (issue #62).
    apply_widths(host);
    run_query(host, QueryKind::Window, false);
    dispatch_view(host);
}

/// Resets one column of the filter row to "no filter".
///
/// Not only the value: an `is_null` filter takes none, so emptying the field
/// would leave it standing. The operator goes back to the column's first
/// offered one.
fn reset_filter_column(root: &ShadowRoot, col: usize) {
    if let Some(operator) = OperatorControl::of(root, col) {
        operator.reset();
    }
    if let Some(control) = ValueControl::of(root, col) {
        control.set_value("");
        control.set_disabled(false);
    }
}

/// Removes what one chip stands for, says so, and keeps the focus in the chips.
pub(super) fn remove_chip(host: &HtmlElement, button: &Element) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(removes) = button.get_attribute("data-chip-remove") else {
        return;
    };
    let said = button
        .closest("[part=\"chip\"]")
        .ok()
        .flatten()
        .and_then(|chip| chip.text_content())
        .map(|text| text.trim_end_matches('\u{00D7}').trim().to_owned())
        .unwrap_or_default();
    // Where the focus goes next: the chip after this one, else the one before,
    // else "Remove all" — never lost to the document.
    let position = root
        .query_selector_all("[data-chip-remove]")
        .ok()
        .and_then(|all| {
            (0..all.length()).position(|index| {
                all.item(index)
                    .is_some_and(|node| node.is_same_node(Some(button.as_ref())))
            })
        })
        .unwrap_or(0);

    // The query first, the sentence after: `set_notice` rides with the *next*
    // result, and set before the query it would also ride with the "loading"
    // frame the query draws — said twice (phase E (j)).
    if removes == "group" {
        let _ = host.remove_attribute(grid::GROUP_BY_ATTRIBUTE);
    } else if removes == NESTED_CHIP {
        if let Some(runtime) = runtime(host) {
            runtime.borrow_mut().nested = None;
        }
        apply_facets(host);
    } else if removes == "search" {
        if let Some(runtime) = runtime(host) {
            runtime.borrow_mut().search_text.clear();
        }
        apply_facets(host);
    } else if let Some(column) = removes.strip_prefix("facet:") {
        if let Some(runtime) = runtime(host) {
            for (name, selection) in runtime.borrow_mut().facets.iter_mut() {
                if name == column {
                    *selection = clear_selection(selection);
                }
            }
        }
        apply_facets(host);
    } else if let Some(column) = removes.strip_prefix("column:")
        && let Some(col) = columns_of(host).iter().position(|name| name == column)
    {
        reset_filter_column(&root, col);
        apply_filters(host);
    }
    if let Some(runtime) = runtime(host) {
        runtime
            .borrow_mut()
            .state
            .set_notice(texts(host).filter_removed(&said));
    }

    draw_chips(host, &root);
    let next = root
        .query_selector_all("[data-chip-remove]")
        .ok()
        .and_then(|all| {
            let count = all.length() as usize;
            (count > 0)
                .then(|| all.item(position.min(count - 1) as u32))
                .flatten()
        })
        .or_else(|| {
            root.query_selector("[data-toolbar=\"filter-row\"]")
                .ok()
                .flatten()
                .map(Node::from)
        });
    if let Some(next) = next.and_then(|node| node.dyn_into::<HtmlElement>().ok()) {
        let _ = next.focus();
    }
}

/// Removes every filter and the grouping, says so once, and puts the focus on
/// the toolbar — the chips it stood in are gone.
pub(super) fn clear_chips(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    for col in 0..columns_of(host).len() {
        reset_filter_column(&root, col);
    }
    let grouped = host.has_attribute(grid::GROUP_BY_ATTRIBUTE);
    if grouped {
        let _ = host.remove_attribute(grid::GROUP_BY_ATTRIBUTE);
    }
    if let Some(runtime) = runtime(host) {
        let mut borrowed = runtime.borrow_mut();
        for (_, selection) in borrowed.facets.iter_mut() {
            *selection = clear_selection(selection);
        }
        borrowed.search_text.clear();
        borrowed.nested = None;
    }
    apply_filters(host);
    // After the query, so the sentence rides with its result only (phase E (j)).
    if let Some(runtime) = runtime(host) {
        runtime
            .borrow_mut()
            .state
            .set_notice(texts(host).filters_cleared.clone());
    }
    draw_chips(host, &root);
    if let Ok(Some(toggle)) = root.query_selector("[data-toolbar=\"filter-row\"]")
        && let Ok(toggle) = toggle.dyn_into::<HtmlElement>()
    {
        let _ = toggle.focus();
    }
}
