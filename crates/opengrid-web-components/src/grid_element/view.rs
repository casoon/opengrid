//! The view as a value: what the grid shows and how it is sorted, filtered, grouped and picked, read out as JSON and restored from it.

use super::*;

thread_local! {
    /// True while [`write_view`] is applying a view.
    ///
    /// The attribute callback of `density` runs its own query, and a view sets
    /// the density on its way to setting everything else. Without this guard a
    /// restore would fire two queries and announce two results — the second one
    /// overwriting the first before anyone heard it, which is exactly the class
    /// of bug `announcements.spec.js` exists to catch.
    static APPLYING_VIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a view is being applied right now.
pub(super) fn applying_view() -> bool {
    APPLYING_VIEW.with(std::cell::Cell::get)
}

/// Gathers the view from the four places it actually lives.
pub(crate) fn current_view(host: &HtmlElement) -> Option<GridView> {
    let runtime = runtime(host)?;
    let root = host.shadow_root();
    let columns = columns_of(host);

    let sort = runtime
        .borrow()
        .state
        .sort_keys()
        .into_iter()
        .map(|(field, direction)| (field, direction.to_owned()))
        .collect();

    // Only the columns that carry a filter: a view is what the reader chose,
    // and an empty operator on every column is not a choice.
    let filters = root
        .as_ref()
        .map(|root| read_filter_entries(root, &columns))
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| !grid::takes_value(entry.op.as_str()) || !entry.value.trim().is_empty())
        .collect();

    let layout = columns::layout(host).borrow().clone();
    let density = grid::density_of(host.get_attribute(grid::DENSITY_ATTRIBUTE).as_deref())
        .0
        .to_owned();

    let (group, expanded) = {
        let borrowed = runtime.borrow();
        match (borrowed.grouping.as_ref(), borrowed.tree.as_ref()) {
            (Some(grouping), _) => (grouping.by().to_vec(), grouping.expanded()),
            // A tree's open nodes, each `[key]` (issue #135).
            (None, Some(tree)) => (Vec::new(), tree.expanded()),
            (None, None) => (Vec::new(), Vec::new()),
        }
    };

    let aggregates = runtime
        .borrow()
        .aggregate_choice
        .iter()
        .map(|(column, function)| (column.clone(), function.as_str().to_owned()))
        .collect();

    let filter_row = runtime.borrow().filter_row;
    let facets = crate::facets::to_json(&runtime.borrow().facets);

    // The selection, where it names records (issue #322) — a view just set
    // holds its own until the source has been asked about it.
    let selected = {
        let borrowed = runtime.borrow();
        match (&borrowed.restore, &borrowed.tree) {
            (Some(restore), _) => restore.keys.clone(),
            (None, Some(tree)) => tree.selected_keys(),
            (None, None) => borrowed.picked.keys(),
        }
    };
    let selected = opengrid_grid::grid_view::selected_keys(selected);
    // The open panels (issue #318), and the ones a restored view still waits
    // for their rows to load.
    let detail = {
        let borrowed = runtime.borrow();
        opengrid_grid::grid_view::selected_keys(
            borrowed
                .details
                .iter()
                .map(|opened| opened.key.clone())
                .chain(borrowed.detail_wanted.iter().cloned()),
        )
    };
    let nested = runtime
        .borrow()
        .nested
        .as_ref()
        .map(opengrid_json::ToJson::to_json);

    Some(GridView {
        sort,
        filters,
        columns: layout,
        density,
        group,
        expanded,
        aggregates,
        filter_row,
        facets,
        selected,
        detail,
        nested,
    })
}

/// [`crate::element::get_query`] — the query of the view, as a JS object.
pub(crate) fn read_query(host: &HtmlElement) -> JsValue {
    match current_query(host) {
        Some(json) => js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL),
        None => JsValue::NULL,
    }
}

/// [`crate::element::prepare_print`] — the print table of the view.
pub(crate) fn prepare_print(host: &HtmlElement) -> js_sys::Promise {
    print::prepare(host)
}

