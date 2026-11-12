//! Master/detail: a panel the page builds under a row (issue #318).
//!
//! The page gives one function (`set_detail_renderer`) that returns the node a
//! record's panel shows. A button in the first cell of a row opens and closes
//! it, and so does `Alt`+`↓` / `Alt`+`↑` on the row — the way a menu button
//! opens. The button carries `aria-expanded`; the row stays a `row`. Under the
//! row stands one more `row` with one `gridcell` across the columns, named by
//! the master row.
//!
//! **The geometry.** A panel is as tall as the page's node, measured after it is
//! in the document and again whenever it resizes. [`Opened`] keeps that height
//! per open row, and every place that turns rows into pixels or back — the
//! rows' `translateY`, the sizer, `aria-rowindex` and `aria-rowcount`, the row
//! the scroll position stands at — counts the panels above (`grid::Panels`).
//!
//! **Which rows.** A panel belongs to a record, so a grid offers it only with a
//! `row-key`, and only ungrouped and not a tree: a panel under a group header
//! or a node has no clear meaning. Where the result is taller than the sizer
//! may be, a row's place is the scale's and not a sum, so nothing is offered
//! there either. A new result closes the panels: its rows are others.

use super::*;
use opengrid_json::Json;

/// Where `set_detail_renderer` keeps the page's function.
const KEY: &str = "__opengridDetailRenderer";
/// The one observer of a grid's panels.
const OBSERVER: &str = "__opengridDetailObserver";
/// The button in the first cell.
const TOGGLE: &str = "button[data-detail-toggle]";
/// A panel's row.
const PANEL: &str = "tr[data-detail-row]";

/// A row whose panel is open.
pub(super) struct Opened {
    /// The row, in the result.
    pub row: u64,
    /// The record's `row-key`, which the row is checked against when it loads.
    pub key: Json,
    /// The panel's measured height in pixels, 0 until it was measured.
    pub height: u64,
}

fn renderer(host: &HtmlElement) -> Option<js_sys::Function> {
    js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(KEY))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()
}

/// The panels as the geometry counts them.
pub(super) fn panels(runtime: &GridRuntime) -> Vec<(u64, u64)> {
    runtime
        .details
        .iter()
        .map(|opened| (opened.row, opened.height))
        .collect()
}

/// Stores the page's function and draws again.
pub(super) fn store(host: &HtmlElement, function: &JsValue) {
    let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(KEY), function);
    if runtime(host).is_some() {
        render(host, false);
    }
}

/// Whether the grid offers panels now.
fn offered(host: &HtmlElement, runtime: &GridRuntime) -> bool {
    renderer(host).is_some()
        && has_row_key(host)
        && runtime.grouping.is_none()
        && runtime.tree.is_none()
        && !grid::is_capped(runtime.state.row_count(), runtime.row_height)
}

fn key_of(runtime: &GridRuntime, row: u64) -> Option<Json> {
    runtime
        .state
        .row_key(row)
        .map(opengrid_json::ToJson::to_json)
}

/// A key as a cache's name.
fn name_of(key: &Json) -> String {
    key.to_string()
}

/// A new result: its rows are others.
pub(super) fn close_all(runtime: &mut GridRuntime) {
    runtime.details.clear();
    runtime.detail_nodes.clear();
}

/// Opens (`Some(true)`), closes (`Some(false)`) or flips (`None`) the panel of
/// `row`, and says so once.
pub(super) fn toggle(host: &HtmlElement, row: u64, open: Option<bool>) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let (key, was) = {
        let borrowed = runtime.borrow();
        if !offered(host, &borrowed) {
            return;
        }
        let Some(key) = key_of(&borrowed, row) else {
            return;
        };
        let was = borrowed.details.iter().any(|opened| opened.row == row);
        (key, was)
    };
    let open = open.unwrap_or(!was);
    if open == was {
        return;
    }
    {
        let mut borrowed = runtime.borrow_mut();
        if open {
            let at = borrowed.details.partition_point(|opened| opened.row < row);
            borrowed.details.insert(
                at,
                Opened {
                    row,
                    key: key.clone(),
                    height: 0,
                },
            );
        } else {
            borrowed.details.retain(|opened| opened.row != row);
            let name = name_of(&key);
            borrowed.detail_nodes.retain(|(cached, _)| *cached != name);
        }
        borrowed.detail_wanted.retain(|wanted| *wanted != key);
    }
    let detail = js_sys::Object::new();
    for (name, value) in [
        ("key", to_js(&key)),
        ("row", JsValue::from_f64(row as f64)),
        ("open", JsValue::from_bool(open)),
    ] {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(name), &value);
    }
    crate::element::dispatch(
        host,
        crate::grid_element_events::DETAIL_TOGGLE_EVENT,
        &detail,
    );
    let texts = texts(host);
    runtime.borrow_mut().state.set_notice(if open {
        texts.detail_shown.clone()
    } else {
        texts.detail_hidden.clone()
    });
    render(host, false);
    view::dispatch_view(host);
}

