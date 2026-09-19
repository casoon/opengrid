//! `<opengrid-grid>` — the registered element (points 16/17, browser only).
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
//!   the window only moves the DOM focus; a move that leaves it sets the window
//!   and re-runs the query, so the window always follows the focus.
//! * **Sorting** — `Enter`/`Space` on a header cell toggles the single-column
//!   sort through [`GridState::toggle_sort`](opengrid_grid::GridState::toggle_sort)
//!   and re-runs the query; `Shift`+`Enter`/`Space` adds/removes the column as an
//!   additional key through
//!   [`GridState::toggle_sort_multi`](opengrid_grid::GridState::toggle_sort_multi),
//!   preserving the key order (point 18). `aria-sort` is rendered on each `<th>`
//!   and a visible, `aria-hidden` index shows the multi-sort order. Paging needs
//!   a total order (rule S6), so the grid starts sorted by its first column and
//!   falls back to it when the user clears the sort
//!   ([`GridState::ensure_sorted`](opengrid_grid::GridState::ensure_sorted)).
//! * **Filtering** — the type-agnostic filter row (`part="filter"`) above the
//!   table has one operator `select` and one value `input` per column; `Enter` in
//!   a control applies the `and` of all non-empty entries as the query's
//!   `filter`, "Clear" empties it. The result count is shown in the status line.
//!   The controls are ordinary focusables outside the `role="grid"` table, so
//!   the roving tabindex and keyboard matrix are untouched.
//! * **Status** — the one `role="status"` line below the filter row carries
//!   every state of a query (point 41): "loading" while one runs, the result
//!   count, "no matches" for an empty result and a readable sentence when it
//!   failed. The wording and its language come from the component's texts
//!   (point 48). A failure no longer replaces the grid with an error paragraph — the
//!   table, its focus and the last loaded rows stay and only the status line
//!   changes, so a screen reader user is not dropped out of the grid they were
//!   navigating.
//!
//! # Virtualization (point 17)
//!
//! The skeleton ([`grid::build_grid`]) is built **once** and the element keeps
//! the [`Dom`] that created its nodes, so every later frame patches the same
//! nodes ([`grid::patch_grid`]) instead of rebuilding the table. A scroll on the
//! viewport maps to a new logical window (the portable arithmetic lives in
//! [`crate::grid`]); the provider is asked for exactly that window (`limit` =
//! pool, `offset` = window start) and the pool rows are recycled — no new nodes
//! appear per scroll step.
//!
//! Focus survives scrolling because [`grid::assign_pool`] pins the slot holding
//! the focused cell: [`grid::patch_grid`] leaves that slot completely untouched.
//! Scroll-driven queries are coalesced to one per animation frame and a
//! generation counter drops results whose window has already been superseded.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Element, Event, HtmlElement, HtmlInputElement, HtmlSelectElement, KeyboardEvent, Node,
    ScrollIntoViewOptions, ScrollLogicalPosition, ShadowRoot,
};

use opengrid_grid::{CellRef, GridState, GridStatus, Patch as GridPatch, Window};
use opengrid_query::CmpOp;
use opengrid_types::{DataType, Value};
use opengrid_web_core::element::{
    ARIA_LABEL_ATTRIBUTE, LABEL_ATTRIBUTE, attach_open_shadow_root, define, mirror_label,
};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::columns::{self, WIDTH_STEP};
use crate::formats::{CellFormat, Formatter, formats};
use opengrid_web_core::renderer::{Dom, WebRenderer};

use crate::element::{clear_root, describe};
use crate::grid::{
    self, ActiveCell, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, FilterEntry, GRID_TAG, GridKey,
    GridNodes, MODE_ATTRIBUTE, PAGE_SIZE_ATTRIBUTE, ROW_HEIGHT_PROPERTY, WINDOW_SIZE_ATTRIBUTE,
};
use crate::texts::texts;

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
    ensure_skeleton(host);
    render(host, false);
    run_query(host, QueryKind::Data, false);
}

/// The per-host interactive state.
///
/// `active` is the cell with the roving tabindex — a header cell or a logical
/// data cell; a data cell is mirrored into the [`GridState`] focus so the state
/// machine stays the owner of the logical focus.
struct GridRuntime {
    state: GridState,
    active: ActiveCell,
    /// The stable node ids of the one-time skeleton, if it was built.
    view: Option<GridNodes>,
    /// The DOM that created `view`'s nodes; kept so later frames patch them.
    dom: Option<Dom<WebRenderer>>,
    /// The scrollable viewport element (cached for scroll math).
    viewport: Option<Element>,
    /// The resolved pixel height of one logical row (`--grid-row-height`).
    ///
    /// Resolved per host from the computed style, so multiple grids can differ;
    /// the portable window math is parameterised by it.
    row_height: u64,
    /// Current slot → logical row assignment of the pool.
    slots: Vec<Option<u64>>,
    /// Window offset a coalesced scroll asked for, if any.
    pending_offset: Option<u64>,
    /// Whether a scroll query is scheduled for the next animation frame.
    raf_pending: bool,
    /// The 0-based page, while `page-size` is set (plan point 38).
    page: u64,
    /// Bumped per query; a result from an older generation is discarded.
    generation: u64,
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
    let columns = columns_of(host);
    let pool = pool_of(host);
    let row_height = resolve_row_height(host);
    let mut state = GridState::new(grid::initial_schema(&columns));
    state.set_window(Window::new(0, pool));
    // Paging needs a total order (rule S6), so the grid starts sorted by its
    // first column; the header shows it as `aria-sort="ascending"`.
    state.ensure_sorted();
    GridRuntime {
        state,
        active: ActiveCell::Header { col: 0 },
        view: None,
        dom: None,
        viewport: None,
        row_height,
        slots: vec![None; pool as usize],
        pending_offset: None,
        raf_pending: false,
        page: 0,
        generation: 0,
    }
}

/// Resolves the `--grid-row-height` custom property on the host.
///
/// Reads the host's computed style (the shadow stylesheet seeds the property
/// with the default, a document/inline rule on the host overrides it and the
/// value inherits into the shadow tree) and parses a `<number>px` value. An
/// absent or invalid value falls back to [`grid::DEFAULT_ROW_HEIGHT`].
fn resolve_row_height(host: &HtmlElement) -> u64 {
    let raw = web_sys::window()
        .and_then(|window| window.get_computed_style(host).ok().flatten())
        .and_then(|style| style.get_property_value(ROW_HEIGHT_PROPERTY).ok())
        .unwrap_or_default();
    grid::parse_row_height(&raw)
}

