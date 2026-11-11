//! Column operations and the table around them: resizing, pinning, moving and hiding a column, the widths and the pinned columns' sheets, and the filter row's alignment with its columns.

use super::*;

/// Makes a column wider or narrower and says so.
///
/// The width is written onto the header cell and the data cells (see
/// [`apply_widths`]). The value is clamped so a column can never be resized into
/// something nobody can find again (and below the 24 px of WCAG 2.5.8).
pub(super) fn resize_column(host: &HtmlElement, col: usize, step: i32) {
    let columns = columns_of(host);
    let Some(name) = columns.get(col).cloned() else {
        return;
    };
    let current = measured_width(host, col).unwrap_or(120);
    let width = columns::layout(host)
        .borrow_mut()
        .resize(&name, step, current);
    apply_widths(host);
    announce(host, &texts(host).column_width(&name, width));
    // A pinned column made wider may be one too wide to keep in place.
    say_release(host);
    dispatch_view(host);
}

/// Where a shown column is pinned: the reader's choice, or else the page's.
pub(super) fn pin_of(host: &HtmlElement, name: &str) -> Option<Pin> {
    columns::layout(host)
        .borrow()
        .pin(name, &presentation::page_columns(host))
}

/// `Alt`+`Shift`+`←`/`→` on a header cell (issue #316): one step towards
/// that side — a column pinned at the other end is let go, one that is not
/// pinned is pinned here.
pub(super) fn step_pin(host: &HtmlElement, col: usize, towards_end: bool) {
    let Some(name) = columns_of(host).get(col).cloned() else {
        return;
    };
    let pin = match (pin_of(host, &name), towards_end) {
        (Some(Pin::Start), true) | (Some(Pin::End), false) => None,
        (_, true) => Some(Pin::End),
        (_, false) => Some(Pin::Start),
    };
    pin_column(host, &name, pin);
}

/// Pins a column, or lets it go, and says so.
///
/// A pinned column stands with the others pinned at its end of the row, so
/// this usually moves it — a new query, around the view the reader has
/// ([`rearrange`]). The announcement rides with that query's result; the focus
/// follows the column.
pub(super) fn pin_column(host: &HtmlElement, name: &str, pin: Option<Pin>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let page = presentation::page_columns(host);
    // A column of a band takes the band along (issue #319), and it is the
    // band that is said.
    let said = match page.band(name) {
        Some(band) => texts(host).band_pinned(band, pin),
        None => texts(host).column_pinned(name, pin),
    };
    let on_band = matches!(runtime.borrow().active, ActiveCell::Band { .. });
    let before = arrangement(host);
    if !columns::update(host, |layout| layout.set_pin(name, pin, &page)) {
        // As it is: said, so the key is not a dead one.
        announce(host, &said);
        return;
    }
    if before.is_some_and(|before| rearrange(host, &runtime, before, Some(said.clone()))) {
        if let Some(col) = columns_of(host).iter().position(|column| column == name) {
            runtime.borrow_mut().set_active(if on_band {
                ActiveCell::Band { col }
            } else {
                ActiveCell::Header { col }
            });
        }
        dispatch_view(host);
        return;
    }
    // The column stands where it stood: only what keeps it there changes.
    apply_widths(host);
    announce(host, &said);
    say_release(host);
    dispatch_view(host);
}

/// The shown columns, the bands above them and the view, read together
/// **before** something changes the order of the columns: the filter row is
/// read by position, so the view has to be gathered while the columns still
/// stand as the DOM has them.
pub(super) type Arrangement = (Vec<String>, Vec<(String, usize, usize)>, GridView);

pub(super) fn arrangement(host: &HtmlElement) -> Option<Arrangement> {
    Some((columns_of(host), bands_of(host), current_view(host)?))
}

