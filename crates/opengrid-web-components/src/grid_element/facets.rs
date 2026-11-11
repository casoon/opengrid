//! Facets: the reader's value lists per column, their counts, and what a change
//! of one does to the filter.

use super::*;

/// The filter a query runs under: the page's `filter` attribute **and** what
/// the reader chose ([`reader_filter`]). Every query of the grid takes its
/// filter from here, so the page's filter holds for all of them (issue #330).
pub(super) fn effective_filter(
    host: &HtmlElement,
    runtime: &GridRuntime,
    except: Option<&str>,
) -> Result<Option<FilterExpr>, String> {
    let reader = reader_filter(host, runtime, except)?;
    Ok(match (page_filter(host)?, reader) {
        (Some(page), Some(reader)) => Some(FilterExpr::And(vec![page, reader])),
        (page, reader) => page.or(reader),
    })
}

/// The page's `filter` attribute as an expression: absent or empty is none,
/// and what is not a filter is a sentence for the status line — never a grid
/// that silently shows every row.
pub(super) fn page_filter(host: &HtmlElement) -> Result<Option<FilterExpr>, String> {
    let raw = host
        .get_attribute(grid::FILTER_ATTRIBUTE)
        .unwrap_or_default();
    if raw.trim().is_empty() {
        return Ok(None);
    }
    opengrid_json::from_str::<FilterExpr>(&raw)
        .map(Some)
        .map_err(|error| format!("the filter attribute is not a filter: {error}"))
}

