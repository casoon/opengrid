//! `<opengrid-grid>` — the registered element (point 16, browser only).
//!
//! The lifecycle mirrors [`crate::element`] (the table): on connect the host gets
//! an **open** shadow root, the empty `<table role="grid">` skeleton is rendered
//! as one patch list, the host `label` is mirrored to the table's `aria-label`
//! (E8/R6) and the provider attached with the exported `set_provider` runs the
//! first query.
//!
//! The interactive part lives here:
//!
//! * **Roving tabindex** — exactly one cell carries `tabindex="0"`; the element
//!   keeps the active cell and mirrors a data cell into
//!   [`GridState::set_focus`](opengrid_grid::GridState::set_focus), so the state
//!   machine owns the logical focus.
//! * **Keyboard matrix** — `keydown` on the shadow root decodes a
//!   [`GridKey`](crate::grid::GridKey) and asks the portable
//!   [`move_active`](crate::grid::move_active) for the next cell. A move inside
//!   the loaded page only moves the DOM focus; a move that leaves the page (page
//!   keys, `Ctrl+Home`/`Ctrl+End`) sets the window and re-runs the query.
//! * **Sorting** — `Enter`/`Space` on a header cell toggles the single-column
//!   sort through [`GridState::toggle_sort`](opengrid_grid::GridState::toggle_sort)
//!   and re-runs the query; `aria-sort` is rendered on the `<th>`. Paging needs a
//!   total order (rule S6), so the grid starts sorted by its first column and
//!   falls back to it when the user clears the sort
//!   ([`GridState::ensure_sorted`](opengrid_grid::GridState::ensure_sorted)).
//!
//! Rendering stays one patch list per transition: point 16 re-renders the loaded
//! page in full because there is no virtualization yet; point 17 replaces that
//! with recycled rows.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Element, Event, HtmlElement, KeyboardEvent, ScrollIntoViewOptions, ScrollLogicalPosition,
    ShadowRoot,
};

use opengrid_grid::{CellRef, GridState, Window};
use opengrid_web_core::element::{
    ARIA_LABEL_ATTRIBUTE, LABEL_ATTRIBUTE, attach_open_shadow_root, define, mirror_label,
};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::element::{apply, clear_root, describe};
use crate::grid::{
    self, ActiveCell, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, GRID_TAG, GridKey,
    PAGE_SIZE_ATTRIBUTE,
};

/// Registers `<opengrid-grid>`; safe to call more than once.
pub(crate) fn define_grid() -> Result<(), JsValue> {
    define(
        GRID_TAG,
        grid::OBSERVED,
        on_connected,
        on_disconnected,
        on_attribute_changed,
    )
}

/// Attaches a data provider to a grid host (point 16).
///
/// Shared with the table: [`crate::element::set_provider`] dispatches on the tag
/// name and calls this once the provider is stored.
pub(crate) fn start(host: &HtmlElement) {
    run_query(host, false);
}

/// The per-host interactive state.
///
/// `active` is the cell with the roving tabindex — a header cell or a logical
/// data cell; a data cell is mirrored into the [`GridState`] focus so the state
/// machine stays the owner of the logical focus.
struct GridRuntime {
    state: GridState,
    active: ActiveCell,
}

impl GridRuntime {
    /// Moves the active cell and keeps [`GridState`]'s focus in step.
    fn set_active(&mut self, active: ActiveCell) {
        self.active = active;
        self.state.set_focus(active.data());
    }
}

thread_local! {
    static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
    static RUNTIMES: RefCell<HashMap<u32, Rc<RefCell<GridRuntime>>>> =
        RefCell::new(HashMap::new());
}

/// The global symbol the runtime id is stored under (as the provider seam).
fn id_symbol() -> js_sys::Symbol {
    js_sys::Symbol::for_("opengrid.grid_id")
}

/// The runtime attached to `host`, if any.
fn runtime(host: &HtmlElement) -> Option<Rc<RefCell<GridRuntime>>> {
    let id = js_sys::Reflect::get(host.as_ref(), id_symbol().as_ref())
        .ok()
        .and_then(|value| value.as_f64())? as u32;
    RUNTIMES.with(|runtimes| runtimes.borrow().get(&id).cloned())
}

/// Stores `runtime` on `host` under a fresh id.
fn attach_runtime(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let id = NEXT_ID.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next += 1;
        id
    });
    RUNTIMES.with(|runtimes| runtimes.borrow_mut().insert(id, Rc::clone(runtime)));
    let _ = js_sys::Reflect::set(
        host.as_ref(),
        id_symbol().as_ref(),
        &JsValue::from_f64(f64::from(id)),
    );
}