/// A click on the button of a row.
pub(super) fn on_click(host: &HtmlElement, target: &Element) -> bool {
    let Ok(Some(button)) = target.closest(TOGGLE) else {
        return false;
    };
    let Some(row) = button
        .closest("td[data-row]")
        .ok()
        .flatten()
        .and_then(|td| td.get_attribute("data-row"))
        .and_then(|row| row.parse::<u64>().ok())
    else {
        return false;
    };
    toggle(host, row, None);
    if let Some(runtime) = runtime(host) {
        // The focus stays where the reader is: on the row they opened.
        runtime
            .borrow_mut()
            .set_active(ActiveCell::Data(CellRef::new(row, 0)));
    }
    true
}

/// `Alt`+`↓` opens and `Alt`+`↑` closes the panel of the active row. `true` when
/// the key was this.
pub(super) fn on_key_down(host: &HtmlElement, event: &KeyboardEvent, active: ActiveCell) -> bool {
    let (open, ActiveCell::Data(cell)) = (
        match (event.alt_key(), event.key().as_str()) {
            (true, "ArrowDown") => true,
            (true, "ArrowUp") => false,
            _ => return false,
        },
        active,
    ) else {
        return false;
    };
    let Some(runtime) = runtime(host) else {
        return false;
    };
    if !offered(host, &runtime.borrow()) {
        return false;
    }
    event.prevent_default();
    toggle(host, cell.row, Some(open));
    true
}

/// A key inside a panel is the page's, bar `Escape`: that closes the panel and
/// goes back to the row. `true` when the key was inside a panel.
pub(super) fn on_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) -> bool {
    let Ok(Some(panel)) = target.closest(PANEL) else {
        return false;
    };
    if event.key() == "Escape"
        && let Some(row) = panel
            .get_attribute("data-detail-row")
            .and_then(|row| row.parse::<u64>().ok())
    {
        event.prevent_default();
        toggle(host, row, Some(false));
        if let Some(runtime) = runtime(host) {
            runtime
                .borrow_mut()
                .set_active(ActiveCell::Data(CellRef::new(row, 0)));
        }
        focus_active(host);
    }
    true
}

/// Draws what the open rows need, after the frame: the buttons, the panels, and
/// the heights they measure. Closes what the result no longer holds.
pub(super) fn draw(host: &HtmlElement, root: &ShadowRoot) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let function = renderer(host);
    let on = offered(host, &runtime.borrow());
    if !on {
        let had = {
            let mut borrowed = runtime.borrow_mut();
            // What a view asked for waits for a renderer.
            let had = !borrowed.details.is_empty();
            close_all(&mut borrowed);
            had
        };
        remove_drawn(root);
        if had {
            render(host, false);
        }
        return;
    }
    let Some(function) = function else {
        return;
    };
    if reconcile(&runtime) {
        // The set of panels changed under the frame: draw it again.
        render(host, false);
        return;
    }
    draw_buttons(host, root, &runtime);
    if draw_panels(host, root, &runtime, &function) {
        render(host, false);
    }
}

/// Takes the buttons and the panels out.
fn remove_drawn(root: &ShadowRoot) {
    if let Ok(nodes) = root.query_selector_all(&format!("{TOGGLE}, {PANEL}")) {
        for index in 0..nodes.length() {
            if let Some(node) = nodes.item(index).and_then(|n| n.dyn_into::<Element>().ok()) {
                node.remove();
            }
        }
    }
}

