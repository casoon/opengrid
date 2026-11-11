//! The grid as a tree: its levels, the window cut from them, and opening and
//! closing a node (issue #135).
//!
//! The start of a run, the line that says "loading" and the sentence a toggle
//! says are shared with the grouped path and live in the parent module.

use super::*;

/// The tree's counterpart of [`run_grouped`] (issue #135, plan point 123).
///
/// 1. the roots, when they are not loaded or the filter or order changed;
/// 2. the children of every open node that is shown and has none yet — a
///    level at a time until nothing open is missing, so a restored view opens
///    nodes deep down;
/// 3. no query for the window: every shown row is in a loaded level, and the
///    page is cut from them.
///
/// Each level is asked whole, at most [`tree::MAX_LEVEL`] siblings; a larger
/// one is a filter's job and said so. The result goes the way of every other
/// one: [`settle`], one render.
pub(super) fn run_tree(
    host: &HtmlElement,
    grid_runtime: &Rc<RefCell<GridRuntime>>,
    kind: QueryKind,
    focus: bool,
) {
    let Some(RunStart {
        provider,
        source,
        columns,
        pool,
        mode,
        filter,
        generation,
    }) = begin_run(host, grid_runtime)
    else {
        return;
    };
    let (sorts, offset, key, parent, choice) = {
        let mut runtime = grid_runtime.borrow_mut();
        let sorts = runtime.state.sort_keys();
        // The levels hold the rows of one filter in one order; under another
        // they are asked again, and what is open stays open.
        let asked = format!(
            "{} {sorts:?}",
            filter
                .as_ref()
                .map(|filter| opengrid_json::ToJson::to_json(filter).to_string())
                .unwrap_or_default()
        );
        if runtime.tree_asked.as_ref() != Some(&asked) {
            if let Some(tree) = runtime.tree.as_mut() {
                tree.invalidate();
            }
            runtime.tree_asked = Some(asked);
        }
        let Some(tree) = runtime.tree.as_ref() else {
            return;
        };
        let (key, parent) = (tree.key().to_owned(), tree.parent().to_owned());
        (
            sorts,
            runtime.state.window().offset,
            key,
            parent,
            runtime.aggregate_choice.clone(),
        )
    };
    // T7: whether any shown column asks for a summary — only then are the
    // types probed, which the summaries a column allows depend on.
    let wants_aggregates = !choice.is_empty() || {
        let configured = presentation::styles(host);
        columns.iter().any(|column| {
            configured
                .get(column)
                .is_some_and(|c| c.aggregate.is_some())
        })
    };
    // Types this grid already learned from an answer spare the probe — the
    // tree path runs on every scroll.
    let learned = {
        let runtime = grid_runtime.borrow();
        columns
            .iter()
            .all(|column| runtime.known.contains_key(column))
            .then(|| grid::known_schema(&columns, &runtime.known))
    };
    announce_loading(host, grid_runtime, kind);

    // The key rides along when the grid does not show it: a node is named by
    // it, and its children are asked with it.
    let width = columns.len();
    let mut select = columns.clone();
    let key_column = match select.iter().position(|column| *column == key) {
        Some(at) => at,
        None => {
            select.push(key.clone());
            width
        }
    };

    let host = host.clone();
    let grid_runtime = grid_runtime.clone();
    spawn_local(async move {
        let stale = |runtime: &Rc<RefCell<GridRuntime>>| superseded(runtime, generation, focus);
        let fail =
            |message: String| settle(&host, generation, Err(message), focus, QueryKind::Data);

        let mut schema: Option<opengrid_types::Schema> = None;

        // The subtree summaries (T7), checked against the real types as a
        // grouping's are: a probe tells them, a summary the type cannot take
        // is said and left out.
        let (aggregates, refused) = if wants_aggregates {
            let typed = match learned {
                Some(schema) => schema,
                None => {
                    let probe = grid::query_json(&source, &columns, &sorts, filter.as_ref(), 0, 0);
                    let result = match ask(&host, &provider, &grid_runtime, &probe, &mode).await {
                        Ok(result) => result,
                        Err(message) => return fail(message),
                    };
                    if stale(&grid_runtime) {
                        return;
                    }
                    result.schema
                }
            };
            effective_aggregates(&host, &typed, &choice)
        } else {
            (Vec::new(), Vec::new())
        };
        {
            let mut runtime = grid_runtime.borrow_mut();
            let runtime = &mut *runtime;
            let Some(tree) = runtime.tree.as_mut() else {
                return;
            };
            // Other summaries than the levels were asked with load them again.
            tree.set_aggregates(aggregates.clone());
            // Said with a reload, not with every scroll that runs this path.
            if !tree.is_loaded() && !refused.is_empty() {
                runtime.state.set_notice(refused.join(" "));
            }
        }

        loop {
            // The roots first; then the first open node still missing.
            let under = {
                let runtime = grid_runtime.borrow();
                let Some(tree) = runtime.tree.as_ref() else {
                    return;
                };
                if !tree.is_loaded() {
                    None
                } else {
                    match tree.missing().into_iter().next() {
                        Some(key) => Some(key),
                        None => break,
                    }
                }
            };
            let query = grid::tree_query_json(
                &source,
                &select,
                &sorts,
                filter.as_ref(),
                tree::tree_part(&key, &parent, under.as_ref(), &aggregates),
                tree::MAX_LEVEL,
            );
            let result = match ask(&host, &provider, &grid_runtime, &query, &mode).await {
                Ok(result) => result,
                Err(message) => return fail(message),
            };
            if stale(&grid_runtime) {
                return;
            }
            if result.total_count > tree::MAX_LEVEL {
                return fail(
                    texts(&host).tree_level_too_large(result.total_count, tree::MAX_LEVEL),
                );
            }
            let level = match tree::Level::from_result(&result, width, key_column) {
                Ok(level) => level,
                Err(message) => return fail(message),
            };
            schema.get_or_insert_with(|| {
                opengrid_types::Schema::new(
                    result.schema.fields().iter().take(width).cloned().collect(),
                )
            });
            let (matches, orphans) = result
                .tree
                .as_ref()
                .map_or((0, 0), |tree| (tree.matches, tree.orphans));
            let mut runtime = grid_runtime.borrow_mut();
            let runtime = &mut *runtime;
            let Some(tree) = runtime.tree.as_mut() else {
                return;
            };
            // Orphans stand at the top (T2): said when there are some, and
            // not again for the same count under the next filter.
            if under.is_none() && orphans > 0 && orphans != tree.orphans() {
                runtime.state.add_notice(texts(&host).tree_orphans(orphans));
            }
            tree.set_level(
                under.as_ref().map(|key| key.to_string()),
                level,
                matches,
                orphans,
            );
        }

        // The window, cut from the loaded levels.
        let (result, settled_as) = {
            let mut runtime = grid_runtime.borrow_mut();
            let schema = schema.unwrap_or_else(|| runtime.state.schema().clone());
            let Some(tree) = runtime.tree.as_ref() else {
                return;
            };
            let total = tree.len();
            let settled_as = settled_kind(kind, runtime.state.total_count(), total);
            // A node closed under the window shortens the list; the window
            // stays inside it, as with a group.
            let offset = grid::clamp_window_offset(offset, total, pool);
            let page = tree.page(offset, pool, width);
            // The selection by key, at the positions its nodes sit at now.
            let selected = tree.selected_positions();
            runtime.state.set_window(Window::new(offset, pool));
            runtime.state.set_selection(selected);
            (
                opengrid_datasource::QueryResult::new(schema, page, total),
                settled_as,
            )
        };
        settle(&host, generation, Ok(result), focus, settled_as);
    });
}