/// Builds the grid again around the view it had, when the columns stand in
/// another order than `before` — a pin set or taken away (issue #316) — or
/// under other bands (issue #319). One
/// query; sort, filters, grouping, the open groups and the search stay.
/// Answers whether the order changed. It fires no `opengrid-view-change`: the
/// page's own `set_columns` comes through here too, and that is not the
/// reader's change.
pub(super) fn rearrange(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    before: Arrangement,
    notice: Option<String>,
) -> bool {
    let (columns, bands, mut view) = before;
    if columns_of(host) == columns && bands_of(host) == bands {
        return false;
    }
    view.columns = columns::layout(host).borrow().clone();
    apply_view(host, runtime, &view, notice, true);
    true
}

/// Says once that the pinned columns scroll with the others (issue #316) —
/// after [`align_filter`] found that they no longer fit, and not again until
/// they held in between.
fn say_release(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    if !std::mem::take(&mut runtime.borrow_mut().release_unsaid) {
        return;
    }
    runtime
        .borrow_mut()
        .state
        .add_notice(texts(host).pins_released.clone());
    render(host, false);
}

/// Moves a column one place and re-runs the query.
///
/// The order is part of the query's projection, so a move is a new query — the
/// same path a sort takes, and for the same reason: the result has a different
/// shape.
pub(super) fn move_column(host: &HtmlElement, col: usize, by: i32) {
    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    let columns = columns_of(host);
    let Some(name) = columns.get(col).cloned() else {
        return;
    };
    let moved = columns::layout(host).borrow_mut().move_column(
        &declared,
        &presentation::page_columns(host),
        &name,
        by,
    );
    let Some(order) = moved else {
        // At the end: nothing happens, and the reader is told why instead of
        // pressing the key again.
        announce(host, &texts(host).column_at_edge(&name));
        return;
    };
    let position = order.iter().position(|column| *column == name).unwrap_or(0);
    // Rebuild **first**: it replaces the runtime, so anything set before — the
    // active cell, a notice — would be thrown away with the old one.
    rebuild(host, true);
    // The focus follows the column, not the place it left. The query that the
    // rebuild started focuses the active cell when its result lands.
    if let Some(runtime) = runtime(host) {
        runtime
            .borrow_mut()
            .set_active(ActiveCell::Header { col: position });
    }
    announce(
        host,
        &texts(host).column_moved(&name, position as u64 + 1, order.len() as u64),
    );
    dispatch_view(host);
}

/// Moves the band above column `col` one place as a whole (issue #319) and
/// re-runs the query, as [`move_column`] does for one column. The focus stays
/// on the band's cell.
pub(super) fn move_band(host: &HtmlElement, col: usize, by: i32) {
    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    let page = presentation::page_columns(host);
    let columns = columns_of(host);
    let Some(name) = columns.get(col) else {
        return;
    };
    let Some(band) = page.band(name) else {
        return;
    };
    let moved = columns::layout(host)
        .borrow_mut()
        .move_band(&declared, &page, band, by);
    let Some(order) = moved else {
        announce(host, &texts(host).band_at_edge(band));
        return;
    };
    let position = order.iter().position(|column| column == name).unwrap_or(0);
    // Where the band starts now: the place of its first column.
    let first = page
        .band_spans(&order)
        .into_iter()
        .find(|(_, first, count)| (*first..first + count).contains(&position))
        .map_or(position, |(_, first, _)| first);
    // Rebuild **first**, as for a column: it replaces the runtime.
    rebuild(host, true);
    if let Some(runtime) = runtime(host) {
        runtime
            .borrow_mut()
            .set_active(ActiveCell::Band { col: position });
    }
    announce(
        host,
        &texts(host).band_moved(band, first as u64 + 1, order.len() as u64),
    );
    dispatch_view(host);
}

/// Hides the band whose cell is at `col` — every column it spans, in one
/// rebuild (issue #352). A band that is all there is stays, and says so. The
/// columns come back from the column list, which lists them; the band is back
/// with the first of them.
pub(super) fn hide_band(host: &HtmlElement, col: usize) {
    let shown = columns_of(host);
    let Some((band, first, count)) = bands_of(host)
        .into_iter()
        .find(|(_, first, _)| *first == col)
    else {
        return;
    };
    let t = texts(host);
    if count >= shown.len() {
        announce(host, &t.band_keep(&band));
        return;
    }
    let names: Vec<String> = shown[first..first + count].to_vec();
    if !columns::update(host, |layout| {
        names.iter().fold(false, |changed, name| {
            layout.set_hidden(name, true) | changed
        })
    }) {
        return;
    }
    let visible = columns_of(host).len() as u64;
    let declared =
        grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref()).len() as u64;
    rebuild(host, true);
    announce(host, &t.column_visibility(&band, true, visible, declared));
    dispatch_view(host);
}