/// Checks the open rows against what is loaded, and opens the ones a view
/// asked for as their rows load. `true` when a panel opened or closed.
fn reconcile(runtime: &Rc<RefCell<GridRuntime>>) -> bool {
    let mut borrowed = runtime.borrow_mut();
    let borrowed = &mut *borrowed;
    let before = borrowed.details.len();
    let state = &borrowed.state;
    // A row that loaded with another key is another record: its panel goes.
    let dropped: Vec<Json> = borrowed
        .details
        .iter()
        .filter(|opened| key_in(state, opened.row).is_some_and(|key| key != opened.key))
        .map(|opened| opened.key.clone())
        .collect();
    borrowed
        .details
        .retain(|opened| key_in(state, opened.row).is_none_or(|key| key == opened.key));
    for key in dropped {
        let name = name_of(&key);
        borrowed.detail_nodes.retain(|(cached, _)| *cached != name);
    }
    let mut changed = borrowed.details.len() != before;
    if !borrowed.detail_wanted.is_empty() {
        let window = state.window();
        let end = window.offset.saturating_add(window.count);
        for row in window.offset..end {
            let Some(key) = key_in(state, row) else {
                continue;
            };
            let Some(at) = borrowed
                .detail_wanted
                .iter()
                .position(|wanted| *wanted == key)
            else {
                continue;
            };
            borrowed.detail_wanted.remove(at);
            let place = borrowed.details.partition_point(|opened| opened.row < row);
            borrowed.details.insert(
                place,
                Opened {
                    row,
                    key,
                    height: 0,
                },
            );
            changed = true;
        }
    }
    changed
}

fn key_in(state: &GridState, row: u64) -> Option<Json> {
    state.row_key(row).map(opengrid_json::ToJson::to_json)
}

/// The button in the first cell of every row in view.
fn draw_buttons(host: &HtmlElement, root: &ShadowRoot, runtime: &Rc<RefCell<GridRuntime>>) {
    let Some(document) = host.owner_document() else {
        return;
    };
    let texts = texts(host);
    let Ok(cells) = root.query_selector_all("tbody td[data-col=\"0\"][data-row]") else {
        return;
    };
    for index in 0..cells.length() {
        let Some(td) = cells.item(index).and_then(|n| n.dyn_into::<Element>().ok()) else {
            continue;
        };
        let Some(row) = td
            .get_attribute("data-row")
            .and_then(|row| row.parse::<u64>().ok())
        else {
            continue;
        };
        let (open, editing, keyed) = {
            let borrowed = runtime.borrow();
            (
                borrowed.details.iter().any(|opened| opened.row == row),
                borrowed.state.editing() == Some(CellRef::new(row, 0)),
                borrowed.state.row_key(row).is_some(),
            )
        };
        if editing || !keyed {
            continue;
        }
        let expanded = if open { "true" } else { "false" };
        if let Ok(Some(button)) = td.query_selector(TOGGLE) {
            if button.get_attribute("aria-expanded").as_deref() == Some(expanded) {
                continue;
            }
            button.remove();
        }
        let Ok(button) = document.create_element("button") else {
            continue;
        };
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("data-detail-toggle", "");
        let _ = button.set_attribute("part", "detail-toggle");
        let _ = button.set_attribute("tabindex", "-1");
        let _ = button.set_attribute("aria-expanded", expanded);
        let _ = button.set_attribute(
            "aria-keyshortcuts",
            if open { "Alt+ArrowUp" } else { "Alt+ArrowDown" },
        );
        let _ = button.set_attribute(
            "aria-label",
            if open {
                &texts.detail_hide
            } else {
                &texts.detail_show
            },
        );
        button.set_text_content(Some(if open { "\u{25be}" } else { "\u{25b8}" }));
        let _ = td.insert_before(&button, td.first_child().as_ref());
    }
}

/// The record of a row as the page's function gets it.
fn record_of(runtime: &GridRuntime, row: u64) -> JsValue {
    let record = js_sys::Object::new();
    for (col, field) in runtime.state.schema().fields().iter().enumerate() {
        if let Some(cell) = runtime.state.cell(CellRef::new(row, col)) {
            let _ = js_sys::Reflect::set(
                &record,
                &JsValue::from_str(field.name.as_str()),
                &to_js(&opengrid_json::ToJson::to_json(cell)),
            );
        }
    }
    record.into()
}

/// What the first cell of `row` says, for the panel's name.
fn title_of(host: &HtmlElement, runtime: &GridRuntime, row: u64) -> String {
    runtime
        .state
        .cell(CellRef::new(row, 0))
        .map(|value| formatter(host, runtime.state.schema()).text(0, value))
        .unwrap_or_default()
}

