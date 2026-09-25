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
use opengrid_query::{CmpOp, FilterExpr, Sort, SortDirection};
use opengrid_types::{DataType, Value};
use opengrid_web_core::element::{
    ARIA_LABEL_ATTRIBUTE, LABEL_ATTRIBUTE, attach_open_shadow_root, define, mirror_label,
};
use opengrid_web_core::host;
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::columns::{self, WIDTH_STEP};
use crate::formats::{CellFormat, Formatter, formats};
use crate::grid_element_events::{CELL_EVENT, SELECTION_EVENT, VIEW_EVENT};
use opengrid_web_core::renderer::{Dom, WebRenderer};

use crate::element::{clear_root, describe};
use crate::grid::{
    self, ActiveCell, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, FilterEntry, GRID_TAG, GridKey,
    GridNodes, GridSkeleton, MODE_ATTRIBUTE, PAGE_SIZE_ATTRIBUTE, ROW_HEIGHT_PROPERTY,
    WINDOW_SIZE_ATTRIBUTE,
};
use crate::grouping::{self, Grouping};
use crate::presentation;
use crate::texts::texts;
use crate::view::GridView;

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
    /// The resolved pixel height of one logical row (`--og-row-height`).
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
    /// The grouping, while `group-by` is set and valid (point 62).
    grouping: Option<Grouping>,
    /// The filter the loaded groups were counted under — a different one means
    /// the counts are stale and the group query has to run again.
    groups_filter: Option<Option<FilterExpr>>,
    /// The reader's choice of aggregate per column, from the view (point 63).
    /// Leads over what `set_columns` configured.
    aggregate_choice: std::collections::BTreeMap<String, presentation::Summary>,
    /// Whether the filter row shows (point 65). Part of the view; on unless
    /// the reader or a view turned it off.
    filter_row: bool,
    /// The reader's facet selections, by column (point 66).
    facets: Vec<(String, crate::facets::Selection)>,
    /// Each list facet's values without any filter — what the facet lists.
    facet_domain: std::collections::BTreeMap<String, Vec<grouping::Group>>,
    /// The counts under every filter but the facet's own, by key.
    facet_counts: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>,
    /// Queries the last count round took — what a facet costs (F4).
    facet_queries: usize,
    /// Bumped per count round; an older round's answers are dropped.
    facet_generation: u64,
    /// The free text the search field applied (point 67), or empty.
    search_text: String,
    /// The viewport's scroll offset, as the reader left it. A browser forgets
    /// the offset of an element that leaves the document, even for a move;
    /// this is what puts it back (point 74).
    scroll_top: i32,
}

impl GridRuntime {
    /// Moves the active cell and keeps [`GridState`]'s focus in step.
    fn set_active(&mut self, active: ActiveCell) {
        self.active = active;
        self.state.set_focus(active.data());
    }
}

thread_local! {
    static RUNTIMES: RefCell<HashMap<u32, Rc<RefCell<GridRuntime>>>> =
        RefCell::new(HashMap::new());
}

/// Forgets the runtime of a collected host (point 74).
fn release(id: u32) {
    RUNTIMES.with(|runtimes| runtimes.borrow_mut().remove(&id));
}

/// The runtime attached to `host`, if any.
fn runtime(host: &HtmlElement) -> Option<Rc<RefCell<GridRuntime>>> {
    let id = host::existing_id(host)?;
    RUNTIMES.with(|runtimes| runtimes.borrow().get(&id).cloned())
}

/// Stores `runtime` under the host's id.
fn attach_runtime(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    host::on_release(release);
    let id = host::id(host);
    RUNTIMES.with(|runtimes| runtimes.borrow_mut().insert(id, Rc::clone(runtime)));
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
        grouping: grouping_of(host).ok().flatten(),
        groups_filter: None,
        aggregate_choice: Default::default(),
        filter_row: true,
        facets: Vec::new(),
        facet_domain: Default::default(),
        facet_counts: Default::default(),
        facet_queries: 0,
        facet_generation: 0,
        search_text: String::new(),
        scroll_top: 0,
    }
}

/// Resolves the `--og-row-height` custom property on the host.
///
/// Whether this grid shows the selection column (point 61).
///
/// A boolean attribute: present means yes, whatever its value — the HTML rule
/// for boolean attributes, so `selection` and `selection=""` mean the same.
fn shows_selection(host: &HtmlElement) -> bool {
    host.has_attribute(grid::SELECTION_ATTRIBUTE)
}

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
        runtime.scroll_top = 0;
        // A rebuild (columns, texts, presentation) is not a reason to close
        // every group the reader opened: the same grouping keeps its expanded
        // set, and only its counts are asked for again.
        let keep = matches!(
            (&runtime.grouping, &fresh.grouping),
            (Some(old), Some(new)) if old.by() == new.by()
        );
        if keep {
            if let Some(grouping) = runtime.grouping.as_mut() {
                grouping.invalidate();
            }
        } else {
            runtime.grouping = fresh.grouping;
        }
        runtime.groups_filter = None;
        // `filter_row` and `aggregate_choice` are the reader's, not the
        // skeleton's: a rebuild keeps them, like it keeps the column layout.
    }
}

/// Renders the skeleton, installs the listeners and runs the first query — or,
/// for a grid that comes back, puts it back as the reader left it.
///
/// Coming back is a move within the page (disconnect and connect in one task)
/// or a return from a cache like Vue's `<KeepAlive>`, after the grid let go of
/// its DOM (see [`on_disconnected`]). Neither is a new result: the state still
/// holds the rows, so the grid redraws and asks nothing — unless the new place
/// gives it a different row height, which changes the window it needs.
fn on_connected(host: HtmlElement) {
    let Ok(root) = attach_open_shadow_root(&host) else {
        return;
    };
    let returning = runtime(&host).is_some();
    let runtime = runtime_or_init(&host);
    let height = resolve_row_height(&host);

    if !returning {
        add_listeners(&root);
        runtime.borrow_mut().row_height = height;
        // A `group-by` in the markup is read before anything else happens; one
        // the grid refuses is said with the first result, and it runs ungrouped.
        if let Err(message) = grouping_of(&host) {
            runtime.borrow_mut().state.set_notice(message);
        }
        ensure_skeleton(&host);
        render(&host, false);
        run_query(&host, QueryKind::Data, false);
        return;
    }

    let resized = {
        let mut borrowed = runtime.borrow_mut();
        let resized = borrowed.row_height != height;
        borrowed.row_height = height;
        resized
    };
    let refetch = runtime.borrow().view.is_none() && follow_active_row(&host, &runtime);
    if resized {
        // The same pixel offset is a different row now; the window follows
        // it, so the scroll event after the restore finds it in place.
        window_from_scroll(&runtime);
    }
    // Drawn first: a fresh skeleton gets its scroll height from the frame, and
    // an offset set before that is clamped to nothing.
    render(&host, false);
    restore_scroll(&runtime);
    if resized {
        run_query(&host, QueryKind::Data, false);
    } else if refetch {
        run_query(&host, QueryKind::Window, false);
    }
}

/// Puts the viewport back at the offset the runtime remembers.
fn restore_scroll(runtime: &Rc<RefCell<GridRuntime>>) {
    let (viewport, top) = {
        let borrowed = runtime.borrow();
        (borrowed.viewport.clone(), borrowed.scroll_top)
    };
    if let Some(viewport) = viewport
        && viewport.scroll_top() != top
    {
        viewport.set_scroll_top(top);
    }
}

/// Sets the window to the one the remembered offset shows — what
/// [`on_scroll`] would ask for, so its event after a restore asks nothing.
fn window_from_scroll(runtime: &Rc<RefCell<GridRuntime>>) {
    let mut borrowed = runtime.borrow_mut();
    let window = borrowed.state.window();
    let offset = grid::window_offset(
        grid::visible_start(borrowed.scroll_top.max(0) as u64, borrowed.row_height),
        borrowed.state.total_count(),
        window.count,
    );
    borrowed.state.set_window(Window::new(offset, window.count));
}