/// Shows or hides a column and re-runs the query.
pub(super) fn set_column_hidden(host: &HtmlElement, name: &str, hidden: bool) {
    if !columns::update(host, |layout| layout.set_hidden(name, hidden)) {
        return;
    }
    let visible = columns_of(host).len() as u64;
    let declared =
        grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref()).len() as u64;
    // The column list stays open across the rebuild (issue #101): ticked
    // there, the focus stays on the checkbox, not on the grid's active cell.
    let panel_open = host
        .shadow_root()
        .and_then(|root| root.query_selector("[part=\"columns\"]").ok().flatten())
        .is_some_and(|panel| panel.matches(":popover-open").unwrap_or(false));
    // Rebuild **first**: it replaces the runtime, and a notice set before would
    // be thrown away with the old state.
    rebuild(host, !panel_open);
    if panel_open {
        reopen_columns(host, name);
    }
    announce(
        host,
        &texts(host).column_visibility(name, hidden, visible, declared),
    );
    dispatch_view(host);
}

/// Says what the attributes ask for and the grid will not do — a grouping
/// it refuses, a tree beside `page-size` — once, riding with the next result.
pub(super) fn announce_refusals(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let refused = tree_of(host).err().or_else(|| grouping_of(host).err());
    if let Some(message) = refused {
        runtime.borrow_mut().state.set_notice(message);
    }
}

/// Takes a new `tree` or `tree-key`: a new skeleton, from the top, nothing
/// selected — the rows mean other things now.
pub(super) fn retree(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    clear_root(&root);
    reset_runtime(host);
    announce_refusals(host, &runtime);
    ensure_skeleton(host);
    render(host, false);
    run_query(host, QueryKind::Data, false);
    dispatch_view(host);
}

/// Rebuilds the skeleton because the set or order of columns changed;
/// `focus_after` gives the focus back to the active cell once it is drawn.
fn rebuild(host: &HtmlElement, focus_after: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    clear_root(&root);
    reset_runtime(host);
    ensure_skeleton(host);
    render(host, false);
    run_query(host, QueryKind::Data, focus_after);
}

/// Opens the column list under its button (issue #101). A click outside and
/// `Escape` close it; the button's `aria-expanded` follows either way.
pub(super) fn open_columns(root: &ShadowRoot, focus_first: bool) {
    let (Ok(Some(panel)), Ok(Some(button))) = (
        root.query_selector("[part=\"columns\"]"),
        root.query_selector("[part=\"columns-toggle\"]"),
    ) else {
        return;
    };
    let (Ok(panel), Ok(button)) = (
        panel.dyn_into::<HtmlElement>(),
        button.dyn_into::<HtmlElement>(),
    ) else {
        return;
    };
    let _ = panel.show_popover();
    let _ = button.set_attribute("aria-expanded", "true");
    place_under(&panel, &button);
    if !panel.has_attribute("data-watched") {
        let _ = panel.set_attribute("data-watched", "");
        // Holds nothing of the grid: a closure that names the root keeps a
        // removed grid alive (issue #227), so it finds the root from the event.
        let closed = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let open = js_sys::Reflect::get(&event, &JsValue::from_str("newState"))
                .ok()
                .and_then(|state| state.as_string())
                .is_some_and(|state| state == "open");
            if !open
                && let Some(root) = root_of_target(&event)
                && let Ok(Some(button)) = root.query_selector("[part=\"columns-toggle\"]")
            {
                let _ = button.set_attribute("aria-expanded", "false");
            }
        })
        .into_js_value();
        let _ = panel.add_event_listener_with_callback("toggle", closed.unchecked_ref());
    }
    if focus_first
        && let Ok(Some(first)) = panel.query_selector("input[type=\"checkbox\"]")
        && let Ok(first) = first.dyn_into::<HtmlElement>()
    {
        let _ = first.focus();
    }
}