/// The panels of the open rows in view: made when missing, placed, measured.
/// `true` when a height changed, so the frame is to be drawn again.
fn draw_panels(
    host: &HtmlElement,
    root: &ShadowRoot,
    runtime: &Rc<RefCell<GridRuntime>>,
    function: &js_sys::Function,
) -> bool {
    let (Some(document), Ok(Some(tbody))) = (host.owner_document(), root.query_selector("tbody"))
    else {
        return false;
    };
    let paging = paging_of(host, runtime);
    let (rows, header_rows, row_height, columns) = {
        let borrowed = runtime.borrow();
        let window = borrowed.state.window();
        let first = if paging.size.is_some() {
            paging.base
        } else {
            window.offset
        };
        let last = if paging.size.is_some() {
            paging.base + paging.rows
        } else {
            window.offset.saturating_add(window.count)
        };
        (
            borrowed
                .details
                .iter()
                .filter(|opened| opened.row >= first && opened.row < last)
                .map(|opened| (opened.row, opened.key.clone()))
                .collect::<Vec<_>>(),
            borrowed.view.as_ref().map_or(1, |view| view.header_rows()),
            borrowed.row_height,
            root.query_selector("tbody tr:not([data-detail-row])")
                .ok()
                .flatten()
                .map_or(1, |row| row.child_element_count().max(1)),
        )
    };
    // The panels of rows out of view go; their nodes stay with the cache.
    let mut present: Vec<u64> = Vec::new();
    if let Ok(panels) = root.query_selector_all(PANEL) {
        for index in 0..panels.length() {
            let Some(panel) = panels
                .item(index)
                .and_then(|n| n.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let row = panel
                .get_attribute("data-detail-row")
                .and_then(|row| row.parse::<u64>().ok());
            match row {
                Some(row) if rows.iter().any(|(open, _)| *open == row) => present.push(row),
                _ => panel.remove(),
            }
        }
    }
    let mut remeasure = false;
    let mut refused: Vec<u64> = Vec::new();
    for (row, key) in rows {
        let panels = panels(&runtime.borrow());
        let local = row.saturating_sub(paging.base);
        let local_panels: Vec<(u64, u64)> = panels
            .iter()
            .filter(|(open, _)| *open >= paging.base)
            .map(|(open, px)| (*open - paging.base, *px))
            .collect();
        let top = (local + 1) * row_height + grid::panel_px_before(&local_panels, local);
        let index = local + 1 + header_rows + grid::panels_before(&local_panels, local) + 1;
        let style = format!("transform: translateY({top}px);");

        let panel = if present.contains(&row) {
            root.query_selector(&format!("tr[data-detail-row=\"{row}\"]"))
                .ok()
                .flatten()
        } else {
            None
        };
        let panel = match panel {
            Some(panel) => panel,
            None => {
                let Some(panel) = build_panel(runtime, function, &document, row, &key, columns)
                else {
                    refused.push(row);
                    continue;
                };
                let _ = tbody.append_child(&panel);
                observe(host, &panel);
                panel
            }
        };
        if panel.get_attribute("style").as_deref() != Some(style.as_str()) {
            let _ = panel.set_attribute("style", &style);
        }
        let index = index.to_string();
        if panel.get_attribute("aria-rowindex").as_deref() != Some(index.as_str()) {
            let _ = panel.set_attribute("aria-rowindex", &index);
        }
        let title = format!(
            "{}: {}",
            texts(host).detail_panel,
            title_of(host, &runtime.borrow(), row)
        );
        // The row carries the name, not the cell: a cell named by an
        // `aria-label` other than its content fails WCAG 2.5.3.
        if panel.get_attribute("aria-label").as_deref() != Some(title.as_str()) {
            let _ = panel.set_attribute("aria-label", &title);
        }
        if let Ok(Some(cell)) = panel.query_selector("td") {
            let height = cell.get_bounding_client_rect().height().ceil().max(0.0) as u64;
            let mut borrowed = runtime.borrow_mut();
            if let Some(opened) = borrowed.details.iter_mut().find(|opened| opened.row == row)
                && opened.height != height
            {
                opened.height = height;
                remeasure = true;
            }
        }
    }
    if !refused.is_empty() {
        // The page has no panel for these records.
        let mut borrowed = runtime.borrow_mut();
        borrowed
            .details
            .retain(|opened| !refused.contains(&opened.row));
        remeasure = true;
    }
    remeasure
}

/// The row of a panel: the node from the cache, or the page's function's.
fn build_panel(
    runtime: &Rc<RefCell<GridRuntime>>,
    function: &js_sys::Function,
    document: &web_sys::Document,
    row: u64,
    key: &Json,
    columns: u32,
) -> Option<Element> {
    let name = name_of(key);
    let cached = runtime
        .borrow()
        .detail_nodes
        .iter()
        .find(|(cached, _)| *cached == name)
        .map(|(_, node)| node.clone());
    let node: Node = match cached.and_then(|node| node.dyn_into::<Node>().ok()) {
        Some(node) => node,
        None => {
            let record = record_of(&runtime.borrow(), row);
            let answer = function
                .call2(&JsValue::NULL, &record, &to_js(key))
                .unwrap_or(JsValue::NULL);
            let node: Node = if let Some(node) = answer.dyn_ref::<Node>() {
                node.clone()
            } else if let Some(text) = answer.as_string() {
                let span = document.create_element("div").ok()?;
                span.set_text_content(Some(&text));
                span.into()
            } else {
                return None;
            };
            runtime
                .borrow_mut()
                .detail_nodes
                .push((name, node.clone().into()));
            node
        }
    };
    let tr = document.create_element("tr").ok()?;
    let _ = tr.set_attribute("role", "row");
    let _ = tr.set_attribute("data-detail-row", &row.to_string());
    let td = document.create_element("td").ok()?;
    let _ = td.set_attribute("role", "gridcell");
    let _ = td.set_attribute("part", "detail");
    let _ = td.set_attribute("colspan", &columns.to_string());
    let _ = td.set_attribute("aria-colspan", &columns.to_string());
    let _ = td.append_child(&node);
    let _ = tr.append_child(&td);
    Some(tr)
}

/// The panel's cell is watched, so a node that grows later (content that
/// loads, a section that opens) moves the rows below it.
fn observe(host: &HtmlElement, panel: &Element) {
    let Ok(Some(td)) = panel.query_selector("td") else {
        return;
    };
    let existing = js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(OBSERVER))
        .ok()
        .and_then(|value| value.dyn_into::<web_sys::ResizeObserver>().ok());
    let observer = match existing {
        Some(observer) => observer,
        None => {
            let callback = Closure::<dyn FnMut(js_sys::Array)>::new(|entries: js_sys::Array| {
                let host = js_sys::Reflect::get(&entries.get(0), &JsValue::from_str("target"))
                    .ok()
                    .and_then(|target| target.dyn_into::<Element>().ok())
                    .and_then(|td| td.get_root_node().dyn_into::<ShadowRoot>().ok())
                    .and_then(|root| root.host().dyn_into::<HtmlElement>().ok());
                if let Some(host) = host {
                    follow_heights(&host);
                }
            })
            .into_js_value();
            let Ok(observer) = web_sys::ResizeObserver::new(callback.unchecked_ref()) else {
                return;
            };
            let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(OBSERVER), &observer);
            observer
        }
    };
    observer.observe(&td);
}

