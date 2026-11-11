//! The grid grouped by a column: its groups, the rows of the window, and opening
//! and closing a group (issues #33, #321).
//!
//! The start of a run, the line that says "loading" and the sentence a toggle
//! says are shared with the tree path and live in the parent module.

use super::*;

/// The grouped counterpart of [`run_query`]: several queries, one result.
///
/// 1. the level-1 groups with their row counts — only when they are not loaded
///    yet or the filter changed;
/// 2. the groups under every open group that shows and has none yet;
/// 3. the rows of the window, **one query per group it touches**, each filtered
///    on the group key and paged with `offset`/`limit`.
///
/// The answers are assembled into one ordinary `QueryResult` whose rows are the
/// window's display positions — group headers carry NULLs, which the renderer
/// never shows — and whose `total_count` is the display list's length. From
/// there it takes the same path as every other result: [`settle`], one render.
pub(super) fn run_grouped(
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

    let (mut sorts, offset, need_groups, choice) = {
        let mut runtime = grid_runtime.borrow_mut();
        // Another filter: what is selected by key stays (issue #322), but
        // need not be among the rows any more.
        if runtime
            .groups_filter
            .as_ref()
            .is_some_and(|asked| asked != &filter)
        {
            runtime.picked.rows_changed();
        }
        let need_groups = runtime.groups_filter.as_ref() != Some(&filter)
            || !runtime.grouping.as_ref().is_some_and(Grouping::is_loaded);
        (
            runtime.state.sort_keys(),
            runtime.state.window().offset,
            need_groups,
            runtime.aggregate_choice.clone(),
        )
    };
    // The rows inside a group are paged with `offset`, and paging needs a total
    // order (S6).
    if sorts.is_empty()
        && let Some(first) = columns.first()
    {
        sorts.push((first.clone(), "asc"));
    }
    announce_loading(host, grid_runtime, kind);

    let host = host.clone();
    let grid_runtime = grid_runtime.clone();
    spawn_local(async move {
        let stale = |runtime: &Rc<RefCell<GridRuntime>>| superseded(runtime, generation, focus);
        let fail =
            |message: String| settle(&host, generation, Err(message), focus, QueryKind::Data);

        // The typed schema, when the groups are asked again: which aggregates
        // a column allows depends on its type, and the display schema of
        // point 23 calls every column text.
        let mut typed: Option<opengrid_types::Schema> = None;

        // 1. The level-1 groups, their aggregates, and the grand total.
        if need_groups {
            let Some(by) = grid_runtime
                .borrow()
                .grouping
                .as_ref()
                .map(|grouping| grouping.by()[0].clone())
            else {
                return;
            };
            let probe = grid::query_json(&source, &columns, &sorts, filter.as_ref(), 0, 0);
            let schema = match ask(&host, &provider, &grid_runtime, &probe, &mode).await {
                Ok(result) => result.schema,
                Err(message) => return fail(message),
            };
            if stale(&grid_runtime) {
                return;
            }
            let (aggregates, refused) = effective_aggregates(&host, &schema, &choice);
            typed = Some(schema);
            {
                let mut runtime = grid_runtime.borrow_mut();
                if !refused.is_empty() {
                    runtime.state.set_notice(refused.join(" "));
                }
                if let Some(grouping) = runtime.grouping.as_mut() {
                    grouping.set_aggregates(aggregates.clone());
                }
            }

            let query = grouping::group_query_json(&source, &by, filter.as_ref(), &aggregates);
            let result = match ask(&host, &provider, &grid_runtime, &query, &mode).await {
                Ok(result) => result,
                Err(message) => return fail(message),
            };
            if stale(&grid_runtime) {
                return;
            }
            // The type of the key is only known now. A key that does not
            // repeat is refused here, said once, and the grid falls back to
            // ungrouped rather than to nothing.
            if let Some(field) = result.schema.fields().first()
                && !grouping::groupable(field.data_type)
            {
                let message = texts(&host).group_invalid(field.name.as_str());
                {
                    let mut runtime = grid_runtime.borrow_mut();
                    runtime.grouping = None;
                    runtime.state.set_notice(message);
                }
                return run_query(&host, QueryKind::Data, focus);
            }
            let groups = match grouping::groups_from(&result) {
                Ok(groups) => groups,
                Err(message) => return fail(message),
            };
            {
                let mut runtime = grid_runtime.borrow_mut();
                if let Some(grouping) = runtime.grouping.as_mut() {
                    grouping.set_groups(groups);
                }
            }

            // The grand total: the same aggregates, no `group` — one row.
            let query = grouping::total_query_json(&source, filter.as_ref(), &aggregates);
            let total = match ask(&host, &provider, &grid_runtime, &query, &mode).await {
                Ok(result) => grouping::total_from(&result),
                Err(message) => return fail(message),
            };
            if stale(&grid_runtime) {
                return;
            }
            {
                // Only now are the groups asked for this filter complete: a
                // newer query in between must ask the total again.
                let mut runtime = grid_runtime.borrow_mut();
                runtime.groups_filter = Some(filter.clone());
                if let Some(grouping) = runtime.grouping.as_mut() {
                    grouping.set_total(total);
                }
            }
        }

        // 2. The groups under open groups that have none yet — a level per
        //    pass, because what a pass loads can be open itself (a view).
        for _ in 1..grouping::MAX_LEVELS {
            let missing = match grid_runtime.borrow().grouping.as_ref() {
                Some(grouping) => grouping.missing_children(),
                None => return,
            };
            for keys in missing {
                let (column, scoped, aggregates) = {
                    let runtime = grid_runtime.borrow();
                    let Some(grouping) = runtime.grouping.as_ref() else {
                        return;
                    };
                    (
                        grouping.by()[keys.len()].clone(),
                        grouping.filter_for(&keys, filter.as_ref()),
                        grouping.aggregates().to_vec(),
                    )
                };
                let query =
                    grouping::group_query_json(&source, &column, scoped.as_ref(), &aggregates);
                let result = match ask(&host, &provider, &grid_runtime, &query, &mode).await {
                    Ok(result) => result,
                    Err(message) => return fail(message),
                };
                if stale(&grid_runtime) {
                    return;
                }
                let children = match grouping::groups_from(&result) {
                    Ok(children) => children,
                    Err(message) => return fail(message),
                };
                if let Some(grouping) = grid_runtime.borrow_mut().grouping.as_mut() {
                    grouping.set_children(&keys, children);
                }
            }
        }

        // 3. The rows of the window, one query per group it touches.
        let (fetches, total, offset, settled_as) = {
            let mut runtime = grid_runtime.borrow_mut();
            let Some(grouping) = runtime.grouping.as_ref() else {
                return;
            };
            let total = grouping.len();
            let settled_as = settled_kind(kind, runtime.state.total_count(), total);
            // A toggle can shorten the list under the window; keep the window
            // inside it, the way the scroll math clamps an ordinary one.
            let offset = grid::clamp_window_offset(offset, total, pool);
            let fetches = grouping.fetches(offset, pool);
            runtime.state.set_window(Window::new(offset, pool));
            (fetches, total, offset, settled_as)
        };

        let mut schema: Option<opengrid_types::Schema> = None;
        let mut rows: Vec<(u64, Vec<Vec<Value>>)> = Vec::new();
        for fetch in &fetches {
            let scoped = grid_runtime
                .borrow()
                .grouping
                .as_ref()
                .and_then(|grouping| grouping.filter_for(&fetch.keys, filter.as_ref()));
            let query = grid::query_json(
                &source,
                &columns,
                &sorts,
                scoped.as_ref(),
                fetch.offset,
                fetch.limit,
            );
            let result = match ask(&host, &provider, &grid_runtime, &query, &mode).await {
                Ok(result) => result,
                Err(message) => return fail(message),
            };
            if stale(&grid_runtime) {
                return;
            }
            schema.get_or_insert_with(|| result.schema.clone());
            rows.push((fetch.position, result.columns));
        }
        // No open group in the window: the typed schema still has to arrive,
        // or the header would keep the display schema's all-`Utf8` types.
        let schema = match schema.or(typed) {
            Some(schema) => schema,
            None => {
                let query = grid::query_json(&source, &columns, &sorts, filter.as_ref(), 0, 0);
                match ask(&host, &provider, &grid_runtime, &query, &mode).await {
                    Ok(result) => result.schema,
                    Err(message) => return fail(message),
                }
            }
        };
        if stale(&grid_runtime) {
            return;
        }

        // Assemble: one column-major page over the window's display positions.
        let width = schema.fields().len();
        let height = total.saturating_sub(offset).min(pool) as usize;
        let mut page: Vec<Vec<Value>> = vec![vec![Value::Null; height]; width];
        for (position, columns) in rows {
            let first = (position - offset) as usize;
            for (col, values) in columns.into_iter().enumerate().take(width) {
                for (index, value) in values.into_iter().enumerate() {
                    if let Some(slot) = page[col].get_mut(first + index) {
                        *slot = value;
                    }
                }
            }
        }
        let result = opengrid_datasource::QueryResult::new(schema, page, total);
        settle(&host, generation, Ok(result), focus, settled_as);
    });
}