/// Closes the column list; with `refocus`, the focus goes back to its button.
pub(super) fn close_columns(root: &ShadowRoot, refocus: bool) {
    if let Ok(Some(panel)) = root.query_selector("[part=\"columns\"]")
        && let Ok(panel) = panel.dyn_into::<HtmlElement>()
    {
        let _ = panel.hide_popover();
    }
    if let Ok(Some(button)) = root.query_selector("[part=\"columns-toggle\"]")
        && let Ok(button) = button.dyn_into::<HtmlElement>()
    {
        let _ = button.set_attribute("aria-expanded", "false");
        if refocus {
            let _ = button.focus();
        }
    }
}

/// After a rebuild the list is a new one: open again, the focus on `column`'s
/// checkbox, where the reader left it.
fn reopen_columns(host: &HtmlElement, column: &str) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    open_columns(&root, false);
    if let Ok(Some(input)) = root.query_selector(&format!(
        "[part=\"columns\"] input[data-column=\"{column}\"]"
    )) && let Ok(input) = input.dyn_into::<HtmlElement>()
    {
        let _ = input.focus();
    }
}

/// Puts a one-off sentence into the status line.
///
/// The one polite live region the grid has (point 41) — a column operation is
/// a change only the sighted see, so it has to be said.
pub(super) fn announce(host: &HtmlElement, message: &str) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    runtime.borrow_mut().state.set_notice(message.to_owned());
    render(host, false);
}