/// The query of the current view without a window (plan point 82).
///
/// Built from the same parts as [`run_query`] — so it cannot say something the
/// grid does not ask — with one addition for a grouped grid: the rows come in
/// the order the reader sees them, by their groups first (ascending, NULL last,
/// as [`grouping::group_query_json`] orders the groups), then by the sort.
pub(super) fn current_query(host: &HtmlElement) -> Option<String> {
    if host.tag_name().to_ascii_lowercase() != GRID_TAG {
        return None;
    }
    let runtime = runtime(host)?;
    let source = host.get_attribute(DATASOURCE_ATTRIBUTE)?;
    let columns = columns_of(host);
    if columns.is_empty() {
        return None;
    }
    let borrowed = runtime.borrow();
    let filter = effective_filter(host, &borrowed, None).ok()?;
    // A tree is not grouped (T10); it is exported whole and flat (T8, #166).
    let groups: Vec<String> = match borrowed.tree {
        Some(_) => Vec::new(),
        None => borrowed
            .grouping
            .as_ref()
            .map(|grouping| grouping.by().to_vec())
            .unwrap_or_default(),
    };
    let mut sorts = borrowed.state.sort_keys();
    // A sort key (S6), as the grid itself pages under one.
    if groups.is_empty() && sorts.is_empty() {
        sorts.push((columns[0].clone(), "asc"));
    }
    let query = grid::view_query_json(&source, &columns, &groups, &sorts, filter.as_ref());
    // The page's collation (S4a), as on every query the grid sends.
    let query = match super::facets::page_collation(host).ok()? {
        Some(locale) => {
            opengrid_grid::view_query::collated(&query, &locale, borrowed.state.schema())
                .unwrap_or(query)
        }
        None => query,
    };
    let Some(tree) = borrowed.tree.as_ref() else {
        return Some(query);
    };
    // Every node of the reader's tree, depth-first, each with its level and
    // path — not only the open ones (#166).
    let mut query = opengrid_json::Json::parse(&query).ok()?;
    query["tree"] = opengrid_json::json!({
        "key": tree.key(),
        "parent": tree.parent(),
        "flat": true,
    });
    Some(query.to_string())
}

/// [`crate::element::get_view`] — the view as a JS object.
pub(crate) fn read_view(host: &HtmlElement) -> JsValue {
    let Some(view) = current_view(host) else {
        return JsValue::NULL;
    };
    to_js(&view.to_json())
}

/// A `opengrid_json::Json` as a real JS value, through `JSON.parse`.
///
/// Not a bridge crate: the JSON codec is here already and this is the only
/// place that needs the conversion — a dependency for one conversion is not
/// one this project takes.
pub(super) fn to_js(value: &opengrid_json::Json) -> JsValue {
    js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL)
}

/// [`crate::element::set_view`] — applies a whole view in one query.
pub(crate) fn write_view(host: &HtmlElement, value: &JsValue, notice: Option<String>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Ok(text) = js_sys::JSON::stringify(value).map(String::from) else {
        return;
    };
    let Ok(json) = opengrid_json::Json::parse(&text) else {
        return;
    };

    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    let view = match GridView::from_json(&json, &declared) {
        // The selection in the one order a view holds it, so that the same
        // selection is the same view.
        Ok(mut view) => {
            view.selected = opengrid_grid::grid_view::selected_keys(view.selected);
            view.detail = opengrid_grid::grid_view::selected_keys(view.detail);
            view
        }
        Err(problems) => {
            // Named, not swallowed: a view saved against another data source
            // would otherwise leave a grid that looks restored and is not.
            let message = problems
                .iter()
                .map(|problem| format!("{}: {}", problem.field, problem.reason))
                .collect::<Vec<_>>()
                .join(" ");
            runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(message));
            render(host, false);
            return;
        }
    };

    // A literal a typed column cannot hold is said, and the view is not
    // applied — the same rule as an unknown column: a grid that looks restored
    // and is not is the worse failure (point 88). Only columns a result has
    // typed can be checked here; the others are checked when the types arrive.
    let checked = grid::known_schema(&declared, &runtime.borrow().known);
    if let Err(problems) = grid::filter_expr(&view.filters, &checked) {
        let texts = texts(host);
        let message = problems
            .iter()
            .map(|problem| texts.filter_invalid(&problem.column, &problem.value))
            .collect::<Vec<_>>()
            .join(" ");
        runtime
            .borrow_mut()
            .state
            .set_status(GridStatus::Error(message));
        render(host, false);
        return;
    }

    if current_view(host).as_ref() == Some(&view) {
        // Setting the view it already has is not a change, and must not cost a
        // query or an announcement.
        return;
    }
    apply_view(host, &runtime, &view, notice, false);
    dispatch_view(host);
}