/// Opens or closes the group whose header is at `position`.
///
/// The focus stays where it is: a group's own position does not move when it
/// opens — only what comes after it does. The change is said once, riding with
/// the result that follows (`set_notice`), and the window is re-asked without
/// the "loading" line, so the reader hears the group, not the machinery.
pub(super) fn toggle_group(host: &HtmlElement, position: u64) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let message = {
        let mut borrowed = runtime.borrow_mut();
        let schema = borrowed.state.schema().clone();
        let Some(grouping) = borrowed.grouping.as_mut() else {
            return;
        };
        let Some(grouping::Item::Group {
            keys,
            value,
            count,
            level,
            ..
        }) = grouping.item_at(position)
        else {
            return;
        };
        let open = grouping.toggle(&keys);
        let column = grouping.by()[level - 1].clone();
        toggled_message(host, &schema, &value, &column, count, open)
    };
    announce_toggle(host, &runtime, message);
}

/// The display position of the group header the active cell stands on, if any.
pub(super) fn active_group(host: &HtmlElement) -> Option<(u64, bool)> {
    let runtime = runtime(host)?;
    let borrowed = runtime.borrow();
    let row = borrowed.active.row()?;
    match borrowed.grouping.as_ref()?.item_at(row)? {
        grouping::Item::Group { expanded, .. } => Some((row, expanded)),
        // The total opens nothing; `Enter` on it is simply not a toggle.
        grouping::Item::Row { .. } | grouping::Item::Total { .. } => None,
    }
}