/// The rendered width of a header cell, if it has been laid out.
/// Lines the filter row up with the columns (issue #62): each column's group
/// as wide as its header cell, the first starting where the first column
/// starts — after the selection column. The row scrolls with the viewport
/// (`on_scroll`), so a field stays under its column.
///
/// The same measurements say where the pinned columns stand (issue #316), so
/// their sheet is written here too ([`crate::pins`]), and a pinned column's
/// field is kept above it.
fn align_filter(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let (Ok(Some(filter)), Ok(Some(table))) = (
        root.query_selector("[part=\"filter\"]"),
        root.query_selector("table"),
    ) else {
        return;
    };
    let Ok(headers) = root.query_selector_all("th[data-col]") else {
        return;
    };
    // Every measurement first, every write after: a write between two reads
    // makes the browser lay the page out again for the next read.
    // Right to left (issue #320) the same distances are measured from the right
    // edges: the row's inside, and the first column's start.
    let rtl = crate::element::is_rtl(&filter);
    let table_box = table.get_bounding_client_rect();
    let table_left = table_box.left();
    let mut groups = Vec::new();
    let mut measured = Vec::new();
    for index in 0..headers.length() {
        let Some(header) = headers
            .item(index)
            .and_then(|node| node.dyn_into::<Element>().ok())
        else {
            continue;
        };
        let Some(col) = header.get_attribute("data-col") else {
            continue;
        };
        let rect = header.get_bounding_client_rect();
        measured.push((
            col.parse::<usize>().unwrap_or(usize::MAX),
            if rtl { rect.right() } else { rect.left() } - table_left,
            rect.width(),
        ));
        if let Ok(Some(operator)) =
            root.query_selector(&format!("[part=\"filter-operator\"][data-col=\"{col}\"]"))
            && let Some(group) = operator.parent_element()
            && let Ok(group) = group.dyn_into::<HtmlElement>()
        {
            groups.push((group, format!("[data-col=\"{col}\"]"), rect.width()));
        }
    }
    // By column: under bands (issue #319) the header cells of the two rows are
    // not the columns in DOM order.
    measured.sort_by_key(|(col, _, _)| *col);
    let Some(mut first) = measured.first().map(|(_, start, _)| *start) else {
        return;
    };
    let cells: Vec<(usize, f64)> = measured
        .iter()
        .map(|(col, _, width)| (*col, *width))
        .collect();
    let viewport = root.query_selector("[part~=\"viewport\"]").ok().flatten();
    // The pinned cells, from the widths just read. A viewport without a width
    // is not laid out: nothing is known, and nothing is written.
    let room = viewport
        .as_ref()
        .map_or(0.0, |viewport| f64::from(viewport.client_width()));
    let pins = (room > 0.0).then(|| {
        let names = columns_of(host);
        let page = presentation::page_columns(host);
        let layout = columns::layout(host);
        let layout = layout.borrow();
        let columns: Vec<(usize, f64, Option<Pin>)> = cells
            .iter()
            .map(|(col, width)| {
                let pin = names.get(*col).and_then(|name| layout.pin(name, &page));
                (*col, *width, pin)
            })
            .collect();
        let select = root
            .query_selector("th[data-select]")
            .ok()
            .flatten()
            .map(|cell| cell.get_bounding_client_rect().width());
        let actions = columns.len().saturating_sub(names.len());
        crate::pins::pins(&columns, select, actions, room)
    });
    let geometry = match &pins {
        Some(crate::pins::Pins::Held(geometry)) => {
            // A pinned band's cell stays above its columns (issue #319).
            let bands: Vec<(usize, usize)> = bands_of(host)
                .into_iter()
                .map(|(_, first, count)| (first, count))
                .collect();
            geometry.clone().with_bands(&bands)
        }
        _ => crate::pins::PinGeometry::default(),
    };
    let offset = |cells: &[(String, f64)], selector: &str| {
        cells
            .iter()
            .find(|(cell, _)| cell == selector)
            .map(|(_, offset)| *offset)
    };
    // A pinned header is measured where it stands now, not where its column
    // starts: scrolled sideways those differ, and the offset is the start —
    // from the table's start edge, where `first` counts from its left one.
    let lead = offset(&geometry.start, "[data-col=\"0\"]");
    if let Some(lead) = lead {
        first = if rtl { table_box.width() - lead } else { lead };
    }
    // How far the viewport's end edge is from the end of the row's content
    // (its inside without the padding), for a field pinned at the end.
    let filter_box = filter.get_bounding_client_rect();
    let (end_gap, end_padding) = match (&viewport, geometry.end.is_empty()) {
        (Some(viewport), false) => {
            let inside =
                viewport.get_bounding_client_rect().left() + f64::from(viewport.client_left());
            let row = filter_box.left() + f64::from(filter.client_left());
            let padding = web_sys::window()
                .and_then(|window| window.get_computed_style(&filter).ok().flatten())
                .and_then(|style| style.get_property_value("padding-inline-end").ok())
                .and_then(|value| value.strip_suffix("px")?.parse::<f64>().ok())
                .unwrap_or(0.0);
            let gap = if rtl {
                inside - row
            } else {
                row + f64::from(filter.client_width()) - inside - room
            };
            (gap - padding, padding)
        }
        _ => (0.0, 0.0),
    };
    // Where the table starts, unscrolled, measured from the row's own inside.
    let scrolled = viewport
        .as_ref()
        .map_or(0, |viewport| viewport.scroll_left());
    // A viewport scrolled right to left has a negative `scrollLeft`; the sum
    // is where the column stands unscrolled in both directions.
    let column = table_left + f64::from(scrolled) + first;
    let inside = filter_box.left() + f64::from(filter.client_left());
    let indent = if rtl {
        inside + f64::from(filter.client_width()) - column
    } else {
        column - inside
    };
    if pins.is_some()
        && let Some(sheet) = pins_sheet(&root)
    {
        sheet
            .replace_sync(&crate::pins::pins_css(&geometry))
            .unwrap_or_default();
    }
    if let (Some(pins), Some(runtime)) = (&pins, runtime(host)) {
        let mut runtime = runtime.borrow_mut();
        let released = *pins == crate::pins::Pins::Released;
        // Said once when the pins let go ([`say_release`]), not while they
        // stay that way; holding again takes an unsaid one back.
        runtime.release_unsaid = released && (runtime.release_unsaid || !runtime.pins_released);
        runtime.pins_released = released;
    }
    for (group, selector, width) in groups {
        // A pinned column's field stays above it: sticky in the row as the
        // cells are in the viewport, on the row's ground so the fields that
        // scroll pass under it — and under the room before the first one,
        // where the selection column stands.
        // A sticky offset counts from the row's content edge, so the row's
        // own padding — at the start the indent written below — is taken off.
        let pinned = if let Some(start) = offset(&geometry.start, &selector) {
            let cover = indent.max(0.0) * if rtl { 1.0 } else { -1.0 };
            format!(
                " position: sticky; z-index: 1; inset-inline-start: {:.2}px; \
                 background: var({ground}); box-shadow: {cover:.2}px 0 0 var({ground});",
                start - lead.unwrap_or(0.0),
                ground = grid::SURFACE_2_PROPERTY
            )
        } else if let Some(end) = offset(&geometry.end, &selector) {
            format!(
                " position: sticky; z-index: 1; inset-inline-end: {:.2}px; background: var({});",
                end_gap + end,
                grid::SURFACE_2_PROPERTY
            )
        } else {
            String::new()
        };
        // Through the CSSOM, like every style the grid writes: a page
        // whose `style-src` lacks 'unsafe-inline' refuses the attribute.
        group.style().set_css_text(&format!(
            "display: inline-flex; align-items: center; gap: 0.25rem; flex: none; \
             box-sizing: border-box; padding-inline-end: 0.25rem; width: {width:.2}px;{pinned}"
        ));
    }
    if let Ok(filter) = filter.dyn_into::<HtmlElement>() {
        let _ = filter
            .style()
            .set_property("padding-inline-start", &format!("{:.2}px", indent.max(0.0)));
        // The row scrolls a field into view as the viewport does a cell: clear
        // of the pinned fields (WCAG 2.4.11), which stand this far in.
        if geometry.is_empty() {
            let _ = filter.style().remove_property("scroll-padding-inline");
        } else {
            let start = lead.map_or(0.0, |lead| indent - lead + geometry.start_width);
            let end = if geometry.end.is_empty() {
                0.0
            } else {
                end_gap + end_padding + geometry.end_width
            };
            let _ = filter.style().set_property(
                "scroll-padding-inline",
                &format!("{:.2}px {:.2}px", start.max(0.0), end.max(0.0)),
            );
        }
        keep_focus_in_sight(&root, &filter);
    }
}