/// Builds the grid around `view`, in one query. With `keep_search` the search
/// stays as it is: the view is the grid's own, arranged anew ([`rearrange`]),
/// not one a page handed in. The caller says whether the view changed
/// ([`dispatch_view`]).
pub(super) fn apply_view(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    view: &GridView,
    notice: Option<String>,
    keep_search: bool,
) {
    let search = host
        .shadow_root()
        .and_then(|root| search_input(&root))
        .filter(|_| keep_search)
        .map(|input| input.value());

    APPLYING_VIEW.with(|flag| flag.set(true));

    // Density first: it only changes geometry, and the rebuild below wants the
    // resolved row height. The attribute callback sees the guard and does not
    // run a query of its own.
    if host.get_attribute(grid::DENSITY_ATTRIBUTE).as_deref() != Some(view.density.as_str()) {
        let _ = host.set_attribute(grid::DENSITY_ATTRIBUTE, &view.density);
    }
    runtime.borrow_mut().row_height = resolve_row_height(host);

    // The grouping, the same way as the density: the attribute is set, its
    // callback sees the guard and does not query on its own.
    let group_by = view.group.join(",");
    if host
        .get_attribute(grid::GROUP_BY_ATTRIBUTE)
        .unwrap_or_default()
        != group_by
    {
        if group_by.is_empty() {
            let _ = host.remove_attribute(grid::GROUP_BY_ATTRIBUTE);
        } else {
            let _ = host.set_attribute(grid::GROUP_BY_ATTRIBUTE, &group_by);
        }
    }

    // The column layout decides which query is sent, so it is set before the
    // skeleton is rebuilt around it.
    columns::update(host, |layout| {
        *layout = view.columns.clone();
        true
    });

    // A rebuild, because the filter row is part of the one-time skeleton and a
    // different column set means a different filter row. It ends in a query,
    // which is the one query this whole call is allowed.
    //
    // The focus comes back only if it was in the grid: a page that applies a
    // view from its own control — a tab, a menu — keeps the focus there. Taking
    // it would break that control's keyboard pattern (point 69).
    let had_focus = host
        .shadow_root()
        .is_some_and(|root| root.active_element().is_some());
    // The selection goes with the old state, and the view's comes instead
    // (issue #322): the keys it holds, where this grid selects by key — none
    // otherwise. One that is dropped is not dropped silently (point 73): the
    // fresh state is told, so the result that follows says "Selection cleared"
    // like a sort would. A tree's selection counts with the nodes it hides
    // (issue #135), a grouped grid's with the rows of closed groups.
    let had_keys = runtime.borrow_mut().picked.clear();
    let had_selection = {
        let borrowed = runtime.borrow();
        had_keys
            || !borrowed.state.selection().is_empty()
            || borrowed
                .tree
                .as_ref()
                .is_some_and(|tree| !tree.selected_keys().is_empty())
    };
    if let Some(root) = host.shadow_root() {
        clear_root(&root);
    }
    reset_runtime(host);
    // The page's reason, said with the result (issue #146).
    if let Some(notice) = &notice {
        runtime
            .borrow_mut()
            .state
            .notice_with_result(notice.clone());
    }
    ensure_skeleton(host);
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.grouping = grouping_of(host).ok().flatten();
        borrowed.groups_filter = None;
        if let Some(grouping) = borrowed.grouping.as_mut() {
            grouping.set_expanded(&view.expanded);
        }
        borrowed.tree = tree_of(host).ok().flatten();
        borrowed.tree_asked = None;
        if let Some(tree) = borrowed.tree.as_mut() {
            tree.set_expanded(&view.expanded);
        }
        borrowed.filter_row = view.filter_row;
        // The search is not part of a view — the prototype's views leave it out
        // too — and a restored view with an old search still applied would show
        // rows the view never named.
        if !keep_search {
            borrowed.search_text.clear();
        }
        borrowed.nested = view
            .nested
            .as_ref()
            .and_then(|tree| opengrid_json::FromJson::from_json(tree).ok());
        borrowed.facets = view
            .facets
            .as_object()
            .map(|facets| {
                facets
                    .iter()
                    .filter_map(|(column, selection)| {
                        crate::facets::Selection::from_json(selection)
                            .map(|selection| (column.clone(), selection))
                    })
                    .collect()
            })
            .unwrap_or_default();
        borrowed.aggregate_choice = view
            .aggregates
            .iter()
            .filter_map(|(column, function)| {
                presentation::aggregate_from(function).map(|function| (column.clone(), function))
            })
            .collect();
    }

    // Now the DOM exists again: write the filters into it and the sort into the
    // state, then render and ask once.
    let visible = columns_of(host);
    if let Some(root) = host.shadow_root() {
        if let (Some(input), Some(search)) = (search_input(&root), &search) {
            input.set_value(search);
        }
        let entries: Vec<FilterEntry> = visible
            .iter()
            .map(|column| {
                view.filters
                    .iter()
                    .find(|entry| &entry.column == column)
                    .cloned()
                    .unwrap_or(FilterEntry {
                        column: column.clone(),
                        op: grid::FilterOp::Cmp(CmpOp::Eq),
                        value: String::new(),
                    })
            })
            .collect();
        write_filter_entries(&root, &entries);
    }

    {
        let schema = runtime.borrow().state.schema().clone();
        let entries: Vec<FilterEntry> = view.filters.clone();
        let filter = grid::filter_expr(&entries, &schema).ok().flatten();
        let mut borrowed = runtime.borrow_mut();
        // A literal on a column no result has typed yet is text for now; the
        // query that follows asks for the types first (point 88). An entry
        // without a value — empty, or a null test — needs no type.
        borrowed.retype_filter = entries.iter().any(|entry| {
            grid::takes_value(entry.op.as_str())
                && !entry.value.trim().is_empty()
                && !borrowed.known.contains_key(&entry.column)
        });
        borrowed.state.set_filter(filter);
        borrowed.state.set_sort(
            view.sort
                .iter()
                .filter_map(|(field, direction)| {
                    opengrid_types::FieldName::new(field)
                        .ok()
                        .map(|field| Sort {
                            field,
                            direction: if direction == "desc" {
                                SortDirection::Desc
                            } else {
                                SortDirection::Asc
                            },
                            // The defaults are the query model's (S4: sorting is
                            // binary), and a view has no business overriding them —
                            // it records what the reader chose, and the reader
                            // chooses a column and a direction.
                            nulls: Default::default(),
                            collation: Default::default(),
                        })
                })
                .collect(),
        );
        // A view saved without a sort still pages under a total order (S6),
        // the same fallback as clearing the last sort by hand.
        borrowed.state.ensure_sorted();
    }

    APPLYING_VIEW.with(|flag| flag.set(false));

    render(host, false);
    // A view may show or hide the filter row, which moves the columns' least
    // widths (issue #62).
    apply_widths(host);
    let by_key = {
        let borrowed = runtime.borrow();
        borrowed.tree.is_some() || keyed(&borrowed)
    };
    {
        // A selection an earlier view was still waiting for is void.
        let mut borrowed = runtime.borrow_mut();
        borrowed.view_round += 1;
        borrowed.restore = None;
    }
    // The panels of a restored view open as their rows load (issue #318).
    runtime.borrow_mut().detail_wanted = view.detail.clone();
    if by_key && !view.selected.is_empty() {
        // The keys are a page's word: the query that follows asks the source
        // which of them name a record first ([`restore_then_query`]).
        runtime.borrow_mut().restore = Some(Restore {
            keys: view.selected.clone(),
            had_selection,
            asked: false,
        });
    } else if had_selection {
        runtime.borrow_mut().state.note_selection_dropped();
        // The page hears it the same way it hears every other selection change.
        if by_key {
            dispatch_keys(host, &[]);
        } else {
            dispatch_selection(host, &[]);
        }
    }
    run_query(host, QueryKind::Data, had_focus);
}