/// Resets the runtime after a data attribute changed and drops the skeleton.
fn reset_runtime(host: &HtmlElement) {
    if let Some(runtime) = runtime(host) {
        let fresh = fresh_runtime(host);
        let mut runtime = runtime.borrow_mut();
        runtime.state = fresh.state;
        runtime.active = fresh.active;
        runtime.view = None;
        runtime.dom = None;
        runtime.viewport = None;
        runtime.slots = fresh.slots;
        runtime.pending_offset = None;
        runtime.raf_pending = false;
        runtime.generation = 0;
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
    let runtime = runtime_or_init(&host);
    // Re-resolve the row height on every (re)connect: the shadow stylesheet (and
    // any host override) is in place by now.
    runtime.borrow_mut().row_height = resolve_row_height(&host);
    ensure_skeleton(&host);
    render(&host, false);
    run_query(&host, QueryKind::Data, false);
}

/// Nothing to tear down: the root, its listeners and the runtime die with the
/// host (the runtime is a thread-local like the provider, point 13/14).
fn on_disconnected(_host: HtmlElement) {}

/// Rebuilds the grid after its texts changed (point 48).
///
/// The labels of the filter row and the operator names sit in the one-time
/// skeleton, so new texts mean a new skeleton — and a rebuild that only kept the
/// *model* would leave the user somewhere else entirely: the new filter row
/// would be empty while the query still filters, the new viewport would sit at
/// the top while the window is at row 500, and the focused cell would be gone
/// from the DOM. So everything the user can see is carried across: the filter
/// controls are read out of the old row and written into the new one, the scroll
/// offset is restored, and the active cell takes the focus back.
pub(crate) fn retext(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let columns = columns_of(host);
    let entries = read_filter_entries(&root, &columns);
    let scroll_top = runtime
        .borrow()
        .viewport
        .as_ref()
        .map(|viewport| viewport.scroll_top());

    clear_root(&root);
    {
        let mut runtime = runtime.borrow_mut();
        runtime.view = None;
        runtime.dom = None;
        runtime.viewport = None;
    }
    ensure_skeleton(host);
    if let Some(root) = host.shadow_root() {
        write_filter_entries(&root, &entries);
    }
    if let (Some(top), Some(viewport)) = (scroll_top, runtime.borrow().viewport.clone()) {
        viewport.set_scroll_top(top);
    }
    render(host, true);
    run_query(host, QueryKind::Data, true);
}

/// Writes `entries` back into the filter row after a rebuild.
///
/// The counterpart of [`read_filter_entries`]: the state keeps the filter as an
/// expression, but the controls are what the user reads, and an empty row above
/// filtered data is a lie.
fn write_filter_entries(root: &ShadowRoot, entries: &[FilterEntry]) {
    for (col, entry) in entries.iter().enumerate() {
        if let Ok(Some(node)) = root.query_selector(&format!("select[data-col=\"{col}\"]"))
            && let Ok(select) = node.dyn_into::<HtmlSelectElement>()
        {
            select.set_value(entry.op.as_str());
        }
        if let Ok(Some(node)) = root.query_selector(&format!("input[data-col=\"{col}\"]"))
            && let Ok(input) = node.dyn_into::<HtmlInputElement>()
        {
            input.set_value(&entry.value);
        }
    }
}

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
        PAGE_SIZE_ATTRIBUTE => {
            // Switching between paging and scrolling changes what the window
            // means, so the grid starts at the first page either way.
            if let Some(runtime) = runtime(&host) {
                runtime.borrow_mut().page = 0;
            }
            run_query(&host, QueryKind::Data, false);
        }
        DATASOURCE_ATTRIBUTE | COLUMNS_ATTRIBUTE | WINDOW_SIZE_ATTRIBUTE | MODE_ATTRIBUTE => {
            let Some(root) = host.shadow_root() else {
                return;
            };
            clear_root(&root);
            reset_runtime(&host);
            ensure_skeleton(&host);
            render(&host, false);
            run_query(&host, QueryKind::Data, false);
        }
        _ => {}
    }
}

/// Builds the one-time skeleton and stores its nodes and the DOM.
fn ensure_skeleton(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    if runtime.borrow().view.is_some() {
        return;
    }
    let Some(document) = host.owner_document() else {
        return;
    };
    let pool = pool_of(host) as usize;
    let label = host.get_attribute(LABEL_ATTRIBUTE);
    let schema = runtime.borrow().state.schema().clone();

    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    // The visibility group lists **every** declared column, the hidden ones
    // included — that is the way back (point 36).
    let declared: Vec<(String, bool)> = {
        let layout = columns::layout(host);
        let layout = layout.borrow();
        grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref())
            .into_iter()
            .map(|name| {
                let visible = !layout.is_hidden(&name);
                (name, visible)
            })
            .collect()
    };
    let view = grid::build_grid(
        &mut buffer,
        &mut nodes,
        label.as_deref(),
        &schema,
        pool,
        &texts(host),
        &declared,
    );
    let root_node: Node = root.clone().unchecked_into();
    let mut dom = Dom::new(WebRenderer::from_document(document), root_node);
    dom.apply_buffer(&buffer);
    let viewport = dom.node(view.viewport).clone().dyn_into::<Element>().ok();

    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.slots = vec![None; pool];
        borrowed.viewport = viewport;
        borrowed.dom = Some(dom);
        borrowed.view = Some(view);
    }
    // The reader's widths live on the header cells and survive the frames that
    // follow; a fresh skeleton has to get them back (point 36).
    apply_widths(host);
}

/// Computes the current window assignment and applies it as one patch list.
///
/// The focused slot is pinned (see [`grid::assign_pool`]); the function never
/// rebuilds the table. `focus_after` re-focuses the active cell once the frame
/// landed, so a keyboard-driven reload does not drop the focus.
fn render(host: &HtmlElement, focus_after: bool) {
    ensure_skeleton(host);
    let Some(runtime) = runtime(host) else {
        return;
    };

    let paging = paging_of(host, &runtime);
    let (sorts, slots, pinned_slot) = {
        let borrowed = runtime.borrow();
        let Some(view) = borrowed.view.as_ref() else {
            return;
        };
        let pool = view.pool();
        let total = borrowed.state.total_count();
        let window = borrowed.state.window();
        let rows = grid::window_rows(window.offset, total, pool as u64);
        let focus = borrowed.active.data().map(|cell| cell.row);
        // The pin keeps the focused row from being recycled out from under the
        // focus while **scrolling** (point 17). A page change replaces every
        // row, so there is nothing to protect — and pinning would leave the
        // focused slot showing the old page's cell (point 38).
        let pinned_slot = if paging.size.is_some() {
            None
        } else {
            focus.and_then(|row| borrowed.slots.iter().position(|slot| *slot == Some(row)))
        };
        let slots = grid::assign_pool(&borrowed.slots, focus, &rows, pool);
        let sorts = borrowed.state.sort_keys();
        (sorts, slots, pinned_slot)
    };

    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.slots = slots;
    }
    {
        let mut borrowed = runtime.borrow_mut();
        let borrowed = &mut *borrowed;
        let Some(view) = borrowed.view.as_ref() else {
            return;
        };
        let mut buffer = PatchBuffer::new();
        grid::patch_grid(
            &mut buffer,
            view,
            &borrowed.state,
            &borrowed.slots,
            borrowed.active,
            &sorts,
            pinned_slot,
            borrowed.row_height,
            &texts(host),
            &Formatter::new(&formats(host), borrowed.state.schema()),
            paging,
        );
        if let Some(dom) = borrowed.dom.as_mut() {
            dom.apply_buffer(&buffer);
        }
    }

    if let Some(root) = host.shadow_root() {
        let schema = runtime.borrow().state.schema().clone();
        fix_operator_choices(&root, &schema);
    }

    if focus_after {
        focus_active(host);
    }
}