/// Scrolls to the active row when a fresh pool cannot show it, and answers
/// whether it did.
///
/// A fresh pool has no pinned slot (see [`grid::assign_pool`]): an active row
/// the pin kept on screen outside the loaded window exists only in the DOM the
/// grid just threw away. Left like that, no cell would carry `tabindex="0"`,
/// and `Tab` would skip the grid. The grid goes where its active cell is —
/// where `Tab` would take the reader anyway — and loads that window: one
/// [`QueryKind::Window`] query, because what the result *is* has not changed.
fn follow_active_row(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) -> bool {
    // While paging, the window is the page, and the active row is on it.
    if grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).is_some() {
        return false;
    }
    {
        let mut borrowed = runtime.borrow_mut();
        let Some(row) = borrowed.active.data().map(|cell| cell.row) else {
            return false;
        };
        let window = borrowed.state.window();
        if (window.offset..window.offset.saturating_add(window.count)).contains(&row) {
            return false;
        }
        let top = row.saturating_mul(borrowed.row_height);
        borrowed.scroll_top = i32::try_from(top).unwrap_or(i32::MAX);
    }
    window_from_scroll(runtime);
    true
}

/// Lets go of the DOM once the grid has really left the document (point 74).
///
/// Not at once: a move disconnects and reconnects in the same task, and must
/// not cost a rebuild. After that, the grid drops every reference it holds into
/// its shadow tree. A reference from WASM is a root the garbage collector
/// cannot see through; as long as one exists, a grid removed for good would be
/// kept alive for ever, and with it its rows and whatever the page handed it.
/// The shadow tree itself stays — it is the host's, and it still holds what
/// the reader typed into the filter row, which [`ensure_skeleton`] carries
/// into a fresh skeleton if the grid comes back.
fn on_disconnected(host: HtmlElement) {
    park_later(&host);
}

/// Parks `host` once the current script and its microtasks are through, if it
/// is still out of the document by then. A framework that removes and inserts
/// further apart pays a rebuild — correct, only dearer.
fn park_later(host: &HtmlElement) {
    let host = host.clone();
    spawn_local(async move {
        let _ = JsFuture::from(js_sys::Promise::resolve(&JsValue::UNDEFINED)).await;
        if host.is_connected() {
            return;
        }
        if let Some(runtime) = runtime(&host) {
            let mut runtime = runtime.borrow_mut();
            runtime.view = None;
            runtime.dom = None;
            runtime.viewport = None;
        }
    });
}

/// Rebuilds the grid after its texts changed (point 48).
///
/// The labels of the filter row and the operator names sit in the one-time
/// skeleton, so new texts mean a new skeleton — and a rebuild that only kept the
/// *model* would leave the user somewhere else entirely: the new filter row
/// would be empty while the query still filters, the new viewport would sit at
/// the top while the window is at row 500, and the focused cell would be gone
/// from the DOM. So everything the user can see is carried across: the grid
/// lets go of its skeleton the way a parked grid does, [`ensure_skeleton`]
/// carries the filter row and the search into the new one, the scroll offset is
/// restored once the frame gave the viewport its height, and the active cell
/// takes the focus back.
pub(crate) fn retext(host: &HtmlElement) {
    if host.shadow_root().is_none() {
        return;
    }
    let Some(runtime) = runtime(host) else {
        return;
    };
    {
        let mut runtime = runtime.borrow_mut();
        runtime.view = None;
        runtime.dom = None;
        runtime.viewport = None;
    }
    // The query below asks for whatever window this leaves.
    follow_active_row(host, &runtime);
    render(host, false);
    restore_scroll(&runtime);
    focus_active(host);
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
        grid::DENSITY_ATTRIBUTE => {
            // A density changes the row height, and the row height is the
            // virtualization contract: the sizer's height, every row's
            // `translateY` and the window the provider is asked for are all
            // derived from it. Resolving it once at connect is therefore not
            // enough — without this the sizer keeps the old total height and
            // the rows sit at offsets that no longer match their slots.
            if applying_view() {
                // `write_view` sets the density on its way to setting
                // everything else and owns the single query that follows.
                return;
            }
            let Some(runtime) = runtime(&host) else {
                return;
            };
            let height = resolve_row_height(&host);
            {
                let mut runtime = runtime.borrow_mut();
                if runtime.row_height == height {
                    return;
                }
                runtime.row_height = height;
            }
            // The viewport now holds a different number of rows, so the window
            // is re-asked for rather than re-drawn at the old size.
            sync_page_window(&host, &runtime);
            render(&host, false);
            run_query(&host, QueryKind::Data, false);
            dispatch_view(&host);
        }
        // `write_view` sets it on its way and owns the single query after.
        grid::GROUP_BY_ATTRIBUTE if applying_view() => {}
        grid::GROUP_BY_ATTRIBUTE => regroup(&host),
        // The selection column is part of the one-time skeleton, so switching
        // it means a new skeleton — the same as changing the columns.
        grid::SELECTION_ATTRIBUTE
        | grid::COLUMN_MENU_ATTRIBUTE
        | grid::TOOLBAR_ATTRIBUTE
        | grid::FACETS_ATTRIBUTE
        | grid::SEARCH_ATTRIBUTE
        | DATASOURCE_ATTRIBUTE
        | COLUMNS_ATTRIBUTE
        | WINDOW_SIZE_ATTRIBUTE
        | MODE_ATTRIBUTE => {
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
    // Children without a view: a skeleton the grid let go of — parked while
    // it was out of the document, or dropped for new texts. What the reader
    // typed lives in it (the filter row is where a view reads its filters
    // from), so it is carried into the new one.
    let carried = (root.child_element_count() > 0).then(|| {
        let entries = read_filter_entries(&root, &columns_of(host));
        let search = search_input(&root).map(|input| input.value());
        clear_root(&root);
        (entries, search)
    });
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
        &GridSkeleton {
            label: label.as_deref(),
            schema: &schema,
            pool,
            texts: &texts(host),
            declared: &declared,
            presentation: &presentation::styles(host),
            selection: shows_selection(host),
            column_menu: host.has_attribute(grid::COLUMN_MENU_ATTRIBUTE),
            toolbar: host.has_attribute(grid::TOOLBAR_ATTRIBUTE),
            facets: host.has_attribute(grid::FACETS_ATTRIBUTE),
            search: host.has_attribute(grid::SEARCH_ATTRIBUTE),
        },
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
    if let Some((entries, search)) = carried {
        write_filter_entries(&root, &entries);
        if let (Some(input), Some(search)) = (search_input(&root), search) {
            input.set_value(&search);
        }
    }
    // Built while out of the document — a page called into a parked grid, or a
    // result arrived after it left: let go again, or it is never collected.
    if !host.is_connected() {
        park_later(host);
    }
}

/// The search field, when the grid has one (point 67).
fn search_input(root: &ShadowRoot) -> Option<HtmlInputElement> {
    root.query_selector("[part~=\"search-input\"]")
        .ok()
        .flatten()
        .and_then(|element| element.dyn_into::<HtmlInputElement>().ok())
}

/// Computes the current window assignment and applies it as one patch list.
///
/// The focused slot is pinned (see [`grid::assign_pool`]); the function never
/// rebuilds the table. `focus_after` re-focuses the active cell once the frame
/// landed, so a keyboard-driven reload does not drop the focus.
fn render(host: &HtmlElement, focus_after: bool) {
    // Nobody sees a grid out of the document, and drawing would only build a
    // skeleton to let go of again; it draws when it comes back (point 74).
    if !host.is_connected() {
        return;
    }
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
            borrowed.grouping.as_ref(),
        );
        if let Some(dom) = borrowed.dom.as_mut() {
            dom.apply_buffer(&buffer);
        }
    }

    if let Some(root) = host.shadow_root() {
        let schema = runtime.borrow().state.schema().clone();
        fix_operator_choices(&root, &schema);
        fix_presentation(&root, &schema, &presentation::styles(host));
    }
    sync_chrome(host);

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
    if grid_runtime.borrow().grouping.is_some() {
        run_grouped(host, &grid_runtime, kind, focus);
        return;
    }
    let pool = pool_of(host);
    sync_page_window(host, &grid_runtime);
    let pool =
        grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).unwrap_or(pool);

    // Into a local first: in the `match` scrutinee the `borrow()` would live
    // for the whole expression, and the `borrow_mut()` in the error arm would
    // panic — a panic in an event handler is a silently dead key. (Found by
    // the facet test of point 66: an invalid bound said nothing at all.)
    let effective = effective_filter(host, &grid_runtime.borrow(), None);
    let filter = match effective {
        Ok(filter) => filter,
        Err(message) => {
            grid_runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(message));
            render(host, false);
            return;
        }
    };
    let (sorts, offset, generation) = {
        let mut runtime = grid_runtime.borrow_mut();
        let generation = runtime.generation + 1;
        runtime.generation = generation;
        (
            runtime.state.sort_keys(),
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
    if kind == QueryKind::Data {
        refresh_facets(host);
    }

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
    // The first result brings the real schema, and a `set_columns` made before
    // the provider was attached has only met the display schema so far — where
    // every column is `Utf8` and `sum` on a number would read as a sum on text
    // (point 60). Re-checking here is cheap and terminates: it only rebuilds
    // when the checked presentation actually changed.
    recolumn(host);
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
        // The selection column is addressed by `data-select`, not by a
        // `data-col` of its own: `data-col` means *schema column* everywhere
        // else, and the selection column is not one (point 61).
        // The cell is the target, the mark inside it is the widget that takes
        // the focus — a `columnheader` may not be a checkbox (point 61).
        ActiveCell::SelectAll => "[data-select=\"all\"] > [part=\"select-mark\"]".to_owned(),
        ActiveCell::Select { row } => {
            format!("tr:has(td[data-row=\"{row}\"]) > td[data-select=\"row\"]")
        }
        ActiveCell::Header { col } => format!("th[data-col=\"{col}\"]"),
        ActiveCell::Data(cell) => {
            format!("td[data-row=\"{}\"][data-col=\"{}\"]", cell.row, cell.col)
        }
    }
}