/// A view's selection, waiting to be checked against the source.
pub(super) struct Restore {
    /// The keys the view holds, as the page handed them in.
    keys: Vec<opengrid_json::Json>,
    /// Whether the view replaced a selection: one that ends up empty was
    /// dropped, and is said.
    had_selection: bool,
    /// The source is being asked; queries wait until it has answered.
    asked: bool,
}

/// Restores a view's selection (issue #322), then runs the query it waited
/// for. The keys are untrusted: only those of the key column's type are asked,
/// and only those the source answers with are selected. What names no record
/// is dropped and said — with the result, in the one live region; a check
/// that fails is said as an error, and selects nothing.
pub(super) fn restore_then_query(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    kind: QueryKind,
    focus: bool,
) {
    let (round, keys) = {
        let mut borrowed = runtime.borrow_mut();
        let round = borrowed.view_round;
        let Some(restore) = borrowed.restore.as_mut() else {
            return;
        };
        // One check per view; a query that comes meanwhile is run by its end.
        if std::mem::replace(&mut restore.asked, true) {
            return;
        }
        (round, restore.keys.clone())
    };
    let (host, runtime) = (host.clone(), runtime.clone());
    spawn_local(async move {
        let outcome = known_ids(&host, &runtime, &keys).await;
        // Another view was set meanwhile: its selection is the one to restore.
        if runtime.borrow().view_round != round {
            return;
        }
        let Some(restore) = runtime.borrow_mut().restore.take() else {
            return;
        };
        let texts = texts(&host);
        let (ids, said) = match outcome {
            Ok(ids) => {
                let missing = (restore.keys.len() - ids.len()) as u64;
                (ids, (missing > 0).then(|| texts.selection_missing(missing)))
            }
            Err(message) => (Vec::new(), Some(texts.error(&message))),
        };
        let none = ids.is_empty();
        let keys = {
            let mut borrowed = runtime.borrow_mut();
            // Under a filter of the reader's the records need not be among
            // the rows; the page's filter they were asked under.
            let filtered = !matches!(reader_filter(&host, &borrowed, None), Ok(None));
            let borrowed = &mut *borrowed;
            // Said with the result of the view's query ([`settle`]).
            borrowed.restore_said = said;
            if none && restore.had_selection {
                borrowed.state.note_selection_dropped();
            }
            match borrowed.tree.as_mut() {
                Some(tree) => {
                    tree.set_selected(ids);
                    tree.selected_keys()
                }
                None => {
                    borrowed.picked.add(ids);
                    if filtered {
                        borrowed.picked.rows_changed();
                    }
                    borrowed.picked.keys()
                }
            }
        };
        if !none || restore.had_selection {
            dispatch_keys(&host, &keys);
        }
        run_query(&host, kind, focus);
    });
}