/// Why a query runs — which decides whether it announces "loading" (point 41).
///
/// A [`Window`](QueryKind::Window) query re-fetches rows of the *same* result
/// set: scrolling fires one per animation frame, and announcing each would turn
/// the polite live region into chatter while the user reads. Only a
/// [`Data`](QueryKind::Data) query — the first load, a sort, a filter, an
/// attribute change — changes what the result *is*, and that is worth
/// announcing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryKind {
    /// The result set changes: announce that the grid is loading.
    Data,
    /// Only the window moves: keep the current status.
    Window,
}

/// Builds the query from the runtime and the host attributes and runs it.
///
/// Returns without doing anything if there is no provider, no shadow root or no
/// columns — the "not ready yet" states, not errors; the status line stays at
/// "loading", which is what the grid is in fact waiting for. `focus` re-focuses
/// the active cell once the result landed, so a keyboard-driven re-render does
/// not drop the focus.
/// Puts the window on the current page, so the query asks for exactly it.
///
/// Paging and virtualizing are exclusive: while `page-size` is set the window
/// **is** the page, and nothing scrolls it (plan point 38).
fn sync_page_window(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let Some(size) = grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref())
    else {
        return;
    };
    let mut borrowed = runtime.borrow_mut();
    let page = borrowed.page;
    borrowed.state.set_window(Window::new(page * size, size));
}

pub(crate) fn run_query(host: &HtmlElement, kind: QueryKind, focus: bool) {
    let Some(provider) = provider(host) else {
        return;
    };
    if host.shadow_root().is_none() {
        return;
    }
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let columns = columns_of(host);
    if columns.is_empty() {
        return;
    }
    let Some(grid_runtime) = runtime(host) else {
        return;
    };
    let pool = pool_of(host);
    sync_page_window(host, &grid_runtime);
    let pool =
        grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).unwrap_or(pool);

    let (sorts, filter, offset, generation) = {
        let mut runtime = grid_runtime.borrow_mut();
        let generation = runtime.generation + 1;
        runtime.generation = generation;
        (
            runtime.state.sort_keys(),
            runtime.state.filter().cloned(),
            runtime.state.window().offset,
            generation,
        )
    };
    if kind == QueryKind::Data {
        grid_runtime
            .borrow_mut()
            .state
            .set_status(GridStatus::Loading);
        render(host, false);
    }

    let query = grid::query_json(&source, &columns, &sorts, filter.as_ref(), offset, pool);
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let promise = provider.execute(&query, &mode);

    let host = host.clone();
    spawn_local(async move {
        let outcome = match JsFuture::from(promise).await {
            Ok(value) => match value.as_string() {
                Some(json) => grid::parse_result(&json),
                None => Err("provider returned a non-string result".to_owned()),
            },
            Err(value) => Err(describe(&value)),
        };
        settle(&host, generation, outcome, focus);
    });
}

/// Applies a finished query to the runtime and renders one frame.
///
/// Success and failure take the same path on purpose: both are a status the
/// state machine owns, both are dropped when a newer query has superseded this
/// one (the generation check), and both end in exactly one [`render`]. An error
/// therefore leaves the table, the roving tabindex and the last loaded rows in
/// place — only the status line changes.
fn settle(
    host: &HtmlElement,
    generation: u64,
    outcome: Result<opengrid_datasource::QueryResult, String>,
    focus: bool,
) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    {
        let mut runtime = runtime.borrow_mut();
        // A newer scroll, key or filter superseded this query.
        if runtime.generation != generation {
            return;
        }
        match outcome {
            Ok(result) => {
                runtime.state.apply_result(result);
            }
            Err(cause) => {
                runtime
                    .state
                    .set_status(GridStatus::Error(texts(host).error(&cause)));
            }
        }
    }
    render(host, focus);
}

/// Focuses the active cell (after a re-render) and scrolls it into view.
fn focus_active(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let (active, viewport) = {
        let runtime = runtime.borrow();
        (runtime.active, runtime.viewport.clone())
    };
    focus_cell(&root, viewport.as_ref(), active);
}

/// Moves the DOM focus from `from` to `to` without re-rendering.
fn focus_from(root: &ShadowRoot, viewport: Option<&Element>, from: ActiveCell, to: ActiveCell) {
    if from != to {
        set_tabindex(root, from, "-1");
    }
    focus_cell(root, viewport, to);
}