/// A filter control that has the focus stays in sight when the row is laid out
/// again (a window resized, a column shown, the facets opened).
///
/// The widths and the indent change under the control, and the row's own
/// horizontal scroll position does not follow: with the focus on a control the
/// browser had scrolled to, the control could end up beside the row's edge and
/// stay there (WCAG 2.4.11). Only the row is moved, sideways — never the page.
fn keep_focus_in_sight(root: &ShadowRoot, filter: &HtmlElement) {
    let Some(focused) = root.active_element() else {
        return;
    };
    if !filter.contains(Some(&focused)) {
        return;
    }
    let (frame, control) = (
        filter.get_bounding_client_rect(),
        focused.get_bounding_client_rect(),
    );
    let behind = frame.left() - control.left();
    let beyond = control.right() - frame.right();
    if behind > 0.5 {
        filter.set_scroll_left(filter.scroll_left() - behind.ceil() as i32);
    } else if beyond > 0.5 {
        filter.set_scroll_left(filter.scroll_left() + beyond.ceil() as i32);
    }
}

/// Re-aligns the filter row whenever the table changes size — a wider
/// window, a column shown, the facets opened beside it. The callback finds
/// its grid through the table it observes and holds nothing, so an observer
/// never keeps a removed grid alive.
pub(super) fn observe_columns(root: &ShadowRoot) {
    let Ok(Some(table)) = root.query_selector("table") else {
        return;
    };
    let callback = Closure::<dyn FnMut(js_sys::Array)>::new(|entries: js_sys::Array| {
        let host = js_sys::Reflect::get(&entries.get(0), &JsValue::from_str("target"))
            .ok()
            .and_then(|target| target.dyn_into::<Element>().ok())
            .and_then(|table| table.get_root_node().dyn_into::<ShadowRoot>().ok())
            .and_then(|root| root.host().dyn_into::<HtmlElement>().ok());
        if let Some(host) = host {
            align_filter(&host);
            say_release(&host);
            follow_row_height(&host);
        }
    })
    .into_js_value();
    if let Ok(observer) = web_sys::ResizeObserver::new(callback.unchecked_ref()) {
        observer.observe(&table);
        // A table wider than the grid keeps its size when the grid gets
        // narrower, and the pinned columns' share is of the grid (issue #316).
        if let Ok(Some(viewport)) = root.query_selector("[part~=\"viewport\"]") {
            observer.observe(&viewport);
        }
        // A custom property cannot be observed, but the rows it sizes can: the
        // first pool row changes its box when `--og-row-height` changes, even
        // where the header has a height of its own.
        if let Ok(Some(row)) = root.query_selector("tbody tr") {
            observer.observe(&row);
        }
    }
}