/// Creates the runtime from the host attributes if it does not exist yet.
fn runtime_or_init(host: &HtmlElement) -> Rc<RefCell<GridRuntime>> {
    if let Some(runtime) = runtime(host) {
        return runtime;
    }
    let runtime = Rc::new(RefCell::new(fresh_runtime(host)));
    attach_runtime(host, &runtime);
    runtime
}

/// A fresh runtime: display schema from `columns`, window at offset 0 and the
/// top-left header cell active.
fn fresh_runtime(host: &HtmlElement) -> GridRuntime {
    let columns = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    let page_size = grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref());
    let mut state = GridState::new(grid::initial_schema(&columns));
    state.set_window(Window::new(0, page_size));
    // Paging needs a total order (rule S6), so the grid starts sorted by its
    // first column; the header shows it as `aria-sort="ascending"`.
    state.ensure_sorted();
    GridRuntime {
        state,
        active: ActiveCell::Header { col: 0 },
    }
}

/// Resets the runtime after a data attribute changed.
fn reset_runtime(host: &HtmlElement) {
    if let Some(runtime) = runtime(host) {
        let fresh = fresh_runtime(host);
        let mut runtime = runtime.borrow_mut();
        runtime.state = fresh.state;
        runtime.active = fresh.active;
    }
}

/// Renders the skeleton, installs the listeners and runs the first query.
fn on_connected(host: HtmlElement) {
    let Ok(root) = attach_open_shadow_root(&host) else {
        return;
    };
    if root.child_element_count() == 0 {
        add_listeners(&root);
    }
    let _ = runtime_or_init(&host);
    render(&host);
    run_query(&host, false);
}

/// Nothing to tear down: the root, its listeners and the runtime die with the
/// host (the runtime is a thread-local like the provider, point 13/14).
fn on_disconnected(_host: HtmlElement) {}

/// Re-mirrors `label`, resets and re-queries on a data attribute change.
fn on_attribute_changed(
    host: HtmlElement,
    name: String,
    _old_value: Option<String>,
    new_value: Option<String>,
) {
    match name.as_str() {
        LABEL_ATTRIBUTE => {
            if let Some(root) = host.shadow_root() {
                update_label(&root, new_value.as_deref());
            }
        }
        DATASOURCE_ATTRIBUTE | COLUMNS_ATTRIBUTE | PAGE_SIZE_ATTRIBUTE => {
            if host.shadow_root().is_none() {
                return;
            }
            reset_runtime(&host);
            render(&host);
            run_query(&host, false);
        }
        _ => {}
    }
}

/// Builds the whole loaded page as one patch list and applies it in one pass.
///
/// The active cell decides which cell carries `tabindex="0"`; the function does
/// not focus it (a background reload must not steal the page's focus). Callers
/// that act on a key focus afterwards with [`focus_active`].
fn render(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let runtime = runtime.borrow();

    clear_root(&root);
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    grid::build_grid(
        &mut buffer,
        &mut nodes,
        host.get_attribute(LABEL_ATTRIBUTE).as_deref(),
        &runtime.state,
        runtime.active,
        runtime.state.single_sort(),
    );
    apply(&root, document, &buffer);
}

/// Builds the query from the runtime and the host attributes and runs it.
///
/// Returns without doing anything if there is no provider, no shadow root or no
/// columns — the "not ready yet" states, not errors. `focus` re-focuses the
/// active cell once the result landed, so a keyboard-driven re-render does not
/// drop the focus.
pub(crate) fn run_query(host: &HtmlElement, focus: bool) {
    let Some(provider) = provider(host) else {
        return;
    };
    if host.shadow_root().is_none() {
        return;
    }
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let columns = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    if columns.is_empty() {
        return;
    }
    let Some(grid_runtime) = runtime(host) else {
        return;
    };

    let (sort, offset, page_size) = {
        let runtime = grid_runtime.borrow();
        (
            runtime
                .state
                .single_sort()
                .map(|(field, direction)| (field.to_owned(), direction)),
            runtime.state.window().offset,
            runtime.state.window().count,
        )
    };
    let query = grid::query_json(
        &source,
        &columns,
        sort.as_ref()
            .map(|(field, direction)| (field.as_str(), *direction)),
        offset,
        page_size,
    );
    let promise = provider.execute(&query);

    let host = host.clone();
    spawn_local(async move {
        match JsFuture::from(promise).await {
            Ok(value) => match value.as_string() {
                Some(json) => match grid::parse_result(&json) {
                    Ok(result) => {
                        if let Some(runtime) = runtime(&host) {
                            runtime.borrow_mut().state.apply_result(result);
                        }
                        render(&host);
                        if focus {
                            focus_active(&host);
                        }
                    }
                    Err(message) => render_error(&host, &message),
                },
                None => render_error(&host, "provider returned a non-string result"),
            },
            Err(value) => render_error(&host, &describe(&value)),
        }
    });
}