/// A panel changed its size: the heights are read again, and the frame follows
/// when one differs.
fn follow_heights(host: &HtmlElement) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let mut changed = false;
    if let Ok(panels) = root.query_selector_all(PANEL) {
        for index in 0..panels.length() {
            let Some(panel) = panels
                .item(index)
                .and_then(|n| n.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(row) = panel
                .get_attribute("data-detail-row")
                .and_then(|row| row.parse::<u64>().ok())
            else {
                continue;
            };
            let Ok(Some(td)) = panel.query_selector("td") else {
                continue;
            };
            let height = td.get_bounding_client_rect().height().ceil().max(0.0) as u64;
            let mut borrowed = runtime.borrow_mut();
            if let Some(opened) = borrowed.details.iter_mut().find(|opened| opened.row == row)
                && opened.height != height
            {
                opened.height = height;
                changed = true;
            }
        }
    }
    if changed {
        render(host, false);
    }
}

/// The row an open panel's pixel position puts the window at: the scroll
/// position through the panels above.
pub(super) fn row_at(runtime: &GridRuntime, y: u64) -> u64 {
    grid::row_at(&panels(runtime), runtime.row_height, y)
}

/// The pixel a row stands at, panels above it counted.
pub(super) fn top_of(runtime: &GridRuntime, row: u64) -> u64 {
    row.saturating_mul(runtime.row_height)
        .saturating_add(grid::panel_px_before(&panels(runtime), row))
}