/// Sets `tabindex="0"` on `to`, focuses it and scrolls it into view.
///
/// A header cell is always at the top of the viewport (the `<thead>` is sticky),
/// so moving to it scrolls the viewport back to the first row.
fn focus_cell(root: &ShadowRoot, viewport: Option<&Element>, to: ActiveCell) {
    set_tabindex(root, to, "0");
    if let (ActiveCell::Header { .. }, Some(viewport)) = (to, viewport) {
        viewport.set_scroll_top(0);
    }
    let Ok(Some(target)) = root.query_selector(&selector(to)) else {
        return;
    };
    let Ok(target) = target.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = target.focus();
    let options = ScrollIntoViewOptions::new();
    // `nearest` scrolls the minimum. For a cell that unfolded past the height of
    // the viewport (point 47) that still aligns its **top**, and the cells'
    // `scroll-margin-top` keeps that top clear of the sticky header — so the
    // value is read from its first line and the rest is scrolled to. Measured,
    // not assumed: without the scroll margin the first lines end up above the
    // header, with it they start right below it.
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
/// [`grid::patch_grid`]).
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

/// Installs the delegated `keydown`, `focusin`, `click` and capture-phase
/// `scroll` listeners once.
fn add_listeners(root: &ShadowRoot) {
    let keydown = Closure::<dyn FnMut(KeyboardEvent)>::new(on_key_down).into_js_value();
    let _ = root.add_event_listener_with_callback("keydown", keydown.unchecked_ref());
    let focusin = Closure::<dyn FnMut(Event)>::new(on_focus_in).into_js_value();
    let _ = root.add_event_listener_with_callback("focusin", focusin.unchecked_ref());
    // The "Clear" button of the filter row needs a click handler; keyboard
    // activation fires a click too, so one listener covers both.
    let click = Closure::<dyn FnMut(Event)>::new(on_filter_clear).into_js_value();
    let _ = root.add_event_listener_with_callback("click", click.unchecked_ref());
    // Choosing an operator changes what the value field means — "has no value"
    // takes none — so the controls follow the choice at once instead of at the
    // next frame (point 51).
    let change = Closure::<dyn FnMut(Event)>::new(on_filter_change).into_js_value();
    let _ = root.add_event_listener_with_callback("change", change.unchecked_ref());
    // Scroll does not bubble, but a capture listener on the shadow root sees the
    // viewport's scroll events.
    let scroll = Closure::<dyn FnMut(Event)>::new(on_scroll).into_js_value();
    let _ = root.add_event_listener_with_callback_and_bool("scroll", scroll.unchecked_ref(), true);
}

/// Handles the WAI-ARIA grid keyboard matrix
/// (plan/spezifikation/09-accessibility.md §Tastatur im Grid Mode).
///
/// Keys typed into a filter control are left to that control; only `Enter` in a
/// value/operator control applies the filter (the clear button keeps its native
/// click). `Tab`/`Shift+Tab` and any other key are left to the browser: the
/// roving tabindex makes the browser move focus out of the grid on its own.
fn on_key_down(event: KeyboardEvent) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && is_filter_control(&target)
    {
        // The operator select is driven directly: native `<select>` keyboard
        // behaviour differs per platform (on macOS the popup needs opening
        // first), so the component implements the standard arrow/Home/End moves
        // itself. This keeps the filter row operable the same way everywhere.
        if target.tag_name().eq_ignore_ascii_case("select")
            && let Ok(select) = target.clone().dyn_into::<HtmlSelectElement>()
        {
            let last = select.length().saturating_sub(1) as i32;
            let selected = select.selected_index();
            match event.key().as_str() {
                "ArrowDown" => {
                    event.prevent_default();
                    select.set_selected_index((selected + 1).min(last));
                }
                "ArrowUp" => {
                    event.prevent_default();
                    select.set_selected_index((selected - 1).max(0));
                }
                "Home" => {
                    event.prevent_default();
                    select.set_selected_index(0);
                }
                "End" => {
                    event.prevent_default();
                    select.set_selected_index(last);
                }
                _ => {}
            }
        }
        // A button (the clear control) keeps its native Enter/Space activation.
        if event.key() == "Enter" && !target.tag_name().eq_ignore_ascii_case("button") {
            event.prevent_default();
            if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
                apply_filters(&host);
            }
        }
        return;
    }
    // The paging buttons sit outside the grid table and keep their **native**
    // keyboard activation: the grid's matrix below would call
    // `prevent_default()` on `Enter` and swallow the click (point 38).
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target.closest("[part=\"pager\"]").ok().flatten().is_some()
    {
        return;
    }

    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let viewport_rows = viewport_rows(&runtime);

    // Keys inside an open editor belong to the editor (point 37): `Enter`
    // commits, `Escape` discards, everything else is ordinary typing. The grid
    // matrix below would eat the arrow keys mid-word.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target.closest("[part=\"editor\"]").ok().flatten().is_some()
    {
        match event.key().as_str() {
            "Enter" => {
                event.prevent_default();
                end_edit(&host, true);
            }
            "Escape" => {
                event.prevent_default();
                end_edit(&host, false);
            }
            _ => {}
        }
        return;
    }

    // Column operations live on the header cell and are **keyboard first**:
    // WCAG 2.5.7 requires that everything reachable by dragging is reachable
    // without it, so this is the primary way, not a fallback (point 36).
    // Read the active cell out first: a `borrow()` held across the body makes
    // the `borrow_mut` inside panic, and a panic in an event handler is a
    // silently dead key.
    let active = runtime.borrow().active;
    if (event.ctrl_key() || event.meta_key())
        && matches!(event.key().as_str(), "ArrowLeft" | "ArrowRight")
        && let ActiveCell::Header { col } = active
    {
        event.prevent_default();
        let by = if event.key() == "ArrowLeft" { -1 } else { 1 };
        if event.shift_key() {
            resize_column(&host, col, by * WIDTH_STEP as i32);
        } else {
            move_column(&host, col, by);
        }
        return;
    }

    // `Ctrl`/`Cmd`+`A` selects every matching row, not only the loaded page.
    if event.key().eq_ignore_ascii_case("a") && (event.ctrl_key() || event.meta_key()) {
        event.prevent_default();
        let patches = runtime.borrow_mut().state.select_all();
        settle_selection(&host, &runtime, patches);
        return;
    }

    match (event.key().as_str(), event.ctrl_key()) {
        ("Enter", _) => {
            event.prevent_default();
            // On a header `Enter` sorts, as it always has; on a data cell it
            // opens the editor — the meaning point 16 left free for this.
            match active {
                ActiveCell::Header { .. } => activate_header(&host, &runtime, event.shift_key()),
                ActiveCell::Data(cell) => begin_edit(&host, cell),
            }
        }
        (" ", _) => {
            event.prevent_default();
            // On a header `Space` sorts, as it always has; on a data cell it
            // now selects the row (point 35). `Shift` extends from the anchor.
            match active {
                ActiveCell::Header { .. } => {
                    activate_header(&host, &runtime, event.shift_key());
                }
                ActiveCell::Data(cell) => {
                    let patches = {
                        let mut runtime = runtime.borrow_mut();
                        if event.shift_key() {
                            runtime.state.extend_selection(cell.row)
                        } else {
                            runtime.state.toggle_selection(cell.row)
                        }
                    };
                    settle_selection(&host, &runtime, patches);
                }
            }
        }
        ("Escape", _) => {
            event.prevent_default();
            escape_to_first(&host, &runtime);
        }
        ("ArrowUp", _) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::ArrowUp),
        ("ArrowDown", _) => {
            move_with_key(&event, &host, &runtime, viewport_rows, GridKey::ArrowDown)
        }
        ("ArrowLeft", _) => {
            move_with_key(&event, &host, &runtime, viewport_rows, GridKey::ArrowLeft)
        }
        ("ArrowRight", _) => {
            move_with_key(&event, &host, &runtime, viewport_rows, GridKey::ArrowRight)
        }
        ("Home", true) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::CtrlHome),
        ("End", true) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::CtrlEnd),
        ("Home", false) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::Home),
        ("End", false) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::End),
        ("PageUp", _) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::PageUp),
        ("PageDown", _) => move_with_key(&event, &host, &runtime, viewport_rows, GridKey::PageDown),
        _ => {}
    }
}