/// Clears the root and renders a short visible error (point 41 formalises this).
fn render_error(host: &HtmlElement, message: &str) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    clear_root(&root);
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    grid::build_error(&mut buffer, &mut nodes, message);
    apply(&root, document, &buffer);
}

/// Focuses the active cell (after a re-render) and scrolls it into view.
fn focus_active(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let active = runtime.borrow().active;
    focus_cell(&root, active);
}

/// Moves the DOM focus from `from` to `to` without re-rendering.
fn focus_from(root: &ShadowRoot, from: ActiveCell, to: ActiveCell) {
    if from != to {
        set_tabindex(root, from, "-1");
    }
    focus_cell(root, to);
}

/// Sets `tabindex="0"` on `to`, focuses it and scrolls it into view.
fn focus_cell(root: &ShadowRoot, to: ActiveCell) {
    set_tabindex(root, to, "0");
    let Ok(Some(target)) = root.query_selector(&selector(to)) else {
        return;
    };
    let Ok(target) = target.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = target.focus();
    let options = ScrollIntoViewOptions::new();
    options.set_block(ScrollLogicalPosition::Nearest);
    target.scroll_into_view_with_scroll_into_view_options(&options);
}

/// Sets the `tabindex` attribute of the DOM node of `active`.
fn set_tabindex(root: &ShadowRoot, active: ActiveCell, value: &str) {
    if let Ok(Some(node)) = root.query_selector(&selector(active)) {
        let _ = node.set_attribute("tabindex", value);
    }
}

/// The selector of the DOM node of `active` (its data attributes are rendered by
/// [`grid::build_grid`]).
fn selector(active: ActiveCell) -> String {
    match active {
        ActiveCell::Header { col } => format!("th[data-col=\"{col}\"]"),
        ActiveCell::Data(cell) => {
            format!("td[data-row=\"{}\"][data-col=\"{}\"]", cell.row, cell.col)
        }
    }
}

/// Reads the active cell back from a DOM cell (click or `focusin`).
fn active_from_element(element: &Element) -> Option<ActiveCell> {
    let col: usize = element.get_attribute("data-col")?.parse().ok()?;
    match element.tag_name().to_ascii_lowercase().as_str() {
        "th" => Some(ActiveCell::Header { col }),
        "td" => {
            let row: u64 = element.get_attribute("data-row")?.parse().ok()?;
            Some(ActiveCell::Data(CellRef::new(row, col)))
        }
        _ => None,
    }
}

/// Installs the delegated `keydown` and `focusin` listeners once.
fn add_listeners(root: &ShadowRoot) {
    let keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(on_key_down).into_js_value();
    let _ = root.add_event_listener_with_callback("keydown", keydown.unchecked_ref());
    let focusin = Closure::<dyn FnMut(Event)>::new(on_focus_in).into_js_value();
    let _ = root.add_event_listener_with_callback("focusin", focusin.unchecked_ref());
}

/// Handles the WAI-ARIA grid keyboard matrix
/// (plan/spezifikation/09-accessibility.md §Tastatur im Grid Mode).
///
/// `Tab`/`Shift+Tab` and any other key are left to the browser: the roving
/// tabindex makes the browser move focus out of the grid on its own.
fn on_key_down(event: KeyboardEvent) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let page_size = grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref());

    match (event.key().as_str(), event.ctrl_key()) {
        ("Enter", _) | (" ", _) => {
            event.prevent_default();
            activate_header(&host, &runtime, page_size);
        }
        ("Escape", _) => {
            event.prevent_default();
            escape_to_first(&host, &runtime, page_size);
        }
        ("ArrowUp", _) => move_with_key(&event, &host, &runtime, page_size, GridKey::ArrowUp),
        ("ArrowDown", _) => move_with_key(&event, &host, &runtime, page_size, GridKey::ArrowDown),
        ("ArrowLeft", _) => move_with_key(&event, &host, &runtime, page_size, GridKey::ArrowLeft),
        ("ArrowRight", _) => move_with_key(&event, &host, &runtime, page_size, GridKey::ArrowRight),
        ("Home", true) => move_with_key(&event, &host, &runtime, page_size, GridKey::CtrlHome),
        ("End", true) => move_with_key(&event, &host, &runtime, page_size, GridKey::CtrlEnd),
        ("Home", false) => move_with_key(&event, &host, &runtime, page_size, GridKey::Home),
        ("End", false) => move_with_key(&event, &host, &runtime, page_size, GridKey::End),
        ("PageUp", _) => move_with_key(&event, &host, &runtime, page_size, GridKey::PageUp),
        ("PageDown", _) => move_with_key(&event, &host, &runtime, page_size, GridKey::PageDown),
        _ => {}
    }
}