/// Takes up a `--og-row-height` that changed while the grid stays in the page
/// (a theme switch, a class on a parent): the sizer, the rows' places and the
/// window follow it, the way they do after a `density` change. Called when the
/// table or a row changed its size; most of the time the height is the same and
/// nothing happens.
fn follow_row_height(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let height = resolve_row_height(host);
    {
        let mut runtime = runtime.borrow_mut();
        if runtime.row_height == height {
            return;
        }
        runtime.row_height = height;
    }
    sync_page_window(host, &runtime);
    if !is_paging(host) {
        window_from_scroll(&runtime);
    }
    render(host, false);
    run_query(host, QueryKind::Data, false);
}

fn measured_width(host: &HtmlElement, col: usize) -> Option<u32> {
    let root = host.shadow_root()?;
    let cell = root
        .query_selector(&format!("th[data-col=\"{col}\"]"))
        .ok()
        .flatten()?;
    let width = cell.get_bounding_client_rect().width();
    (width > 0.0).then_some(width as u32)
}

/// Writes the reader's widths onto the header cells **and** the data cells.
///
/// Every body row is its own `display: table` (it is absolutely positioned, so
/// it can be moved without reflowing the others), and a row-table does not
/// read the header's widths. As long as no column had a width, both split the
/// same total evenly and lined up by accident; one configured width among
/// automatic ones pulled header and values apart. So the widths also go into a
/// small stylesheet for `td[data-col]` — one write per change rather than one
/// per rendered cell, and pooled cells pick it up as they are recycled.
pub(super) fn apply_widths(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let layout = columns::layout(host);
    let layout = layout.borrow();
    let mut rules = String::new();
    let filter_shown = runtime(host).is_some_and(|runtime| runtime.borrow().filter_row)
        && root
            .query_selector("[part=\"filter\"]:not([hidden])")
            .ok()
            .flatten()
            .is_some();
    let narrowest = grid::header_min_width(host.has_attribute(grid::COLUMN_MENU_ATTRIBUTE)).max(
        if filter_shown {
            grid::FILTER_MIN_WIDTH
        } else {
            0
        },
    );
    // What the columns need together: a column without a width of its own
    // shares what is left, and in a narrow grid that was 11 px — header and
    // filter unusable (issue #62). With this as the table's least width, the
    // viewport scrolls sideways instead.
    let mut least = 0u32;
    for (col, name) in columns_of(host).iter().enumerate() {
        // The reader's resize leads; the configuration is only where a column
        // starts (point 60, the same attribute/value relationship as the view).
        // Neither may draw it narrower than its header needs (issue #61).
        let width = layout
            .width(name)
            .or_else(|| presentation::styles(host).width(name))
            .map(|width| width.max(narrowest));
        least += width.unwrap_or(narrowest);
        if let Some(width) = width {
            // The `<col>` is there in a header with bands (issue #319), whose
            // first row no longer says how wide a column under a band is.
            rules.push_str(&format!(
                "td[data-col=\"{col}\"], col[data-width-col=\"{col}\"] {{ width: {width}px; }}\n"
            ));
        }
        let Ok(Some(cell)) = root.query_selector(&format!("th[data-col=\"{col}\"]")) else {
            continue;
        };
        match (width, cell.dyn_into::<HtmlElement>()) {
            (Some(width), Ok(cell)) => cell.style().set_css_text(&format!("width: {width}px;")),
            (None, Ok(cell)) => {
                let _ = cell.remove_attribute("style");
            }
            _ => {}
        }
    }
    // The selection column, when there is one, is 44 px (its own rule).
    if root
        .query_selector("th[data-select]")
        .ok()
        .flatten()
        .is_some()
    {
        least += 44;
    }
    rules.push_str(&format!(
        "table {{ min-width: {least}px; {}: {least}px; }}\n",
        grid::TABLE_LEAST_PROPERTY
    ));
    if let Some(sheet) = widths_sheet(&root) {
        sheet.replace_sync(&rules).unwrap_or_default();
    }
    align_filter(host);
}