/// Moves focus for one navigation key, reloading the window only when the target
/// leaves it (the window follows the focus).
fn move_with_key(
    event: &KeyboardEvent,
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    viewport_rows: u64,
    key: GridKey,
) {
    event.prevent_default();
    let (active, ncols, total_count, window, pool) = {
        let runtime = runtime.borrow();
        let pool = runtime
            .view
            .as_ref()
            .map(|view| view.pool() as u64)
            .unwrap_or_else(|| pool_of(host));
        (
            runtime.active,
            runtime.state.schema().len(),
            runtime.state.total_count(),
            runtime.state.window(),
            pool,
        )
    };
    let next = grid::move_active(active, key, ncols, total_count, viewport_rows);
    let reload = grid::requested_window(key, next, window, total_count, pool);

    runtime.borrow_mut().set_active(next);
    match reload {
        Some(new_offset) => {
            runtime
                .borrow_mut()
                .state
                .set_window(Window::new(new_offset, pool));
            run_query(host, QueryKind::Window, true);
        }
        None => {
            if let Some(root) = host.shadow_root() {
                let viewport = runtime.borrow().viewport.clone();
                focus_from(&root, viewport.as_ref(), active, next);
            }
        }
    }
}

/// Fires the selection event when the state has just dropped the selection.
///
/// Sorting and filtering do it as a side effect, so the page would otherwise
/// keep acting on rows that are no longer selected.
fn announce_if_cleared(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    if runtime.borrow().state.selection().is_empty() {
        dispatch_selection(host, &[]);
    }
}

/// Makes a column wider or narrower and says so.
///
/// The width is written onto the header cell; `table-layout: fixed` carries it
/// to every row. The value is clamped so a column can never be resized into
/// something nobody can find again (and below the 24 px of WCAG 2.5.8).
fn resize_column(host: &HtmlElement, col: usize, step: i32) {
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
}

/// Moves a column one place and re-runs the query.
///
/// The order is part of the query's projection, so a move is a new query — the
/// same path a sort takes, and for the same reason: the result has a different
/// shape.
fn move_column(host: &HtmlElement, col: usize, by: i32) {
    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    let columns = columns_of(host);
    let Some(name) = columns.get(col).cloned() else {
        return;
    };
    let moved = columns::layout(host)
        .borrow_mut()
        .move_column(&declared, &name, by);
    let Some(order) = moved else {
        // At the end: nothing happens, and the reader is told why instead of
        // pressing the key again.
        announce(host, &texts(host).column_at_edge(&name));
        return;
    };
    let position = order.iter().position(|column| *column == name).unwrap_or(0);
    // Rebuild **first**: it replaces the runtime, so anything set before — the
    // active cell, a notice — would be thrown away with the old one.
    rebuild(host);
    // The focus follows the column, not the place it left. The query that the
    // rebuild started focuses the active cell when its result lands.
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().active = ActiveCell::Header { col: position };
    }
    announce(
        host,
        &texts(host).column_moved(&name, position as u64 + 1, order.len() as u64),
    );
}

/// Shows or hides a column and re-runs the query.
fn set_column_hidden(host: &HtmlElement, name: &str, hidden: bool) {
    if !columns::update(host, |layout| layout.set_hidden(name, hidden)) {
        return;
    }
    let visible = columns_of(host).len() as u64;
    let declared =
        grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref()).len() as u64;
    // Rebuild **first**: it replaces the runtime, and a notice set before would
    // be thrown away with the old state.
    rebuild(host);
    announce(
        host,
        &texts(host).column_visibility(name, hidden, visible, declared),
    );
}

/// Rebuilds the skeleton because the set or order of columns changed.
fn rebuild(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    clear_root(&root);
    reset_runtime(host);
    ensure_skeleton(host);
    render(host, false);
    run_query(host, QueryKind::Data, true);
}

/// Puts a one-off sentence into the status line.
///
/// The one polite live region the grid has (point 41) — a column operation is
/// a change only the sighted see, so it has to be said.
fn announce(host: &HtmlElement, message: &str) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    runtime.borrow_mut().state.set_notice(message.to_owned());
    render(host, false);
}

/// The rendered width of a header cell, if it has been laid out.
fn measured_width(host: &HtmlElement, col: usize) -> Option<u32> {
    let root = host.shadow_root()?;
    let cell = root
        .query_selector(&format!("th[data-col=\"{col}\"]"))
        .ok()
        .flatten()?;
    let width = cell.get_bounding_client_rect().width();
    (width > 0.0).then_some(width as u32)
}

/// Writes the reader's widths onto the header cells.
fn apply_widths(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let layout = columns::layout(host);
    let layout = layout.borrow();
    for (col, name) in columns_of(host).iter().enumerate() {
        let Ok(Some(cell)) = root.query_selector(&format!("th[data-col=\"{col}\"]")) else {
            continue;
        };
        match layout.width(name) {
            Some(width) => {
                let _ = cell.set_attribute("style", &format!("width: {width}px;"));
            }
            None => {
                let _ = cell.remove_attribute("style");
            }
        }
    }
}

/// The columns the grid actually shows, in the order it shows them.
///
/// The `columns` attribute says which columns exist; the reader's layout
/// (point 36) says which of them are visible and in what order. One function,
/// so the query, the skeleton and the render can never disagree.
fn columns_of(host: &HtmlElement) -> Vec<String> {
    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    columns::layout(host).borrow().effective(&declared)
}

/// How many DOM row slots the grid keeps.
///
/// While paging that is the **page size**: the pool is the page, because there
/// is nothing to scroll and every row of the page is on screen. While
/// virtualizing it is `window-size`, the recycled pool of point 17. One
/// function, so the skeleton, the window and the query can never disagree about
/// how many rows there are.
fn pool_of(host: &HtmlElement) -> u64 {
    grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).unwrap_or_else(|| {
        grid::parse_window_size(host.get_attribute(WINDOW_SIZE_ATTRIBUTE).as_deref())
    })
}

/// A click on one of the paging buttons (plan point 38).
///
/// Returns whether it was one, so the shared click listener can stop there.
fn on_pager_click(host: &HtmlElement, target: &Element) -> bool {
    let Ok(Some(button)) = target.closest("button[part^=\"page-\"]") else {
        return false;
    };
    let Some(part) = button.get_attribute("part") else {
        return false;
    };
    let Some(size) = grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref())
    else {
        return false;
    };
    let Some(runtime) = runtime(host) else {
        return false;
    };

    let pages = {
        let borrowed = runtime.borrow();
        grid::Paging::count(borrowed.state.total_count(), size)
    };
    let current = runtime.borrow().page;
    let next = match part.as_str() {
        "page-first" => 0,
        "page-previous" => current.saturating_sub(1),
        "page-next" => (current + 1).min(pages - 1),
        "page-last" => pages - 1,
        _ => return false,
    };
    if next == current {
        return true;
    }
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.page = next;
        // The focus goes to the first cell of the new page: leaving it on a row
        // that no longer exists drops it to the document.
        borrowed.active = ActiveCell::Data(CellRef::new(next * size, 0));
    }
    // A page change is a different set of rows, so it announces (point 41).
    run_query(host, QueryKind::Data, true);
    true
}