/// Moves focus for one navigation key, reloading the page only when the target
/// leaves the loaded window.
fn move_with_key(
    event: &KeyboardEvent,
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    page_size: u64,
    key: GridKey,
) {
    event.prevent_default();
    let (active, ncols, total_count, offset, loaded_rows) = {
        let runtime = runtime.borrow();
        (
            runtime.active,
            runtime.state.schema().len(),
            runtime.state.total_count(),
            runtime.state.window().offset,
            runtime.state.loaded_rows(),
        )
    };
    let next = grid::move_active(
        active,
        key,
        ncols,
        total_count,
        offset,
        loaded_rows,
        page_size,
    );
    let reload = grid::requested_offset(key, next, offset, total_count, page_size);

    runtime.borrow_mut().set_active(next);
    match reload {
        Some(new_offset) => {
            runtime
                .borrow_mut()
                .state
                .set_window(Window::new(new_offset, page_size));
            run_query(host, true);
        }
        None => {
            if let Some(root) = host.shadow_root() {
                focus_from(&root, active, next);
            }
        }
    }
}

/// `Enter`/`Space` on a header cell toggles its single-column sort and re-runs
/// the query; on a data cell it is a no-op.
fn activate_header(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, page_size: u64) {
    let active = runtime.borrow().active;
    let ActiveCell::Header { col } = active else {
        return;
    };
    let field = {
        let runtime = runtime.borrow();
        runtime
            .state
            .schema()
            .fields()
            .get(col)
            .map(|field| field.name.as_str().to_owned())
    };
    let Some(field) = field else {
        return;
    };
    {
        let mut runtime = runtime.borrow_mut();
        runtime.state.toggle_sort(&field);
        // Clearing the last sort falls back to the default first column so the
        // next page request still has a total order (rule S6).
        runtime.state.ensure_sorted();
        runtime.state.set_window(Window::new(0, page_size));
    }
    run_query(host, true);
}

/// `Escape` returns focus to the first cell of the grid (the top-left header
/// cell), loading the first page if the grid had scrolled on.
fn escape_to_first(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, page_size: u64) {
    let (from, offset, ncols) = {
        let runtime = runtime.borrow();
        (
            runtime.active,
            runtime.state.window().offset,
            runtime.state.schema().len(),
        )
    };
    if ncols == 0 {
        return;
    }
    let first = ActiveCell::Header { col: 0 };
    runtime.borrow_mut().set_active(first);
    if offset != 0 {
        runtime
            .borrow_mut()
            .state
            .set_window(Window::new(0, page_size));
        run_query(host, true);
    } else if let Some(root) = host.shadow_root() {
        focus_from(&root, from, first);
    }
}

/// Keeps the runtime in step when the user focuses a cell directly (mouse click
/// or `Tab` into the grid).
fn on_focus_in(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let Some(target) = event.target() else {
        return;
    };
    let Ok(target) = target.dyn_into::<Element>() else {
        return;
    };
    let Some(active) = active_from_element(&target) else {
        return;
    };
    let from = runtime.borrow().active;
    if from == active {
        return;
    }
    runtime.borrow_mut().set_active(active);
    focus_from(&root, from, active);
}

/// The shadow root a delegated listener was installed on.
fn current_shadow_root(event: &Event) -> Option<ShadowRoot> {
    event.current_target()?.dyn_into::<ShadowRoot>().ok()
}

/// Updates the existing table's `aria-label` after a label change.
fn update_label(root: &ShadowRoot, label: Option<&str>) {
    let Ok(Some(table)) = root.query_selector("table") else {
        return;
    };
    match mirror_label(label) {
        Some((name, value)) => {
            let _ = table.set_attribute(name, value);
        }
        None => {
            let _ = table.remove_attribute(ARIA_LABEL_ATTRIBUTE);
        }
    }
}