thread_local! {
    /// The grid's look ([`grid::grid_css`]), parsed once and shared by every
    /// grid of the document.
    static GRID_SHEET: Option<web_sys::CssStyleSheet> = web_sys::CssStyleSheet::new()
        .ok()
        .inspect(|sheet| sheet.replace_sync(&grid::grid_css()).unwrap_or_default());
}

/// Adopts the grid's sheets into a fresh shadow root: the shared look, then
/// two sheets of this grid's own — for its column widths ([`apply_widths`])
/// and for its pinned columns ([`align_filter`]).
///
/// Adopted rather than `<style>` children: a page whose `style-src` lacks
/// 'unsafe-inline' — no nonce, hashes only — refuses a `<style>` element
/// written from script, and the grid would draw without its look, its widths
/// and its row positions. Constructed sheets are not governed by `style-src`.
/// They also outlive [`clear_root`], so a rebuild keeps them.
pub(super) fn adopt_sheets(root: &ShadowRoot) {
    GRID_SHEET.with(|look| {
        if let (Some(look), Ok(widths), Ok(pins)) = (
            look,
            web_sys::CssStyleSheet::new(),
            web_sys::CssStyleSheet::new(),
        ) {
            let sheets = js_sys::Array::of3(look, &widths, &pins);
            root.set_adopted_style_sheets(&JsValue::from(sheets));
        }
    });
}

/// The sheet [`adopt_sheets`] gave this grid for its column widths.
fn widths_sheet(root: &ShadowRoot) -> Option<web_sys::CssStyleSheet> {
    root.adopted_style_sheets().get(1).dyn_into().ok()
}

/// The sheet [`adopt_sheets`] gave this grid for its pinned columns.
fn pins_sheet(root: &ShadowRoot) -> Option<web_sys::CssStyleSheet> {
    root.adopted_style_sheets().get(2).dyn_into().ok()
}

/// The columns the grid actually shows, in the order it shows them.
///
/// The `columns` attribute says which columns exist; the reader's layout
/// (point 36) says which of them are visible and in what order. One function,
/// so the query, the skeleton and the render can never disagree.
pub(super) fn columns_of(host: &HtmlElement) -> Vec<String> {
    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    columns::layout(host)
        .borrow()
        .effective(&declared, &presentation::page_columns(host))
}

/// The column bands above the shown columns (issue #319), each with its first
/// column and how many it spans — read where the order of the columns is, so
/// the header and the order cannot disagree.
pub(super) fn bands_of(host: &HtmlElement) -> Vec<(String, usize, usize)> {
    presentation::page_columns(host).band_spans(&columns_of(host))
}

/// How many DOM row slots the grid keeps.
///
/// While paging that is the **page size**: the pool is the page, because there
/// is nothing to scroll and every row of the page is on screen. While
/// virtualizing it is `window-size`, the recycled pool of point 17. One
/// function, so the skeleton, the window and the query can never disagree about
/// how many rows there are.
pub(super) fn pool_of(host: &HtmlElement) -> u64 {
    grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).unwrap_or_else(|| {
        grid::parse_window_size(host.get_attribute(WINDOW_SIZE_ATTRIBUTE).as_deref())
    })
}