/// What the grid is rendering: one page, or the whole virtualized result.
fn paging_of(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) -> grid::Paging {
    let total = runtime.borrow().state.total_count();
    match grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()) {
        Some(size) => grid::Paging::page(runtime.borrow().page, size, total),
        None => grid::Paging::whole(total),
    }
}

/// Draws the grid again without asking the source anything.
///
/// What changed is how the values are written, not which ones there are
/// (point 42).
pub(crate) fn rerender(host: &HtmlElement) {
    render(host, false);
}

/// Renders a changed selection and tells the page about it.
///
/// One place, so the event and what is on screen can never disagree: whatever
/// changed the selection produces patches, and this turns them into both.
fn settle_selection(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    patches: Vec<GridPatch>,
) {
    let Some(rows) = patches.into_iter().find_map(|patch| match patch {
        GridPatch::Selection(rows) => Some(rows),
        _ => None,
    }) else {
        return;
    };
    render(host, false);
    dispatch_selection(host, &rows);
    let _ = runtime;
}

/// The choices a page supplied for a column, turning its editor into a select.
///
/// V1 has no enum type, so a `<select>` has no source of options in the schema —
/// the page is the only honest one (plan point 37).
fn choices_for(host: &HtmlElement, col: usize) -> Option<Vec<String>> {
    let name = columns_of(host).get(col)?.clone();
    let value =
        js_sys::Reflect::get(host.as_ref(), &JsValue::from_str("__opengridChoices")).ok()?;
    let list = js_sys::Reflect::get(&value, &JsValue::from_str(&name)).ok()?;
    let array = list.dyn_into::<js_sys::Array>().ok()?;
    let options: Vec<String> = array.iter().filter_map(|item| item.as_string()).collect();
    (!options.is_empty()).then_some(options)
}

/// Opens the editor on the focused data cell (plan point 37).
///
/// The editor is an ordinary form control inside the cell, typed after the
/// column: `text`, `number` with the column's step, `date`, `checkbox`, or a
/// `<select>` when the page supplied choices. The same derivation as the filter
/// row (point 51) — one place decides what a column can hold.
fn begin_edit(host: &HtmlElement, cell: CellRef) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let (data_type, current) = {
        let borrowed = runtime.borrow();
        let Some(field) = borrowed.state.schema().fields().get(cell.col) else {
            return;
        };
        let current = borrowed
            .state
            .cell(cell)
            .map(crate::formats::plain_text)
            .unwrap_or_default();
        (field.data_type, current)
    };
    if runtime.borrow_mut().state.begin_edit(cell).is_empty() {
        return;
    }
    render(host, false);

    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };

    td.set_text_content(None);
    let choices = choices_for(host, cell.col);
    let editor: Element = match &choices {
        Some(options) => {
            let Ok(select) = document.create_element("select") else {
                return;
            };
            for option in options {
                if let Ok(node) = document.create_element("option") {
                    node.set_text_content(Some(option));
                    let _ = node.set_attribute("value", option);
                    if *option == current {
                        let _ = node.set_attribute("selected", "");
                    }
                    let _ = select.append_child(&node);
                }
            }
            select
        }
        None => {
            let Ok(input) = document.create_element("input") else {
                return;
            };
            let kind = grid::input_type(data_type);
            let _ = input.set_attribute("type", kind);
            if let Some(step) = grid::input_step(data_type) {
                let _ = input.set_attribute("step", &step);
            }
            if kind == "checkbox" {
                if current == "true" {
                    let _ = input.set_attribute("checked", "");
                }
            } else if let Ok(field) = input.clone().dyn_into::<HtmlInputElement>() {
                field.set_value(&current);
            }
            input
        }
    };
    let _ = editor.set_attribute("part", "editor");
    // The column name is the accessible name: the cell it sits in has none.
    if let Some(field) = runtime.borrow().state.schema().fields().get(cell.col) {
        let _ = editor.set_attribute("aria-label", field.name.as_str());
    }
    let _ = td.append_child(&editor);
    if let Ok(element) = editor.dyn_into::<HtmlElement>() {
        let _ = element.focus();
    }
}

/// Closes the editor, optionally taking what it holds (plan point 37).
///
/// An input the column cannot hold is **refused and announced** — never
/// silently rounded, never silently dropped. Whatever happens, the focus goes
/// back to the cell: an editor that vanishes and leaves the focus on the
/// document is a dead end for a keyboard.
fn end_edit(host: &HtmlElement, commit: bool) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(cell) = runtime.borrow().state.editing() else {
        return;
    };
    let Some(root) = host.shadow_root() else {
        return;
    };

    let typed = root
        .query_selector("[part=\"editor\"]")
        .ok()
        .flatten()
        .map(
            |editor| match editor.clone().dyn_into::<HtmlInputElement>() {
                Ok(input) if input.type_() == "checkbox" => input.checked().to_string(),
                Ok(input) => input.value(),
                Err(_) => editor
                    .dyn_into::<HtmlSelectElement>()
                    .map(|select| select.value())
                    .unwrap_or_default(),
            },
        );

    let (data_type, field_name, nullable) = {
        let borrowed = runtime.borrow();
        let field = borrowed.state.schema().fields().get(cell.col).cloned();
        match field {
            Some(field) => (
                field.data_type,
                field.name.as_str().to_owned(),
                field.nullable,
            ),
            None => return,
        }
    };

    let mut rejected = None;
    // **An empty editor means NULL** — on a column that may hold one. It is the
    // only way to clear a value, and a typed input sanitizes most other
    // nonsense away before it ever gets here. On a required column it is a
    // refusal, not an empty string.
    if commit
        && typed.as_deref().is_some_and(str::is_empty)
        && !matches!(data_type, DataType::Utf8 | DataType::Bool)
    {
        if nullable {
            let previous = runtime
                .borrow()
                .state
                .cell(cell)
                .map(crate::formats::plain_text)
                .unwrap_or_default();
            if !runtime
                .borrow_mut()
                .state
                .set_cell(cell, Value::Null)
                .is_empty()
            {
                dispatch_cell_change(host, cell, &field_name, "", &previous);
            }
        } else {
            rejected = Some(String::new());
        }
    } else if commit && let Some(text) = typed.as_deref() {
        // One notation: the same parser the filter row and the wire format use,
        // so a value typed into a cell means what it would have meant anywhere
        // else (E13, S8, S9).
        match grid::literal(text, data_type).and_then(|json| {
            let text = json.to_string();
            let mut deserializer = serde_json::Deserializer::from_str(&text);
            Value::deserialize_typed(&mut deserializer, &data_type).ok()
        }) {
            Some(value) => {
                let previous = runtime
                    .borrow()
                    .state
                    .cell(cell)
                    .map(crate::formats::plain_text)
                    .unwrap_or_default();
                if !runtime.borrow_mut().state.set_cell(cell, value).is_empty() {
                    dispatch_cell_change(host, cell, &field_name, text, &previous);
                }
            }
            None => rejected = Some(text.to_owned()),
        }
    }

    runtime.borrow_mut().state.end_edit();
    if let Some(value) = &rejected {
        let texts = texts(host);
        let message = if value.is_empty() {
            texts.cell_required(&field_name)
        } else {
            texts.filter_invalid(&field_name, value)
        };
        runtime.borrow_mut().state.set_notice(message);
    }

    // **The element owns the editor node, so the element removes it.** The
    // edited cell sits in the focused row, and the focused slot is the one the
    // renderer never touches (point 17) — waiting for a patch to clear it would
    // wait forever.
    if let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) {
        let text = {
            let borrowed = runtime.borrow();
            borrowed
                .state
                .cell(cell)
                .map(|value| {
                    Formatter::new(&formats(host), borrowed.state.schema()).text(cell.col, value)
                })
                .unwrap_or_default()
        };
        td.set_text_content(Some(&text));
        if runtime.borrow().state.is_changed(cell) {
            let _ = td.set_attribute("data-changed", "true");
        }
    }

    render(host, false);
    focus_active(host);
}