/// Opens or closes the tree's node at `position` — the counterpart of
/// [`toggle_group`]: the focus stays, the change is said once with the
/// result, and only what is missing is asked.
pub(super) fn toggle_tree(host: &HtmlElement, position: u64) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let message = {
        let mut borrowed = runtime.borrow_mut();
        let schema = borrowed.state.schema().clone();
        let Some(tree) = borrowed.tree.as_mut() else {
            return;
        };
        let Some(open) = tree.toggle(position) else {
            return;
        };
        let Some(count) = tree.entry_at(position).map(|entry| entry.children) else {
            return;
        };
        let value = tree
            .page(position, 1, 1)
            .into_iter()
            .next()
            .and_then(|values| values.into_iter().next())
            .unwrap_or(Value::Null);
        let column = columns_of(host).first().cloned().unwrap_or_default();
        toggled_message(host, &schema, &value, &column, count, open)
    };
    announce_toggle(host, &runtime, message);
}

/// The display position of the tree's node the active cell stands on, and
/// what it is.
pub(super) fn active_tree(host: &HtmlElement) -> Option<(u64, tree::Entry)> {
    let runtime = runtime(host)?;
    let borrowed = runtime.borrow();
    let row = borrowed.active.row()?;
    let entry = borrowed.tree.as_ref()?.entry_at(row)?.clone();
    Some((row, entry))
}