/// What the reader filters by: the filter row's **and** every facet's —
/// except `except`, which is how a facet is counted without its own
/// restriction. A bound that is not a value of its column is a sentence for the
/// status line, not a silently dropped half of a range.
pub(super) fn reader_filter(
    host: &HtmlElement,
    runtime: &GridRuntime,
    except: Option<&str>,
) -> Result<Option<FilterExpr>, String> {
    // The free text of the search field is one more filter under the facets
    // (point 67) — part of the base, so the facet counts see it too.
    let text = crate::search::free_text(&runtime.search_text, runtime.state.schema());
    // The filter row, the free text and the nested filter (issue #324), and-ed.
    let mut parts: Vec<FilterExpr> = runtime
        .state
        .filter()
        .cloned()
        .into_iter()
        .chain(text)
        .chain(runtime.nested.clone())
        .collect();
    let base = match parts.len() {
        0 | 1 => parts.pop(),
        _ => Some(FilterExpr::And(parts)),
    };
    crate::facets::effective(
        base.as_ref(),
        &runtime.facets,
        except,
        runtime.state.schema(),
    )
    .map_err(|problems| {
        let texts = texts(host);
        problems
            .iter()
            .map(|problem| texts.filter_invalid(&problem.column, &problem.value))
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// The empty selection of the same kind.
pub(super) fn clear_selection(selection: &crate::facets::Selection) -> crate::facets::Selection {
    use crate::facets::Selection;
    match selection {
        Selection::Values(_) => Selection::Values(Vec::new()),
        Selection::Range { .. } => Selection::Range {
            min: String::new(),
            max: String::new(),
        },
        Selection::Period { .. } => Selection::Period {
            from: String::new(),
            to: String::new(),
        },
    }
}

/// Brings the reader's selections in line with what the page configured: one
/// per configured facet, in the page's order, keeping what was chosen.
fn sync_facets(host: &HtmlElement, runtime: &mut GridRuntime) {
    let configured = presentation::styles(host).facets();
    let mut next = Vec::with_capacity(configured.len());
    for (column, kind) in configured {
        let kept = runtime
            .facets
            .iter()
            .find(|(name, _)| name == &column)
            .map(|(_, selection)| selection.clone())
            .filter(|selection| {
                matches!(
                    (selection, kind),
                    (
                        crate::facets::Selection::Values(_),
                        presentation::FacetKind::List | presentation::FacetKind::Pills
                    ) | (
                        crate::facets::Selection::Range { .. },
                        presentation::FacetKind::Range
                    ) | (
                        crate::facets::Selection::Period { .. },
                        presentation::FacetKind::Period
                    )
                )
            });
        next.push((
            column,
            kept.unwrap_or_else(|| crate::facets::Selection::empty(kind)),
        ));
    }
    runtime.facets = next;
}

/// Counts every list facet under every filter but its own (F4: always).
///
/// One `group` query per facet column — plus, once, the unfiltered list of its
/// values, so a value the other facets exclude still shows (with 0) instead of
/// vanishing from under the reader's pointer.
pub(super) fn refresh_facets(host: &HtmlElement) {
    if !host.has_attribute(grid::FACETS_ATTRIBUTE) {
        return;
    }
    let Some(provider) = provider(host) else {
        return;
    };
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let Some(grid_runtime) = runtime(host) else {
        return;
    };
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let configured = presentation::styles(host).facets();
    let (generation, plan) = {
        let mut runtime = grid_runtime.borrow_mut();
        sync_facets(host, &mut runtime);
        runtime.facet_generation += 1;
        let plan: Vec<(String, bool, Option<FilterExpr>)> = configured
            .iter()
            .filter(|(_, kind)| {
                matches!(
                    kind,
                    presentation::FacetKind::List | presentation::FacetKind::Pills
                )
            })
            .filter_map(|(column, _)| {
                let filter = effective_filter(host, &runtime, Some(column)).ok()?;
                Some((
                    column.clone(),
                    !runtime.facet_domain.contains_key(column),
                    filter,
                ))
            })
            .collect();
        (runtime.facet_generation, plan)
    };

    // A facet lists the values of the rows the page shows, not of the source.
    let domain = page_filter(host).ok().flatten();
    let host = host.clone();
    spawn_local(async move {
        let mut queries = 0usize;
        for (column, needs_domain, filter) in plan {
            if needs_domain {
                let query = grouping::group_query_json(&source, &column, domain.as_ref(), &[]);
                queries += 1;
                let Ok(result) = ask_uncounted(&host, &provider, &query, &mode).await else {
                    continue;
                };
                // A round a newer one replaced may come from another source.
                if grid_runtime.borrow().facet_generation != generation {
                    return;
                }
                if let Ok(domain) = grouping::groups_from(&result) {
                    grid_runtime
                        .borrow_mut()
                        .facet_domain
                        .insert(column.clone(), domain);
                }
            }
            let query = grouping::group_query_json(&source, &column, filter.as_ref(), &[]);
            queries += 1;
            let Ok(result) = ask_uncounted(&host, &provider, &query, &mode).await else {
                continue;
            };
            if grid_runtime.borrow().facet_generation != generation {
                return;
            }
            let counts = grouping::groups_from(&result)
                .unwrap_or_default()
                .into_iter()
                .map(|group| (group.key.to_string(), group.count))
                .collect();
            grid_runtime
                .borrow_mut()
                .facet_counts
                .insert(column, counts);
        }
        if grid_runtime.borrow().facet_generation != generation {
            return;
        }
        grid_runtime.borrow_mut().facet_queries = queries;
        if let Some(root) = host.shadow_root() {
            draw_facets(&host, &root);
        }
    });
}

/// A facet value as the reader sees it — the group-key rule of point 62:
/// NULL and the empty string are two values with two names.
fn facet_label(
    host: &HtmlElement,
    column: &str,
    value: &Value,
    schema: &opengrid_types::Schema,
) -> String {
    grid::group_value_text(
        value,
        column,
        schema.fields(),
        &texts(host),
        &formatter(host, schema),
    )
}

/// What a facet's chip says.
pub(super) fn facet_chip_text(
    host: &HtmlElement,
    column: &str,
    selection: &crate::facets::Selection,
    schema: &opengrid_types::Schema,
    domain: Option<&Vec<grouping::Group>>,
) -> String {
    use crate::facets::Selection;
    let texts = texts(host);
    match selection {
        Selection::Values(keys) => {
            let labels: Vec<String> = keys
                .iter()
                .map(|key| {
                    // The typed value comes from the facet's own list: a key
                    // is wire JSON, and JSON alone does not say its type.
                    let value = domain
                        .and_then(|domain| domain.iter().find(|group| &group.key == key))
                        .map(|group| group.value.clone())
                        .unwrap_or_else(|| match key {
                            opengrid_json::Json::Null => Value::Null,
                            opengrid_json::Json::String(text) => Value::Utf8(text.clone()),
                            other => Value::Utf8(other.to_string()),
                        });
                    facet_label(host, column, &value, schema)
                })
                .collect();
            if labels.len() == 1 {
                let is = texts.operator(4, "eq");
                format!("{} {is} {}", texts.column(column), labels[0])
            } else {
                texts.facet_chip_values(column, &labels.join(", "))
            }
        }
        Selection::Range {
            min: low,
            max: high,
        }
        | Selection::Period {
            from: low,
            to: high,
        } => {
            format!(
                "{} {} \u{2013} {}",
                texts.column(column),
                if low.trim().is_empty() {
                    "\u{2026}"
                } else {
                    low.trim()
                },
                if high.trim().is_empty() {
                    "\u{2026}"
                } else {
                    high.trim()
                }
            )
        }
    }
}

/// Draws the sidebar. The structure is rebuilt only when what it lists
/// changed; checked states, counts and the cost line are updated in place —
/// a rebuild after every count round would take the focus off the box the
/// reader just ticked.
/// Writes the footer's source (issue #33): the provider's `kind` in words and
/// the queries asked for what is shown — the rows' or groups', and the facets'.
pub(super) fn show_source(host: &HtmlElement) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let Ok(Some(node)) = root.query_selector("[part=\"source\"]") else {
        return;
    };
    let queries = {
        let borrowed = runtime.borrow();
        // The facets' counts are asked for only while the sidebar is there.
        let facets = if host.has_attribute(grid::FACETS_ATTRIBUTE) {
            borrowed.facet_queries
        } else {
            0
        };
        borrowed.queries + facets
    };
    let text = if queries == 0 {
        String::new()
    } else {
        let kind = provider(host).and_then(|provider| provider.kind());
        texts(host).source(kind.as_deref(), queries)
    };
    if node.text_content().as_deref() != Some(text.as_str()) {
        node.set_text_content(Some(&text));
    }
}

pub(super) fn draw_facets(host: &HtmlElement, root: &ShadowRoot) {
    show_source(host);
    let Ok(Some(sidebar)) = root.query_selector("[part=\"facets\"]") else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let configured = presentation::styles(host).facets();
    {
        let mut borrowed = runtime.borrow_mut();
        if borrowed.facets.len() != configured.len() {
            sync_facets(host, &mut borrowed);
        }
    }
    let borrowed = runtime.borrow();
    let schema = borrowed.state.schema().clone();
    let texts = texts(host);

    let signature = configured
        .iter()
        .map(|(column, kind)| {
            let keys = borrowed
                .facet_domain
                .get(column)
                .map(|domain| {
                    domain
                        .iter()
                        .map(|group| group.key.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            format!("{column}:{}:{keys}", kind.as_str())
        })
        .collect::<Vec<_>>()
        .join("|");
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let new = |tag: &str| document.create_element(tag).ok();

    if sidebar.get_attribute("data-drawn").as_deref() != Some(signature.as_str()) {
        let _ = sidebar.set_attribute("data-drawn", &signature);
        sidebar.set_inner_html("");

        if let Some(head) = new("div") {
            let _ = head.set_attribute("part", "facets-head");
            if let Some(cost) = new("span") {
                let _ = cost.set_attribute("part", "facet-cost");
                // Our sentence around a number — nothing of the page's in it.
                if !texts.lang.trim().is_empty() {
                    let _ = cost.set_attribute("lang", &texts.lang);
                }
                let _ = head.append_child(&cost);
            }
            if let Some(reset) = new("button") {
                let _ = reset.set_attribute("type", "button");
                let _ = reset.set_attribute("data-facets-reset", "");
                if !texts.lang.trim().is_empty() {
                    let _ = reset.set_attribute("lang", &texts.lang);
                }
                reset.set_text_content(Some(&texts.facets_reset));
                let _ = head.append_child(&reset);
            }
            let _ = sidebar.append_child(&head);
        }

        for (column, kind) in &configured {
            let Some(fieldset) = new("fieldset") else {
                continue;
            };
            let _ = fieldset.set_attribute("part", "facet");
            let _ = fieldset.set_attribute("data-facet", column);
            if let Some(legend) = new("legend") {
                legend.set_text_content(Some(texts.column(column)));
                let _ = fieldset.append_child(&legend);
            }
            match kind {
                presentation::FacetKind::List => {
                    for group in borrowed.facet_domain.get(column).into_iter().flatten() {
                        let Some(label) = new("label") else { continue };
                        let _ = label.set_attribute("part", "facet-value");
                        if let Some(input) = new("input") {
                            let _ = input.set_attribute("type", "checkbox");
                            let _ = input.set_attribute("data-facet", column);
                            let _ = input.set_attribute("data-key", &group.key.to_string());
                            let _ = label.append_child(&input);
                        }
                        if let Some(name) = new("span") {
                            name.set_text_content(Some(&facet_label(
                                host,
                                column,
                                &group.value,
                                &schema,
                            )));
                            let _ = label.append_child(&name);
                        }
                        if let Some(count) = new("span") {
                            let _ = count.set_attribute("part", "facet-count");
                            let _ = label.append_child(&count);
                        }
                        let _ = fieldset.append_child(&label);
                    }
                }
                presentation::FacetKind::Pills => {
                    let Some(pills) = new("div") else { continue };
                    let _ = pills.set_attribute("part", "facet-pills");
                    for group in borrowed.facet_domain.get(column).into_iter().flatten() {
                        let Some(pill) = new("button") else { continue };
                        let _ = pill.set_attribute("type", "button");
                        let _ = pill.set_attribute("part", "facet-pill");
                        let _ = pill.set_attribute("data-facet", column);
                        let _ = pill.set_attribute("data-key", &group.key.to_string());
                        let _ = pill.set_attribute("aria-pressed", "false");
                        if let Some(name) = new("span") {
                            name.set_text_content(Some(&facet_label(
                                host,
                                column,
                                &group.value,
                                &schema,
                            )));
                            let _ = pill.append_child(&name);
                        }
                        if let Some(count) = new("span") {
                            let _ = count.set_attribute("part", "facet-count");
                            let _ = pill.append_child(&count);
                        }
                        let _ = pills.append_child(&pill);
                    }
                    let _ = fieldset.append_child(&pills);
                }
                presentation::FacetKind::Range | presentation::FacetKind::Period => {
                    let Some(bounds) = new("div") else { continue };
                    let _ = bounds.set_attribute("part", "facet-bounds");
                    for (bound, word) in [("low", &texts.facet_from), ("high", &texts.facet_to)] {
                        let Some(label) = new("label") else { continue };
                        // A visible word, not a placeholder: "min" in an empty
                        // field is gone the moment someone types (3.3.2).
                        if let Some(caption) = new("span") {
                            caption.set_text_content(Some(word));
                            if !texts.lang.trim().is_empty() {
                                let _ = caption.set_attribute("lang", &texts.lang);
                            }
                            let _ = label.append_child(&caption);
                        }
                        if let Some(input) = new("input") {
                            let date = *kind == presentation::FacetKind::Period;
                            let _ = input.set_attribute("type", if date { "date" } else { "text" });
                            if !date {
                                let _ = input.set_attribute("inputmode", "decimal");
                            }
                            let _ = input.set_attribute("data-facet", column);
                            let _ = input.set_attribute("data-bound", bound);
                            let _ = label.append_child(&input);
                        }
                        let _ = bounds.append_child(&label);
                    }
                    let _ = fieldset.append_child(&bounds);
                }
            }
            let _ = sidebar.append_child(&fieldset);
        }
    }

    // States, counts, the cost line — in place.
    if let Ok(Some(cost)) = sidebar.query_selector("[part=\"facet-cost\"]") {
        cost.set_text_content(Some(&texts.facet_queries(borrowed.facet_queries)));
    }
    let focused = root.active_element();
    for (column, selection) in &borrowed.facets {
        let counts = borrowed.facet_counts.get(column);
        let Ok(items) = sidebar.query_selector_all(&format!("[data-facet=\"{column}\"][data-key]"))
        else {
            continue;
        };
        for index in 0..items.length() {
            let Some(item) = items
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let key = item.get_attribute("data-key").unwrap_or_default();
            let parsed = opengrid_json::Json::parse(&key).ok();
            let chosen = matches!(selection, crate::facets::Selection::Values(values)
                if values.iter().any(|value| Some(value) == parsed.as_ref()));
            if let Ok(input) = item.clone().dyn_into::<HtmlInputElement>() {
                input.set_checked(chosen);
            } else {
                let _ = item.set_attribute("aria-pressed", &chosen.to_string());
            }
            let count = counts
                .and_then(|counts| counts.get(&key))
                .copied()
                .unwrap_or(0);
            let holder = item
                .closest("[part=\"facet-value\"]")
                .ok()
                .flatten()
                .unwrap_or(item.clone());
            if let Ok(Some(number)) = holder.query_selector("[part=\"facet-count\"]") {
                number.set_text_content(Some(&count.to_string()));
            }
        }
        let (low, high) = match selection {
            crate::facets::Selection::Range { min, max } => (min.clone(), max.clone()),
            crate::facets::Selection::Period { from, to } => (from.clone(), to.clone()),
            crate::facets::Selection::Values(_) => continue,
        };
        for (bound, value) in [("low", low), ("high", high)] {
            if let Ok(Some(node)) =
                sidebar.query_selector(&format!("input[data-facet=\"{column}\"][data-bound=\"{bound}\"]"))
                && let Ok(input) = node.dyn_into::<HtmlInputElement>()
                // Never overwrite the field someone is typing in.
                && focused.as_ref().is_none_or(|focused| !focused.is_same_node(Some(input.as_ref())))
            {
                input.set_value(&value);
            }
        }
    }
}

/// Applies the facets: the rows change, so the window starts over and the
/// selection goes — the same rule a filter follows (point 35).
pub(super) fn apply_facets(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let pool = pool_of(host);
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.state.invalidate_selection();
        borrowed.state.set_window(Window::new(0, pool));
    }
    announce_if_cleared(host, &runtime);
    run_query(host, QueryKind::Data, false);
    dispatch_view(host);
}

/// A list checkbox or a bound field changed.
pub(super) fn on_facet_change(host: &HtmlElement, target: &Element) {
    let Some(column) = target.get_attribute("data-facet") else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    if let Some(key) = target.get_attribute("data-key") {
        toggle_key(&runtime, &column, &key);
    } else if let Some(bound) = target.get_attribute("data-bound")
        && let Ok(input) = target.clone().dyn_into::<HtmlInputElement>()
    {
        let value = input.value();
        for (name, selection) in runtime.borrow_mut().facets.iter_mut() {
            if name != &column {
                continue;
            }
            match selection {
                crate::facets::Selection::Range {
                    min: low,
                    max: high,
                }
                | crate::facets::Selection::Period {
                    from: low,
                    to: high,
                } => {
                    if bound == "low" {
                        *low = value.clone();
                    } else {
                        *high = value.clone();
                    }
                }
                crate::facets::Selection::Values(_) => {}
            }
        }
    }
    apply_facets(host);
}

/// A pill was pressed.
pub(super) fn toggle_facet_value(host: &HtmlElement, pill: &Element) {
    let (Some(column), Some(key)) = (
        pill.get_attribute("data-facet"),
        pill.get_attribute("data-key"),
    ) else {
        return;
    };
    if let Some(runtime) = runtime(host) {
        toggle_key(&runtime, &column, &key);
    }
    apply_facets(host);
}

fn toggle_key(runtime: &Rc<RefCell<GridRuntime>>, column: &str, key: &str) {
    let Ok(value) = opengrid_json::Json::parse(key) else {
        return;
    };
    for (name, selection) in runtime.borrow_mut().facets.iter_mut() {
        if name == column
            && let crate::facets::Selection::Values(values) = selection
        {
            if let Some(at) = values.iter().position(|chosen| chosen == &value) {
                values.remove(at);
            } else {
                values.push(value.clone());
            }
        }
    }
}

/// Clears every facet.
pub(super) fn reset_facets(host: &HtmlElement) {
    if let Some(runtime) = runtime(host) {
        for (_, selection) in runtime.borrow_mut().facets.iter_mut() {
            *selection = clear_selection(selection);
        }
    }
    apply_facets(host);
}