/// Fires `opengrid-cell-change` on the host — the contract of point 35.
fn dispatch_cell_change(
    host: &HtmlElement,
    cell: CellRef,
    column: &str,
    value: &str,
    previous: &str,
) {
    let detail = js_sys::Object::new();
    for (key, value) in [
        ("row", JsValue::from_f64(cell.row as f64)),
        ("column", JsValue::from_str(column)),
        ("value", JsValue::from_str(value)),
        ("previous", JsValue::from_str(previous)),
    ] {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(key), &value);
    }

    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(false);
    init.set_detail(&detail);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(CELL_EVENT, &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// The name of the cell-change event (plan point 37).
pub const CELL_EVENT: &str = "opengrid-cell-change";

/// Fires `opengrid-selection-change` on the host.
///
/// **The event contract (point 35), which every later event follows:**
///
/// * The name is prefixed with `opengrid-`, so it cannot collide with an event
///   the page already uses.
/// * `bubbles` **and** `composed`: the event is dispatched on the host, and
///   `composed` lets it cross a shadow boundary the host may itself sit in —
///   without it, a page that wraps the grid in its own component hears nothing.
/// * Not `cancelable`: it reports what has already happened. An event that can
///   be prevented needs a state machine that can be rolled back, and V1 has
///   none.
/// * `detail` is plain JSON-ish data — no Rust types, no live references:
///   `{ rows: [u64], count: number }`.
fn dispatch_selection(host: &HtmlElement, rows: &[u64]) {
    let list = js_sys::Array::new();
    for row in rows {
        list.push(&JsValue::from_f64(*row as f64));
    }
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("rows"), &list);
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("count"),
        &JsValue::from_f64(rows.len() as f64),
    );

    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(false);
    init.set_detail(&detail);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(SELECTION_EVENT, &init) {
        let _ = host.dispatch_event(&event);
    }
}

/// The name of the selection event (point 35).
pub const SELECTION_EVENT: &str = "opengrid-selection-change";

/// `Enter`/`Space` on a header cell sorts by its column and re-runs the query;
/// on a data cell it is a no-op.
///
/// A plain activation toggles the **single** sort (asc → desc → default). With
/// `Shift` the column is added/removed as an **additional** key, keeping the
/// existing keys' order (plan point 18 multi-sort). Either way the grid keeps at
/// least the default sort (rule S6) and returns to the first window.
fn activate_header(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, multi: bool) {
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
    let pool = pool_of(host);
    {
        let mut runtime = runtime.borrow_mut();
        if multi {
            runtime.state.toggle_sort_multi(&field);
        } else {
            runtime.state.toggle_sort(&field);
        }
        // Clearing the last sort falls back to the default first column so the
        // next page request still has a total order (rule S6).
        runtime.state.ensure_sorted();
        runtime.state.set_window(Window::new(0, pool));
    }
    // Sorting drops the selection (a selection names positions, and the order
    // just changed): the page hears it the same way it hears every other
    // selection change.
    announce_if_cleared(host, runtime);
    run_query(host, QueryKind::Data, true);
}

/// Whether `element` sits inside the filter row (and not in the grid table).
fn is_filter_control(element: &Element) -> bool {
    element.closest("[data-filter]").ok().flatten().is_some()
}

/// Reads the filter row into [`FilterEntry`]s, one per output column.
///
/// The operator `select` and value `input` are ordinary form controls; a missing
/// control (should not happen after the skeleton) falls back to `eq`/empty.
fn read_filter_entries(root: &ShadowRoot, columns: &[String]) -> Vec<FilterEntry> {
    columns
        .iter()
        .enumerate()
        .map(|(col, column)| {
            let op = root
                .query_selector(&format!("select[data-col=\"{col}\"]"))
                .ok()
                .flatten()
                .and_then(|node| node.dyn_into::<HtmlSelectElement>().ok())
                .map(|select| select.value())
                .unwrap_or_default();
            let value = root
                .query_selector(&format!("input[data-col=\"{col}\"]"))
                .ok()
                .flatten()
                .and_then(|node| node.dyn_into::<HtmlInputElement>().ok())
                .map(|input| input.value())
                .unwrap_or_default();
            FilterEntry {
                column: column.clone(),
                op: grid::FilterOp::parse(&op).unwrap_or(grid::FilterOp::Cmp(CmpOp::Eq)),
                value,
            }
        })
        .collect()
}