/// Reads the active cell back from a DOM cell (click or `focusin`).
fn active_from_element(element: &Element) -> Option<ActiveCell> {
    // `closest`, not the attribute: the focus lands on the checkbox *inside*
    // the header cell, and a click can land on either.
    if let Some(cell) = element.closest("[data-select]").ok().flatten()
        && let Some(kind) = cell.get_attribute("data-select")
    {
        let element = &cell;
        return match kind.as_str() {
            "all" => Some(ActiveCell::SelectAll),
            "row" => element
                .closest("tr")
                .ok()
                .flatten()
                .and_then(|row| row.query_selector("td[data-row]").ok().flatten())
                .and_then(|cell| cell.get_attribute("data-row"))
                .and_then(|row| row.parse().ok())
                .map(|row| ActiveCell::Select { row }),
            _ => None,
        };
    }
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
    // The search field follows the typing: its hint and its suggestions (67).
    let input = Closure::<dyn FnMut(Event)>::new(on_search_input).into_js_value();
    let _ = root.add_event_listener_with_callback("input", input.unchecked_ref());
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
    // `prevent_default()` on `Enter` and swallow the click (point 38). The
    // toolbar and the chips of point 65 are the same kind of control, and hit
    // the same trap the first time they were tried.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target
            .closest(
                "[part=\"pager\"], [part=\"toolbar\"], [part=\"chips\"], [part=\"facets\"], \
                 [part=\"empty\"]",
            )
            .ok()
            .flatten()
            .is_some()
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

    // In the search field the keys are the combobox's (point 67).
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target.closest("[part=\"search\"]").ok().flatten().is_some()
    {
        on_search_key(&host, &event, &target);
        return;
    }

    // Inside an open column menu the keys are the menu's (point 64).
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target
            .closest("[part=\"column-menu\"]")
            .ok()
            .flatten()
            .is_some()
    {
        on_menu_key(&host, &event, &target);
        return;
    }

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

    // The column menu opens from its header cell (point 64): `Alt`+`↓`, the
    // way a menu button or a combobox opens, and the two context-menu keys.
    let opens_menu = (event.alt_key() && event.key() == "ArrowDown")
        || (event.shift_key() && event.key() == "F10")
        || event.key() == "ContextMenu";
    if opens_menu
        && host.has_attribute(grid::COLUMN_MENU_ATTRIBUTE)
        && let ActiveCell::Header { col } = runtime.borrow().active
    {
        event.prevent_default();
        open_column_menu(&host, col);
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

    // A group header (point 62): `Enter` and `Space` open and close it, and on
    // its first cell `→` opens and `←` closes — the treegrid keys (F2). `→` on
    // an open group and `←` on a closed one fall through to moving, so nothing
    // is a dead end.
    if let Some((position, expanded)) = active_group(&host) {
        let on_first = matches!(active, ActiveCell::Data(cell) if cell.col == 0);
        let toggles = match event.key().as_str() {
            "Enter" | " " => true,
            "ArrowRight" => on_first && !expanded,
            "ArrowLeft" => on_first && expanded,
            _ => false,
        };
        if toggles && !event.ctrl_key() && !event.meta_key() {
            event.prevent_default();
            toggle_group(&host, position);
            return;
        }
    }
    // Grouped, a position is a display position: a selection would name
    // headers as well as rows and move on every toggle, and an edit would be
    // reported against a row number the page cannot map to anything. Both are
    // off while `group-by` is set — recorded as an open question of point 62.
    if is_grouped(&host)
        && (matches!(event.key().as_str(), "Enter" | " ")
            || (event.key().eq_ignore_ascii_case("a") && (event.ctrl_key() || event.meta_key())))
    {
        event.prevent_default();
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
                // The selection column's two cells do the same on `Enter` as
                // on `Space`: there is nothing else they could mean, and a key
                // that does nothing on a focusable cell is a dead end.
                ActiveCell::SelectAll => toggle_all(&host, &runtime),
                ActiveCell::Select { row } => toggle_row(&host, &runtime, row, false),
                ActiveCell::Header { .. } => activate_header(&host, &runtime, event.shift_key()),
                ActiveCell::Data(cell) => begin_edit(&host, cell),
            }
        }
        (" ", _) => {
            event.prevent_default();
            // On a header `Space` sorts, as it always has; on a data cell it
            // now selects the row (point 35). `Shift` extends from the anchor.
            match active {
                ActiveCell::SelectAll => toggle_all(&host, &runtime),
                ActiveCell::Select { row } => toggle_row(&host, &runtime, row, event.shift_key()),
                ActiveCell::Header { .. } => {
                    activate_header(&host, &runtime, event.shift_key());
                }
                ActiveCell::Data(cell) => {
                    toggle_row(&host, &runtime, cell.row, event.shift_key());
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

/// Selects or deselects one row; `extend` continues a range from the anchor.
fn toggle_row(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, row: u64, extend: bool) {
    let patches = {
        let mut borrowed = runtime.borrow_mut();
        if extend {
            borrowed.state.extend_selection(row)
        } else {
            borrowed.state.toggle_selection(row)
        }
    };
    settle_selection(host, runtime, patches);
}

/// Selects **every matching row**, or clears the selection if all are already
/// selected.
///
/// The same promise `Ctrl`+`A` has made since point 35, now with a control
/// attached: a header that said "all" about the loaded window would be a
/// different and smaller promise, and the grid holds one window at a time.
fn toggle_all(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let (patches, count) = {
        let mut borrowed = runtime.borrow_mut();
        let total = borrowed.state.total_count();
        let all = borrowed.state.selection().len() as u64 >= total && total > 0;
        let patches = if all {
            borrowed.state.clear_selection()
        } else {
            borrowed.state.select_all()
        };
        (patches, if all { 0 } else { total })
    };
    settle_selection(host, runtime, patches);
    // Said in words, because the change is off screen: selecting 100 000 rows
    // looks, on screen, exactly like selecting the eight that are visible.
    announce(host, &texts(host).selected_all(count));
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
    let next = grid::move_active(
        active,
        key,
        ncols,
        total_count,
        viewport_rows,
        shows_selection(host),
    );
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
/// The width is written onto the header cell and the data cells (see
/// [`apply_widths`]). The value is clamped so a column can never be resized into
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
    dispatch_view(host);
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
    dispatch_view(host);
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
    dispatch_view(host);
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

/// Writes the reader's widths onto the header cells **and** the data cells.
///
/// Every body row is its own `display: table` (it is absolutely positioned, so
/// it can be moved without reflowing the others), and a row-table does not
/// read the header's widths. As long as no column had a width, both split the
/// same total evenly and lined up by accident; one configured width among
/// automatic ones pulled header and values apart. So the widths also go into a
/// small stylesheet for `td[data-col]` — one write per change rather than one
/// per rendered cell, and pooled cells pick it up as they are recycled.
fn apply_widths(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let layout = columns::layout(host);
    let layout = layout.borrow();
    let mut rules = String::new();
    for (col, name) in columns_of(host).iter().enumerate() {
        // The reader's resize leads; the configuration is only where a column
        // starts (point 60, the same attribute/value relationship as the view).
        let width = layout
            .width(name)
            .or_else(|| presentation::styles(host).width(name));
        if let Some(width) = width {
            rules.push_str(&format!("td[data-col=\"{col}\"] {{ width: {width}px; }}\n"));
        }
        let Ok(Some(cell)) = root.query_selector(&format!("th[data-col=\"{col}\"]")) else {
            continue;
        };
        match width {
            Some(width) => {
                let _ = cell.set_attribute("style", &format!("width: {width}px;"));
            }
            None => {
                let _ = cell.remove_attribute("style");
            }
        }
    }
    let sheet = match root.query_selector("style[data-widths]") {
        Ok(Some(sheet)) => Some(sheet),
        _ => web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.create_element("style").ok())
            .inspect(|sheet| {
                let _ = sheet.set_attribute("data-widths", "");
                let _ = root.append_child(sheet);
            }),
    };
    if let Some(sheet) = sheet
        && sheet.text_content().unwrap_or_default() != rules
    {
        sheet.set_text_content(Some(&rules));
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
    //
    // It has to say *which* page: the row count does not change when you turn
    // one, so without this the line reads "60 matches" before and after and the
    // only thing that moved is invisible to anyone not looking at the pager.
    // `page_of` is the pager's own wording, so the label and the announcement
    // cannot drift apart. `set_notice` carries it across the query that follows
    // (the result line would otherwise wipe it out before it was read).
    let message = texts(host).page_of(next + 1, pages);
    runtime.borrow_mut().state.set_notice(message);
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

/// Where `set_choices` stores what the page supplied.
pub(crate) const CHOICES_KEY: &str = "__opengridChoices";

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
    dispatch_view(host);
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
    dispatch_view(host);
}

/// An operator control changed: bring its value field in line.
fn on_filter_change(event: Event) {
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && target.closest("[part=\"facets\"]").ok().flatten().is_some()
        && let Some(root) = current_shadow_root(&event)
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
    {
        on_facet_change(&host, &target);
        return;
    }
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
/// Writes the presentation markers of point 60 onto the header and the pool.
///
/// The same reason `fix_operator_choices` exists next to it: the skeleton is
/// built at connect, from the **display schema** of point 23 where every column
/// is `Utf8` — and the real types only arrive with the first result. An
/// alignment derived at build time would say "text, so left" about every
/// column, including the numbers, and would stay wrong for the life of the
/// grid.
///
/// Written per result rather than per frame: it is a handful of attributes per
/// column, and the browser ignores a `setAttribute` that changes nothing.
fn fix_presentation(
    root: &ShadowRoot,
    schema: &opengrid_types::Schema,
    styles: &presentation::ColumnStyles,
) {
    for (col, field) in schema.fields().iter().enumerate() {
        let markers = styles.markers(field.name.as_str(), field.data_type);
        let Ok(cells) = root.query_selector_all(&format!("[data-col=\"{col}\"]")) else {
            continue;
        };
        for index in 0..cells.length() {
            let Some(cell) = cells
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            // Only the table's own cells: the filter row shares `data-col`.
            if !matches!(cell.tag_name().as_str(), "TD" | "TH") {
                continue;
            }
            for name in ["data-mono", "data-emphasis", "data-muted"] {
                if !markers.iter().any(|(marker, _)| *marker == name) {
                    let _ = cell.remove_attribute(name);
                }
            }
            for (name, value) in &markers {
                let _ = cell.set_attribute(name, value);
            }
        }
    }
}

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

    // The empty state's reset (point 68): the same as "Remove all" (65), not
    // a second way with its own behaviour.
    if target
        .closest("[data-empty-reset]")
        .ok()
        .flatten()
        .is_some()
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
    {
        clear_chips(&host);
        // The button is gone with the rows back; the focus goes to the top of
        // the grid rather than to the document.
        if let Some(runtime) = runtime(&host)
            && root
                .query_selector("[data-toolbar=\"filter-row\"]")
                .ok()
                .flatten()
                .is_none()
        {
            runtime
                .borrow_mut()
                .set_active(ActiveCell::Header { col: 0 });
            focus_active(&host);
        }
        return;
    }

    // A suggestion of the search field (point 67).
    if let Ok(Some(option)) = target.closest("[part=\"search-list\"] [role=\"option\"]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
    {
        take_suggestion(&host, &option);
        return;
    }

    // The facet sidebar (point 66): its switch, its pills, its reset.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if target
            .closest("[data-toolbar=\"facets\"]")
            .ok()
            .flatten()
            .is_some()
        {
            let _ = if host.has_attribute(grid::FACETS_ATTRIBUTE) {
                host.remove_attribute(grid::FACETS_ATTRIBUTE)
            } else {
                host.set_attribute(grid::FACETS_ATTRIBUTE, "")
            };
            return;
        }
        if let Ok(Some(pill)) = target.closest("[part=\"facet-pill\"]") {
            toggle_facet_value(&host, &pill);
            return;
        }
        if target
            .closest("[data-facets-reset]")
            .ok()
            .flatten()
            .is_some()
        {
            reset_facets(&host);
            return;
        }
    }

    // The toolbar and the chips (point 65).
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if target
            .closest("[data-toolbar=\"filter-row\"]")
            .ok()
            .flatten()
            .is_some()
        {
            toggle_filter_row(&host);
            return;
        }
        if let Ok(Some(button)) = target.closest("[data-density]")
            && let Some(density) = button.get_attribute("data-density")
        {
            let _ = host.set_attribute(grid::DENSITY_ATTRIBUTE, &density);
            return;
        }
        if let Ok(Some(button)) = target.closest("[data-chip-remove]") {
            remove_chip(&host, &button);
            return;
        }
        if target
            .closest("[data-chips-clear]")
            .ok()
            .flatten()
            .is_some()
        {
            clear_chips(&host);
            return;
        }
    }

    // The column menu (point 64): its trigger opens it, its entries act.
    if let Ok(host) = root.host().dyn_into::<HtmlElement>() {
        if let Ok(Some(item)) = target.closest("[part=\"column-menu\"] [data-action]") {
            activate_menu_item(&host, &item);
            return;
        }
        if let Ok(Some(trigger)) = target.closest("[part=\"column-menu-button\"]")
            && let Some(col) = trigger
                .closest("th")
                .ok()
                .flatten()
                .and_then(|th| th.get_attribute("data-col"))
                .and_then(|col| col.parse::<usize>().ok())
        {
            open_column_menu(&host, col);
            return;
        }
    }

    // A group row (point 62): a click anywhere on it opens or closes it — the
    // pointer path to what `Enter` does.
    if let Ok(Some(row)) = target.closest("tr[data-kind=\"group\"]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && let Some(position) = row
            .query_selector("td[data-row]")
            .ok()
            .flatten()
            .and_then(|cell| cell.get_attribute("data-row"))
            .and_then(|row| row.parse::<u64>().ok())
    {
        toggle_group(&host, position);
        return;
    }

    // The selection column (point 61). The mouse path only: the keys already
    // reach it through the grid matrix, and a `click` here would otherwise fire
    // a second time for the keyboard activation.
    if let Ok(Some(cell)) = target.closest("[data-select]")
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && let Some(runtime) = runtime(&host)
        && !is_grouped(&host)
    {
        match active_from_element(&cell) {
            Some(ActiveCell::SelectAll) => {
                toggle_all(&host, &runtime);
                return;
            }
            Some(ActiveCell::Select { row }) => {
                toggle_row(&host, &runtime, row, false);
                return;
            }
            _ => {}
        }
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
    // The capture listener hears every scroller in the root — the filter row
    // and the facets scroll too — and only the viewport moves the window.
    let Some(viewport) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .filter(|target| {
            target
                .get_attribute("part")
                .is_some_and(|part| part.split_whitespace().any(|name| name == "viewport"))
        })
    else {
        return;
    };
    let scroll_top = viewport.scroll_top().max(0);
    // Remembered for the return: a browser forgets it (point 74).
    runtime.borrow_mut().scroll_top = scroll_top;
    let scroll_top = scroll_top as u64;

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

// ---------------------------------------------------------------------------
// The view as a value (point 59)
// ---------------------------------------------------------------------------

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
fn applying_view() -> bool {
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
        match borrowed.grouping.as_ref() {
            Some(grouping) => (grouping.by().to_vec(), grouping.expanded()),
            None => (Vec::new(), Vec::new()),
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
    })
}

/// [`crate::element::get_view`] — the view as a JS object.
pub(crate) fn read_view(host: &HtmlElement) -> JsValue {
    let Some(view) = current_view(host) else {
        return JsValue::NULL;
    };
    to_js(&view.to_json())
}

/// A `serde_json::Value` as a real JS value, through `JSON.parse`.
///
/// Not `serde-wasm-bindgen`: the crate has `serde_json` already and this is the
/// only place that needs the bridge — a dependency for one conversion is not
/// one this project takes.
fn to_js(value: &serde_json::Value) -> JsValue {
    js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL)
}

/// [`crate::element::set_view`] — applies a whole view in one query.
pub(crate) fn write_view(host: &HtmlElement, value: &JsValue) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Ok(text) = js_sys::JSON::stringify(value).map(String::from) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };

    let declared = grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());
    let view = match GridView::from_json(&json, &declared) {
        Ok(view) => view,
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

    if current_view(host).as_ref() == Some(&view) {
        // Setting the view it already has is not a change, and must not cost a
        // query or an announcement.
        return;
    }

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
    // The selection goes with the old state. It is dropped on purpose — a view
    // carries none — but not silently (point 73): the fresh state is told, so
    // the result that follows says "Selection cleared" like a sort would.
    let had_selection = !runtime.borrow().state.selection().is_empty();
    if let Some(root) = host.shadow_root() {
        clear_root(&root);
    }
    reset_runtime(host);
    if had_selection {
        runtime.borrow_mut().state.note_selection_dropped();
    }
    ensure_skeleton(host);
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.grouping = grouping_of(host).ok().flatten();
        borrowed.groups_filter = None;
        if let Some(grouping) = borrowed.grouping.as_mut() {
            grouping.set_expanded(&view.expanded);
        }
        borrowed.filter_row = view.filter_row;
        // The search is not part of a view — the prototype's views leave it out
        // too — and a restored view with an old search still applied would show
        // rows the view never named.
        borrowed.search_text.clear();
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
    if had_selection {
        // The page hears it the same way it hears every other selection change.
        dispatch_selection(host, &[]);
    }
    run_query(host, QueryKind::Data, had_focus);
    dispatch_view(host);
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

    let init = web_sys::CustomEventInit::new();
    init.set_bubbles(true);
    init.set_composed(true);
    init.set_cancelable(false);
    init.set_detail(&detail);
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(VIEW_EVENT, &init) {
        let _ = host.dispatch_event(&event);
    }
}

// ---------------------------------------------------------------------------
// Column presentation (point 60)
// ---------------------------------------------------------------------------

/// Checks the stored `set_columns` configuration against the current schema and
/// rebuilds if it holds.
///
/// Called twice for the same configuration, and deliberately: once when the page
/// sets it, and once when the first result brings the real schema. A page that
/// configures before attaching a provider — which is the ordinary order, exactly
/// as `set_texts` documents — would otherwise be checked against the display
/// schema of point 23, where every column is `Utf8` and `sum` on a number would
/// be refused for being a sum on text.
pub(crate) fn recolumn(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let raw = presentation::raw(host);
    if raw.is_empty() {
        return;
    }
    let schema = runtime.borrow().state.schema().clone();
    match presentation::validate(
        &raw,
        &schema,
        &grid::parse_columns(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref()),
    ) {
        Ok(checked) => {
            let styles = presentation::ColumnStyles::new(checked);
            if *presentation::styles(host) == styles {
                return;
            }
            presentation::store(host, styles);
            // The markers live in the one-time skeleton (a pool cell always
            // shows the same column), so a new presentation is a new skeleton.
            rebuild(host);
        }
        Err(problems) => {
            let message = problems
                .iter()
                .map(|problem| format!("{}: {}", problem.column, problem.reason))
                .collect::<Vec<_>>()
                .join(" ");
            runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(message));
            render(host, false);
        }
    }
}

// ---------------------------------------------------------------------------
// Grouping (point 62)
// ---------------------------------------------------------------------------

/// The grouping the attributes ask for, or why it cannot be had.
///
/// `Ok(None)`: no `group-by`. `Err`: a sentence for the status line — more than
/// two levels, a column the grid does not show, or paging, which grouping
/// cannot live beside (paging excludes virtualization since point 38, and the
/// group arithmetic presupposes it).
fn grouping_of(host: &HtmlElement) -> Result<Option<Grouping>, String> {
    let raw = host.get_attribute(grid::GROUP_BY_ATTRIBUTE);
    let texts = texts(host);
    let by = grouping::parse_group_by(raw.as_deref()).map_err(|all| texts.group_invalid(&all))?;
    if by.is_empty() {
        return Ok(None);
    }
    let shown = columns_of(host);
    if let Some(missing) = by.iter().find(|name| !shown.contains(name)) {
        return Err(texts.group_invalid(missing));
    }
    if host.has_attribute(PAGE_SIZE_ATTRIBUTE) {
        return Err(format!(
            "{} ({PAGE_SIZE_ATTRIBUTE})",
            texts.group_invalid(&by.join(","))
        ));
    }
    Ok(Some(Grouping::new(by)))
}

/// Whether the grid is grouped right now.
fn is_grouped(host: &HtmlElement) -> bool {
    runtime(host).is_some_and(|runtime| runtime.borrow().grouping.is_some())
}

/// Takes a new `group-by`: starts at the top, drops the selection, and says it
/// when the grouping is refused — once, riding with the next result.
fn regroup(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let pool = pool_of(host);
    let grouping = grouping_of(host);
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.grouping = grouping.clone().ok().flatten();
        borrowed.groups_filter = None;
        borrowed.state.set_window(Window::new(0, pool));
        borrowed.active = ActiveCell::Header { col: 0 };
        // Grouped, a position is a display position — a selection naming them
        // would name headers as well as rows, and would move on every toggle.
        borrowed.state.clear_selection();
        if let Err(message) = &grouping {
            borrowed.state.set_notice(message.clone());
        }
    }
    let viewport = runtime.borrow().viewport.clone();
    if let Some(viewport) = viewport {
        viewport.set_scroll_top(0);
    }
    run_query(host, QueryKind::Data, false);
    dispatch_view(host);
}

/// The aggregates the groups show, and what could not be had (point 63).
///
/// Per shown column: the reader's choice from the view, else what `set_columns`
/// configured, else **nothing** — a default "sum every number" would sum the
/// ids. A choice the column's type does not allow (a sum over text) is named,
/// not tried: the view is checked against names when it is set, but types are
/// only known once a result has come.
fn effective_aggregates(
    host: &HtmlElement,
    schema: &opengrid_types::Schema,
    choice: &std::collections::BTreeMap<String, presentation::Summary>,
) -> (Vec<(String, presentation::Summary)>, Vec<String>) {
    let configured = presentation::styles(host);
    let mut chosen = Vec::new();
    let mut refused = Vec::new();
    for field in schema.fields() {
        let name = field.name.as_str();
        let Some(function) = choice
            .get(name)
            .copied()
            .or_else(|| configured.get(name).and_then(|column| column.aggregate))
        else {
            continue;
        };
        if presentation::aggregates_for(field.data_type).contains(&function) {
            chosen.push((name.to_owned(), function));
        } else {
            refused.push(format!(
                "{name}: {} is not an aggregate for this type",
                function.as_str()
            ));
        }
    }
    (chosen, refused)
}

/// Runs one query through the provider and reads its result.
async fn ask(
    provider: &Rc<dyn opengrid_web_core::provider::DataProvider>,
    query: &str,
    mode: &str,
) -> Result<opengrid_datasource::QueryResult, String> {
    match JsFuture::from(provider.execute(query, mode)).await {
        Ok(value) => match value.as_string() {
            Some(json) => grid::parse_result(&json),
            None => Err("provider returned a non-string result".to_owned()),
        },
        Err(value) => Err(describe(&value)),
    }
}

/// The grouped counterpart of [`run_query`]: several queries, one result.
///
/// 1. the level-1 groups with their row counts — only when they are not loaded
///    yet or the filter changed;
/// 2. the level-2 groups of every open level-1 group that has none yet;
/// 3. the rows of the window, **one query per group it touches**, each filtered
///    on the group key and paged with `offset`/`limit`.
///
/// The answers are assembled into one ordinary `QueryResult` whose rows are the
/// window's display positions — group headers carry NULLs, which the renderer
/// never shows — and whose `total_count` is the display list's length. From
/// there it takes the same path as every other result: [`settle`], one render.
fn run_grouped(
    host: &HtmlElement,
    grid_runtime: &Rc<RefCell<GridRuntime>>,
    kind: QueryKind,
    focus: bool,
) {
    let Some(provider) = provider(host) else {
        return;
    };
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let columns = columns_of(host);
    let pool = pool_of(host);
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();

    let (mut sorts, filter, offset, generation, need_groups, choice) = {
        let mut runtime = grid_runtime.borrow_mut();
        let generation = runtime.generation + 1;
        runtime.generation = generation;
        // The filter row *and* the facets (point 66): a facet change makes the
        // group counts stale exactly like a filter change does.
        let filter = effective_filter(host, &runtime, None)
            .unwrap_or_else(|_| runtime.state.filter().cloned());
        let need_groups = runtime.groups_filter.as_ref() != Some(&filter)
            || !runtime.grouping.as_ref().is_some_and(Grouping::is_loaded);
        (
            runtime.state.sort_keys(),
            filter,
            runtime.state.window().offset,
            generation,
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
    if kind == QueryKind::Data {
        grid_runtime
            .borrow_mut()
            .state
            .set_status(GridStatus::Loading);
        render(host, false);
        refresh_facets(host);
    }

    let host = host.clone();
    let grid_runtime = grid_runtime.clone();
    spawn_local(async move {
        let stale = |runtime: &Rc<RefCell<GridRuntime>>| runtime.borrow().generation != generation;
        let fail = |message: String| settle(&host, generation, Err(message), focus);

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
            let schema = match ask(&provider, &probe, &mode).await {
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
            let result = match ask(&provider, &query, &mode).await {
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
                runtime.groups_filter = Some(filter.clone());
                if let Some(grouping) = runtime.grouping.as_mut() {
                    grouping.set_groups(groups);
                }
            }

            // The grand total: the same aggregates, no `group` — one row.
            let query = grouping::total_query_json(&source, filter.as_ref(), &aggregates);
            let total = match ask(&provider, &query, &mode).await {
                Ok(result) => grouping::total_from(&result),
                Err(message) => return fail(message),
            };
            if stale(&grid_runtime) {
                return;
            }
            if let Some(grouping) = grid_runtime.borrow_mut().grouping.as_mut() {
                grouping.set_total(total);
            }
        }

        // 2. The level-2 groups of open level-1 groups that have none yet.
        let (missing, second) = {
            let runtime = grid_runtime.borrow();
            let Some(grouping) = runtime.grouping.as_ref() else {
                return;
            };
            (grouping.missing_children(), grouping.by().get(1).cloned())
        };
        if let Some(second) = second {
            for key in missing {
                let (scoped, aggregates) = {
                    let runtime = grid_runtime.borrow();
                    let Some(grouping) = runtime.grouping.as_ref() else {
                        return;
                    };
                    (
                        grouping.filter_for(std::slice::from_ref(&key), filter.as_ref()),
                        grouping.aggregates().to_vec(),
                    )
                };
                let query =
                    grouping::group_query_json(&source, &second, scoped.as_ref(), &aggregates);
                let result = match ask(&provider, &query, &mode).await {
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
                    grouping.set_children(&key, children);
                }
            }
        }

        // 3. The rows of the window, one query per group it touches.
        let (fetches, total, offset) = {
            let mut runtime = grid_runtime.borrow_mut();
            let Some(grouping) = runtime.grouping.as_ref() else {
                return;
            };
            let total = grouping.len();
            // A toggle can shorten the list under the window; keep the window
            // inside it, the way the scroll math clamps an ordinary one.
            let offset = grid::window_offset_for_row(offset.min(total), total, pool);
            let fetches = grouping.fetches(offset, pool);
            runtime.state.set_window(Window::new(offset, pool));
            (fetches, total, offset)
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
            let result = match ask(&provider, &query, &mode).await {
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
                match ask(&provider, &query, &mode).await {
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
        let result = opengrid_datasource::QueryResult {
            schema,
            columns: page,
            total_count: total,
        };
        settle(&host, generation, Ok(result), focus);
    });
}

/// Opens or closes the group whose header is at `position`.
///
/// The focus stays where it is: a group's own position does not move when it
/// opens — only what comes after it does. The change is said once, riding with
/// the result that follows (`set_notice`), and the window is re-asked without
/// the "loading" line, so the reader hears the group, not the machinery.
fn toggle_group(host: &HtmlElement, position: u64) {
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
        let texts = texts(host);
        let label = grid::group_value_text(
            &value,
            &column,
            schema.fields(),
            &texts,
            &Formatter::new(&formats(host), &schema),
        );
        texts.group_toggled(&label, count, open)
    };
    runtime.borrow_mut().state.set_notice(message);
    run_query(host, QueryKind::Window, true);
    dispatch_view(host);
}

/// The display position of the group header the active cell stands on, if any.
fn active_group(host: &HtmlElement) -> Option<(u64, bool)> {
    let runtime = runtime(host)?;
    let borrowed = runtime.borrow();
    let row = borrowed.active.row()?;
    match borrowed.grouping.as_ref()?.item_at(row)? {
        grouping::Item::Group { expanded, .. } => Some((row, expanded)),
        // The total opens nothing; `Enter` on it is simply not a toggle.
        grouping::Item::Row { .. } | grouping::Item::Total { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// The column menu (point 64)
// ---------------------------------------------------------------------------

/// Opens the menu of the column at `col`, focus on its first entry.
///
/// Built when it opens rather than kept in the skeleton: it shows *current*
/// state — which sort is checked, which aggregate, whether the column is
/// grouped — and a menu that is rebuilt cannot be stale.
fn open_column_menu(host: &HtmlElement, col: usize) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    close_column_menu(host, false);

    let texts = texts(host);
    let (entries, name) = {
        let borrowed = runtime.borrow();
        let Some(field) = borrowed.state.schema().fields().get(col).cloned() else {
            return;
        };
        let name = field.name.as_str().to_owned();
        let sort = borrowed
            .state
            .sort_keys()
            .into_iter()
            .find(|(column, _)| column == &name)
            .map(|(_, direction)| direction);
        let aggregate = borrowed.aggregate_choice.get(&name).copied().or_else(|| {
            borrowed
                .grouping
                .as_ref()
                .and_then(|grouping| {
                    grouping
                        .aggregates()
                        .iter()
                        .find(|(column, _)| column == &name)
                        .map(|(_, function)| *function)
                })
                .or_else(|| {
                    presentation::styles(host)
                        .get(&name)
                        .and_then(|column| column.aggregate)
                })
        });
        let group_by = borrowed
            .grouping
            .as_ref()
            .map(|grouping| grouping.by().to_vec())
            .unwrap_or_default();
        let column = crate::column_menu::Column {
            name: &name,
            data_type: field.data_type,
            sort,
            aggregate,
            group_by: &group_by,
            can_group: !host.has_attribute(PAGE_SIZE_ATTRIBUTE),
            visible: borrowed.state.schema().fields().len(),
        };
        (crate::column_menu::entries(&column, &texts), name)
    };

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    for (attribute, value) in [
        ("part", "column-menu"),
        ("role", "menu"),
        ("popover", "auto"),
        ("data-col", &col.to_string()),
    ] {
        let _ = menu.set_attribute(attribute, value);
    }
    let _ = menu.set_attribute("aria-label", &texts.column_menu(&name));
    // Our words throughout — the menu names no data of the page's except the
    // column in its label, and that one is also the page's word for it.
    if !texts.lang.trim().is_empty() {
        let _ = menu.set_attribute("lang", &texts.lang);
    }
    append_entries(&document, &menu, &entries);
    let _ = root.append_child(&menu);

    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = menu.show_popover();
    place_menu(&root, &menu, col);
    if let Ok(Some(first)) = menu.query_selector("[role^=\"menuitem\"]")
        && let Ok(first) = first.dyn_into::<HtmlElement>()
    {
        let _ = first.focus();
    }
}

/// Writes the entries into the menu element.
fn append_entries(
    document: &web_sys::Document,
    parent: &Element,
    entries: &[crate::column_menu::Entry],
) {
    use crate::column_menu::Entry;
    for entry in entries {
        match entry {
            Entry::Separator => {
                if let Ok(line) = document.create_element("div") {
                    let _ = line.set_attribute("role", "separator");
                    let _ = parent.append_child(&line);
                }
            }
            Entry::Group { label, entries } => {
                let Ok(group) = document.create_element("div") else {
                    continue;
                };
                let _ = group.set_attribute("role", "group");
                // The group's name, visible and referenced: `aria-labelledby`
                // points inside the same shadow root, which E8/R6 allows.
                if let Ok(caption) = document.create_element("div") {
                    let id = format!("og-menu-{}", label.len());
                    let _ = caption.set_attribute("part", "menu-label");
                    let _ = caption.set_attribute("id", &id);
                    caption.set_text_content(Some(label));
                    let _ = group.append_child(&caption);
                    let _ = group.set_attribute("aria-labelledby", &id);
                }
                append_entries(document, &group, entries);
                let _ = parent.append_child(&group);
            }
            Entry::Item {
                action,
                label,
                checked,
            } => {
                let Ok(item) = document.create_element("div") else {
                    continue;
                };
                match checked {
                    Some(checked) => {
                        let _ = item.set_attribute("role", "menuitemradio");
                        let _ = item.set_attribute("aria-checked", &checked.to_string());
                    }
                    None => {
                        let _ = item.set_attribute("role", "menuitem");
                    }
                }
                let _ = item.set_attribute("tabindex", "-1");
                let _ = item.set_attribute("data-action", action);
                item.set_text_content(Some(label));
                let _ = parent.append_child(&item);
            }
        }
    }
}

/// Puts the menu under its header — never over it (WCAG 2.2 §2.4.11): the
/// focus returns there when the menu closes, and a menu that covered it would
/// have hidden where the reader is going back to. Right-aligned when it would
/// run off the right edge, above the header when it would run off the bottom.
fn place_menu(root: &ShadowRoot, menu: &HtmlElement, col: usize) {
    let Ok(Some(header)) = root.query_selector(&format!("th[data-col=\"{col}\"]")) else {
        return;
    };
    let cell = header.get_bounding_client_rect();
    let own = menu.get_bounding_client_rect();
    let (width, height) = web_sys::window()
        .map(|window| {
            (
                window
                    .inner_width()
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(1024.0),
                window
                    .inner_height()
                    .ok()
                    .and_then(|value| value.as_f64())
                    .unwrap_or(768.0),
            )
        })
        .unwrap_or((1024.0, 768.0));
    const GAP: f64 = 4.0;
    let mut left = cell.left();
    if left + own.width() > width - GAP {
        left = (cell.right() - own.width()).max(GAP);
    }
    let mut top = cell.bottom() + GAP;
    if top + own.height() > height - GAP && cell.top() - own.height() - GAP >= GAP {
        top = cell.top() - own.height() - GAP;
    }
    let _ = menu.style().set_property("left", &format!("{left}px"));
    let _ = menu.style().set_property("top", &format!("{top}px"));
}

/// Closes an open menu; with `refocus`, the focus goes back to its header.
fn close_column_menu(host: &HtmlElement, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(menus) = root.query_selector_all("[part=\"column-menu\"]") else {
        return;
    };
    let mut col = None;
    for index in 0..menus.length() {
        let Some(menu) = menus
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        else {
            continue;
        };
        col = col.or_else(|| {
            menu.get_attribute("data-col")
                .and_then(|col| col.parse().ok())
        });
        let _ = menu.hide_popover();
        menu.remove();
    }
    if refocus
        && let Some(col) = col
        && let Some(runtime) = runtime(host)
    {
        runtime.borrow_mut().set_active(ActiveCell::Header { col });
        focus_active(host);
    }
}

/// The keys inside the menu: the arrows, `Home`/`End`, `Enter`/`Space`,
/// `Escape` and `Tab` — the protocol written down in point 64 before building.
fn on_menu_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(menu)) = target.closest("[part=\"column-menu\"]") else {
        return;
    };
    let Ok(items) = menu.query_selector_all("[role^=\"menuitem\"]") else {
        return;
    };
    let items: Vec<HtmlElement> = (0..items.length())
        .filter_map(|index| items.item(index))
        .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
        .collect();
    if items.is_empty() {
        return;
    }
    let at = items
        .iter()
        .position(|item| item.is_same_node(Some(target)))
        .unwrap_or(0);
    let go = |index: usize| {
        let _ = items[index].focus();
    };
    match event.key().as_str() {
        "ArrowDown" => {
            event.prevent_default();
            go((at + 1) % items.len());
        }
        "ArrowUp" => {
            event.prevent_default();
            go((at + items.len() - 1) % items.len());
        }
        "Home" => {
            event.prevent_default();
            go(0);
        }
        "End" => {
            event.prevent_default();
            go(items.len() - 1);
        }
        "Enter" | " " => {
            event.prevent_default();
            activate_menu_item(host, &items[at]);
        }
        "Escape" => {
            event.prevent_default();
            close_column_menu(host, true);
        }
        // Not prevented: the focus goes back to the header and `Tab` then moves
        // on from there, as it always does from a header cell. No trap — it is
        // a menu, not a dialog.
        "Tab" => close_column_menu(host, true),
        _ => {}
    }
}

/// Does what an entry says, closes the menu, and puts the focus back.
///
/// Nothing here announces: the result of each action does, through the one
/// live region (point 41) — a sort re-queries, a hidden column says so, a
/// grouping reloads.
fn activate_menu_item(host: &HtmlElement, item: &Element) {
    let Some(action) = item.get_attribute("data-action") else {
        return;
    };
    let Some(col) = item
        .closest("[part=\"column-menu\"]")
        .ok()
        .flatten()
        .and_then(|menu| menu.get_attribute("data-col"))
        .and_then(|col| col.parse::<usize>().ok())
    else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(name) = runtime
        .borrow()
        .state
        .schema()
        .fields()
        .get(col)
        .map(|field| field.name.as_str().to_owned())
    else {
        return;
    };
    close_column_menu(host, true);

    match action.as_str() {
        "sort:asc" | "sort:desc" => {
            let direction = if action == "sort:asc" {
                SortDirection::Asc
            } else {
                SortDirection::Desc
            };
            let pool = pool_of(host);
            {
                let mut borrowed = runtime.borrow_mut();
                if let Ok(field) = opengrid_types::FieldName::new(&name) {
                    borrowed.state.set_sort(vec![Sort {
                        field,
                        direction,
                        nulls: Default::default(),
                        collation: Default::default(),
                    }]);
                }
                borrowed.state.set_window(Window::new(0, pool));
            }
            announce_if_cleared(host, &runtime);
            run_query(host, QueryKind::Data, true);
            dispatch_view(host);
        }
        "filter" => {
            // Into the filter row, where filtering already lives (point 48). A
            // column whose operator takes no value has its field disabled, so
            // the operator is where the focus can go.
            if let Some(root) = host.shadow_root() {
                let field = root
                    .query_selector(&format!("input[data-col=\"{col}\"]:not([disabled])"))
                    .ok()
                    .flatten()
                    .or_else(|| {
                        root.query_selector(&format!("select[data-col=\"{col}\"]"))
                            .ok()
                            .flatten()
                    });
                if let Some(field) = field.and_then(|field| field.dyn_into::<HtmlElement>().ok()) {
                    let _ = field.focus();
                }
            }
        }
        "hide" => set_column_hidden(host, &name, true),
        grouping if grouping.starts_with("group:") => {
            let current = runtime
                .borrow()
                .grouping
                .as_ref()
                .map(|grouping| grouping.by().to_vec())
                .unwrap_or_default();
            let next = crate::column_menu::regrouped(grouping, &name, &current);
            if next.is_empty() {
                let _ = host.remove_attribute(grid::GROUP_BY_ATTRIBUTE);
            } else {
                let _ = host.set_attribute(grid::GROUP_BY_ATTRIBUTE, &next.join(","));
            }
        }
        aggregate if aggregate.starts_with("aggregate:") => {
            let choice = presentation::aggregate_from(&aggregate["aggregate:".len()..]);
            {
                let mut borrowed = runtime.borrow_mut();
                match choice {
                    Some(function) => {
                        borrowed.aggregate_choice.insert(name.clone(), function);
                    }
                    None => {
                        borrowed.aggregate_choice.remove(&name);
                    }
                }
                // The counts were asked together with the old aggregates.
                if let Some(grouping) = borrowed.grouping.as_mut() {
                    grouping.invalidate();
                }
                borrowed.groups_filter = None;
            }
            run_query(host, QueryKind::Window, true);
            dispatch_view(host);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The toolbar and the chips (point 65)
// ---------------------------------------------------------------------------

/// Brings the toolbar, the chips and the filter row in line with the state.
///
/// Runs after every render, so it has to be cheap and it must **not** touch
/// what did not change: the chips are rebuilt only when what they say changed.
/// A scroll frame that rebuilt them would take the focus off a chip the reader
/// is standing on.
fn sync_chrome(host: &HtmlElement) {
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
struct Chip {
    /// The filter or grouping in words: "country is DE".
    text: String,
    /// `column:<name>` for a filter, `group` for the grouping.
    removes: String,
}

/// The chips the current view calls for.
fn chips_of(host: &HtmlElement) -> Vec<Chip> {
    let Some(view) = current_view(host) else {
        return Vec::new();
    };
    let texts = texts(host);
    let mut out = Vec::new();
    if !view.group.is_empty() {
        out.push(Chip {
            text: texts.group_chip(&view.group.join(" \u{203A} ")),
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
        let token = entry.op.as_str();
        let index = crate::shared::FILTER_OPERATORS
            .iter()
            .position(|op| *op == token)
            .unwrap_or(0);
        let operator = texts.operator(index, token);
        let text = if grid::takes_value(token) {
            format!("{} {operator} {}", entry.column, entry.value.trim())
        } else {
            format!("{} {operator}", entry.column)
        };
        out.push(Chip {
            text,
            removes: format!("column:{}", entry.column),
        });
    }
    out
}

/// Draws the chips, but only when they say something different from before.
fn draw_chips(host: &HtmlElement, root: &ShadowRoot) {
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
        // The chip's words are the page's column name and value mixed with our
        // operator word — no `lang` fits both halves (the open question from
        // phase E (k), for point 71).
        if let Ok(text) = document.create_element("span") {
            text.set_text_content(Some(&chip.text));
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
fn toggle_filter_row(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.filter_row = !borrowed.filter_row;
    }
    sync_chrome(host);
    run_query(host, QueryKind::Window, false);
    dispatch_view(host);
}

/// Resets one column of the filter row to "no filter".
///
/// Not only the value: an `is_null` filter takes none, so emptying the field
/// would leave it standing. The operator goes back to the column's first
/// offered one.
fn reset_filter_column(root: &ShadowRoot, col: usize) {
    if let Ok(Some(node)) = root.query_selector(&format!("select[data-col=\"{col}\"]"))
        && let Ok(select) = node.dyn_into::<HtmlSelectElement>()
        && let Ok(options) = select.query_selector_all("option")
    {
        for index in 0..options.length() {
            if let Some(option) = options
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
                && !option.has_attribute("hidden")
                && let Some(value) = option.get_attribute("value")
            {
                select.set_value(&value);
                break;
            }
        }
    }
    if let Ok(Some(node)) = root.query_selector(&format!("input[data-col=\"{col}\"]"))
        && let Ok(input) = node.dyn_into::<HtmlInputElement>()
    {
        input.set_value("");
        input.set_disabled(false);
    }
}

/// Removes what one chip stands for, says so, and keeps the focus in the chips.
fn remove_chip(host: &HtmlElement, button: &Element) {
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
fn clear_chips(host: &HtmlElement) {
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

// ---------------------------------------------------------------------------
// Facets (point 66)
// ---------------------------------------------------------------------------

/// The filter a query runs under: the filter row's **and** every facet's —
/// except `except`, which is how a facet is counted without its own
/// restriction. A bound that is not a value of its column is a sentence for the
/// status line, not a silently dropped half of a range.
fn effective_filter(
    host: &HtmlElement,
    runtime: &GridRuntime,
    except: Option<&str>,
) -> Result<Option<FilterExpr>, String> {
    // The free text of the search field is one more filter under the facets
    // (point 67) — part of the base, so the facet counts see it too.
    let text = crate::search::free_text(&runtime.search_text, runtime.state.schema());
    let base = match (runtime.state.filter().cloned(), text) {
        (Some(row), Some(text)) => Some(FilterExpr::And(vec![row, text])),
        (row, text) => row.or(text),
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
fn clear_selection(selection: &crate::facets::Selection) -> crate::facets::Selection {
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
fn refresh_facets(host: &HtmlElement) {
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

    let host = host.clone();
    spawn_local(async move {
        let mut queries = 0usize;
        for (column, needs_domain, filter) in plan {
            if needs_domain {
                let query = grouping::group_query_json(&source, &column, None, &[]);
                queries += 1;
                let Ok(result) = ask(&provider, &query, &mode).await else {
                    continue;
                };
                if let Ok(domain) = grouping::groups_from(&result) {
                    grid_runtime
                        .borrow_mut()
                        .facet_domain
                        .insert(column.clone(), domain);
                }
            }
            let query = grouping::group_query_json(&source, &column, filter.as_ref(), &[]);
            queries += 1;
            let Ok(result) = ask(&provider, &query, &mode).await else {
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
        &Formatter::new(&formats(host), schema),
    )
}

/// What a facet's chip says.
fn facet_chip_text(
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
                            serde_json::Value::Null => Value::Null,
                            serde_json::Value::String(text) => Value::Utf8(text.clone()),
                            other => Value::Utf8(other.to_string()),
                        });
                    facet_label(host, column, &value, schema)
                })
                .collect();
            if labels.len() == 1 {
                let is = texts.operator(2, "eq");
                format!("{column} {is} {}", labels[0])
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
                "{column} {} \u{2013} {}",
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
fn draw_facets(host: &HtmlElement, root: &ShadowRoot) {
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
                legend.set_text_content(Some(column));
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
            let parsed = serde_json::from_str::<serde_json::Value>(&key).ok();
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
fn apply_facets(host: &HtmlElement) {
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
fn on_facet_change(host: &HtmlElement, target: &Element) {
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
fn toggle_facet_value(host: &HtmlElement, pill: &Element) {
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
    let Ok(value) = serde_json::from_str::<serde_json::Value>(key) else {
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
fn reset_facets(host: &HtmlElement) {
    if let Some(runtime) = runtime(host) {
        for (_, selection) in runtime.borrow_mut().facets.iter_mut() {
            *selection = clear_selection(selection);
        }
    }
    apply_facets(host);
}

// ---------------------------------------------------------------------------
// The search field (point 67)
// ---------------------------------------------------------------------------

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
fn on_search_input(event: Event) {
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
    let offered = crate::search::suggestions(&value, &texts.query_and, &columns);
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
        option.set_text_content(Some(column));
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
fn on_search_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
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
fn take_suggestion(host: &HtmlElement, option: &Element) {
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
        &column,
    ));
    update_search(host, &root);
    let _ = input.focus();
}

/// Applies the field: an expression becomes filter-row entries, anything else
/// a free-text search.
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
        match crate::search::parse(&value, &texts.query_and, &schema) {
            Ok(entries) => {
                // Written into the filter row's own fields and applied by its
                // own code: one place a filter lives (F5).
                for entry in &entries {
                    let Some(col) = columns.iter().position(|name| name == &entry.column) else {
                        continue;
                    };
                    if let Ok(Some(node)) =
                        root.query_selector(&format!("select[data-col=\"{col}\"]"))
                        && let Ok(select) = node.dyn_into::<HtmlSelectElement>()
                    {
                        select.set_value(entry.op.as_str());
                    }
                    if let Ok(Some(node)) =
                        root.query_selector(&format!("input[data-col=\"{col}\"]"))
                        && let Ok(field) = node.dyn_into::<HtmlInputElement>()
                    {
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

// ---------------------------------------------------------------------------
// The empty state (point 68)
// ---------------------------------------------------------------------------

/// Shows the empty state when a result has no rows.
///
/// Two sentences, because they are two situations: no row matches **these
/// filters** — and then there is a way out, the reset — or the source has no
/// rows at all, where a reset would promise something it cannot do.
fn draw_empty(host: &HtmlElement, root: &ShadowRoot, runtime: &Rc<RefCell<GridRuntime>>) {
    let Ok(Some(panel)) = root.query_selector("[part=\"empty\"]") else {
        return;
    };
    let (empty, filtered) = {
        let borrowed = runtime.borrow();
        let empty = matches!(borrowed.state.status(), GridStatus::Empty);
        let filtered = effective_filter(host, &borrowed, None).is_ok_and(|filter| filter.is_some());
        (empty, filtered)
    };
    if !empty {
        let _ = panel.set_attribute("hidden", "");
        return;
    }
    let texts = texts(host);
    if let Ok(Some(sentence)) = panel.query_selector("[part=\"empty-text\"]") {
        let text = if filtered {
            &texts.empty_filtered
        } else {
            &texts.empty_source
        };
        if sentence.text_content().as_deref() != Some(text.as_str()) {
            sentence.set_text_content(Some(text));
        }
    }
    if let Ok(Some(reset)) = panel.query_selector("[part=\"empty-reset\"]") {
        let _ = if filtered {
            reset.remove_attribute("hidden")
        } else {
            reset.set_attribute("hidden", "")
        };
    }
    let _ = panel.remove_attribute("hidden");
}