/// Which of `keys` name a record of the source, as ids — asked by the key
/// field (`tree-key` in a tree, else `row-key`) in pieces a filter list may
/// hold. A key that is not of the field's type names none, and is not asked.
/// Under the page's `filter` (issue #330): it says which records this grid
/// has, so a record it hides is none of them. The reader's filter does not
/// count — a selected record may well be filtered out of sight.
async fn known_ids(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    keys: &[opengrid_json::Json],
) -> Result<Vec<String>, String> {
    // The most values one `in` list takes (the query model's default).
    const PIECE: usize = 1_000;
    let field = {
        let borrowed = runtime.borrow();
        match borrowed.tree.as_ref() {
            Some(tree) => opengrid_types::FieldName::new(tree.key()).ok(),
            None => row_key_of(host),
        }
    };
    let (Some(provider), Some(source), Some(field)) = (
        provider(host),
        host.get_attribute(DATASOURCE_ATTRIBUTE),
        field,
    ) else {
        return Ok(Vec::new());
    };
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let page = page_filter(host)?;
    let select = [field.as_str().to_owned()];
    let sorts = [(field.as_str().to_owned(), "asc")];
    let known = runtime.borrow().known.get(field.as_str()).cloned();
    let data_type = match known {
        Some(known) => known.data_type,
        // No result has typed the column yet: asked for, without rows.
        None => {
            let probe = grid::query_json(&source, &select, &sorts, None, 0, 0);
            let result = ask_uncounted(host, &provider, &probe, &mode).await?;
            match result.schema.fields().first() {
                Some(field) => field.data_type,
                None => return Ok(Vec::new()),
            }
        }
    };
    let typed: Vec<opengrid_json::Json> = keys
        .iter()
        .filter(|key| Value::from_json_typed(key, &data_type).is_ok())
        .cloned()
        .collect();
    let mut ids = Vec::new();
    for piece in typed.chunks(PIECE) {
        let among = FilterExpr::Cmp {
            field: field.clone(),
            op: CmpOp::In,
            value: opengrid_json::Json::Array(piece.to_vec()),
        };
        let filter = match page.clone() {
            Some(page) => FilterExpr::And(vec![page, among]),
            None => among,
        };
        let query = grid::query_json(&source, &select, &sorts, Some(&filter), 0, PIECE as u64);
        let result = ask_uncounted(host, &provider, &query, &mode).await?;
        ids.extend(
            result
                .columns
                .first()
                .into_iter()
                .flatten()
                .map(grouping::key_id),
        );
    }
    Ok(ids)
}

/// Fires `opengrid-view-change`, unless a view is being applied.
///
/// The same contract as the other two events (point 35): on the host, `bubbles`
/// and `composed` — without `composed` it would not leave a shadow root the page
/// wrapped the element in — and not `cancelable`, because it reports what has
/// already happened.
pub(crate) fn dispatch_view(host: &HtmlElement) {
    let Some(view) = current_view(host) else {
        return;
    };
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("view"), &to_js(&view.to_json()));

    crate::element::dispatch(host, VIEW_EVENT, &detail);
}