/// Applies the filter row: builds the `filter` expression, returns to the first
/// window and re-runs the query without moving focus (it stays in the control
/// the user typed into).
///
/// An input the column cannot hold does **not** become a query (point 51). It
/// lands in the status line instead, which is announced — the user typed it, so
/// the user hears about it, rather than the engine answering with a validation
/// error about a value nobody can see.
fn apply_filters(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let columns = columns_of(host);
    let entries = read_filter_entries(&root, &columns);
    let schema = runtime.borrow().state.schema().clone();

    let filter = match grid::filter_expr(&entries, &schema) {
        Ok(filter) => filter,
        Err(problems) => {
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
    };

    let pool = pool_of(host);
    {
        let mut runtime = runtime.borrow_mut();
        runtime.state.set_filter(filter);
        runtime.state.set_window(Window::new(0, pool));
    }
    // A filter changes which rows exist, so the selection is gone (point 35).
    announce_if_cleared(host, &runtime);
    run_query(host, QueryKind::Data, false);
}

/// An operator control changed: bring its value field in line.
fn on_filter_change(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    // A column-visibility checkbox (point 36): ordinary form behaviour, so the
    // keyboard needs nothing special.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && let Some(name) = target.get_attribute("data-column")
        && let Ok(input) = target.dyn_into::<HtmlInputElement>()
        && input.type_() == "checkbox"
    {
        set_column_hidden(&host, &name, !input.checked());
        return;
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let schema = runtime.borrow().state.schema().clone();
    fix_operator_choices(&root, &schema);
}

/// Moves a column's operator control onto an operator its type allows.
///
/// The options are built before the types are known (point 51), so the default
/// selection can be one the column does not offer — `contains` on a number.
/// Patches can disable that option, but the *selection* is a DOM property, so it
/// is corrected here, after the frame landed.
fn fix_operator_choices(root: &ShadowRoot, schema: &opengrid_types::Schema) {
    for (col, field) in schema.fields().iter().enumerate() {
        let Ok(Some(node)) = root.query_selector(&format!("select[data-col=\"{col}\"]")) else {
            continue;
        };
        let Ok(select) = node.dyn_into::<HtmlSelectElement>() else {
            continue;
        };
        let allowed = grid::operators_for(field.data_type, field.nullable);
        let current = select.value();
        if !allowed.contains(&current.as_str())
            && let Some(first) = allowed.first()
        {
            select.set_value(first);
        }

        // The two operators that take no value say so: their input is disabled
        // rather than silently ignored.
        if let Ok(Some(node)) = root.query_selector(&format!("input[data-col=\"{col}\"]"))
            && let Ok(input) = node.dyn_into::<HtmlInputElement>()
        {
            input.set_disabled(!grid::takes_value(&select.value()));
        }
    }
}

/// Clears every value input and applies the (now empty) filter.
fn on_filter_clear(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
    else {
        return;
    };
    // The column list opens and closes from its own button (point 36).
    if let Ok(Some(button)) = target.closest("button[part=\"columns-toggle\"]")
        && let Some(root) = current_shadow_root(&event)
        && let Ok(Some(list)) = root.query_selector("[part=\"columns\"]")
    {
        let open = button.get_attribute("aria-expanded").as_deref() == Some("true");
        let _ = button.set_attribute("aria-expanded", if open { "false" } else { "true" });
        if open {
            let _ = list.set_attribute("hidden", "");
        } else {
            let _ = list.remove_attribute("hidden");
        }
        return;
    }

    // The same listener serves the paging buttons (point 38); keyboard
    // activation fires a click too, so `Enter`/`Space` on them works natively.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && on_pager_click(&host, &target)
    {
        return;
    }
    if target
        .closest("[data-filter-clear]")
        .ok()
        .flatten()
        .is_none()
    {
        return;
    }
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    if let Ok(inputs) = root.query_selector_all("input[data-col]") {
        for index in 0..inputs.length() {
            if let Some(node) = inputs.item(index)
                && let Ok(input) = node.dyn_into::<HtmlInputElement>()
            {
                input.set_value("");
            }
        }
    }
    apply_filters(&host);
}

/// `Escape` returns focus to the first cell of the grid (the top-left header
/// cell), loading the first window if the grid had scrolled on.
fn escape_to_first(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
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
    let pool = pool_of(host);
    let first = ActiveCell::Header { col: 0 };
    runtime.borrow_mut().set_active(first);
    if offset != 0 {
        runtime.borrow_mut().state.set_window(Window::new(0, pool));
        run_query(host, QueryKind::Window, true);
    } else if let Some(root) = host.shadow_root() {
        let viewport = runtime.borrow().viewport.clone();
        focus_from(&root, viewport.as_ref(), from, first);
    }
}

/// Handles a scroll of the viewport: derives the new window from `scrollTop` and
/// schedules a coalesced query for the next animation frame.
///
/// A fast scroll can fire many events before a frame renders; the schedule flag
/// makes them collapse into a single query.
fn on_scroll(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    // While paging there is nothing to scroll into: the sizer is the page, and
    // the window belongs to the pager (point 38).
    if grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).is_some() {
        return;
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let scroll_top = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .map(|viewport| viewport.scroll_top().max(0) as u64)
        .unwrap_or(0);

    let (total_count, pool, offset, row_height) = {
        let runtime = runtime.borrow();
        let pool = runtime
            .view
            .as_ref()
            .map(|view| view.pool() as u64)
            .unwrap_or(0);
        (
            runtime.state.total_count(),
            pool,
            runtime.state.window().offset,
            runtime.row_height,
        )
    };
    if total_count == 0 || pool == 0 {
        return;
    }
    let wanted = grid::window_offset(
        grid::visible_start(scroll_top, row_height),
        total_count,
        pool,
    );
    if wanted == offset {
        return;
    }
    runtime.borrow_mut().pending_offset = Some(wanted);
    schedule_scroll_query(&host, &runtime);
}

/// Schedules the pending scroll window as one query on the next animation frame.
fn schedule_scroll_query(host: &HtmlElement, grid_runtime: &Rc<RefCell<GridRuntime>>) {
    if grid_runtime.borrow().raf_pending {
        return;
    }
    grid_runtime.borrow_mut().raf_pending = true;
    let host = host.clone();
    let callback = Closure::once_into_js(move || {
        let Some(runtime) = runtime(&host) else {
            return;
        };
        let offset = {
            let mut runtime = runtime.borrow_mut();
            runtime.raf_pending = false;
            runtime.pending_offset.take()
        };
        let Some(offset) = offset else {
            return;
        };
        let pool = {
            let runtime = runtime.borrow();
            runtime
                .view
                .as_ref()
                .map(|view| view.pool() as u64)
                .unwrap_or(0)
        };
        if pool == 0 {
            return;
        }
        {
            let mut runtime = runtime.borrow_mut();
            if runtime.state.window().offset == offset {
                return;
            }
            runtime.state.set_window(Window::new(offset, pool));
        }
        run_query(&host, QueryKind::Window, false);
    });
    if let Some(window) = web_sys::window() {
        let _ = window.request_animation_frame(callback.unchecked_ref::<js_sys::Function>());
    }
}

/// The viewport height in rows, for the `PageUp`/`PageDown` step; falls back to
/// a constant when the browser has not laid the grid out yet.
fn viewport_rows(runtime: &Rc<RefCell<GridRuntime>>) -> u64 {
    let borrowed = runtime.borrow();
    let height = borrowed
        .viewport
        .as_ref()
        .map(|viewport| viewport.client_height())
        .unwrap_or(0);
    if height > 0 {
        (height as u64 / borrowed.row_height).max(1)
    } else {
        grid::DEFAULT_VIEWPORT_ROWS
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
    let (from, viewport) = {
        let runtime = runtime.borrow();
        (runtime.active, runtime.viewport.clone())
    };
    if from == active {
        return;
    }
    runtime.borrow_mut().set_active(active);
    focus_from(&root, viewport.as_ref(), from, active);
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
