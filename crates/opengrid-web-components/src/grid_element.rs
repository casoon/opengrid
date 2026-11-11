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

use opengrid_grid::{CellRef, GridState, GridStatus, Patch as GridPatch, RowKind, Window};
use opengrid_query::{CmpOp, FilterExpr, Sort, SortDirection};
use opengrid_types::{DataType, Value};
use opengrid_web_core::element::{
    ARIA_LABEL_ATTRIBUTE, LABEL_ATTRIBUTE, attach_open_shadow_root, define, mirror_label,
};
use opengrid_web_core::host;
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::columns::{self, Pin, WIDTH_STEP};
use crate::formats::{CellFormat, Formatter, formats};
use crate::grid_element_events::{
    BULK_EVENT, CELL_EVENT, PASTE_EVENT, RECORD_OPEN_EVENT, ROW_ACTION_EVENT, ROW_ADD_EVENT,
    SELECTION_EVENT, VIEW_EVENT,
};

mod builder;
mod bulk;
mod chips;
mod column_menu;
mod column_ops;
mod combo;
mod editing;
mod facets;
mod filter_row;
mod group_path;
mod history;
mod paste;
mod presentation_columns;
mod print;
mod quick_doors;
mod range;
mod record;
mod search;
mod template;
mod tree_path;
mod validate;
mod view;
use opengrid_web_core::renderer::{Dom, WebRenderer};

use crate::element::{
    MenuAct, answer_size, clear_root, describe, dispatch_query, menu_items, menu_key, now,
    place_under,
};
use crate::grid::{
    self, ActiveCell, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, FilterEntry, GRID_TAG, GridKey,
    GridNodes, GridSkeleton, MODE_ATTRIBUTE, PAGE_SIZE_ATTRIBUTE, ROW_HEIGHT_PROPERTY,
    WINDOW_SIZE_ATTRIBUTE,
};
use crate::grouping::{self, Grouping};
use crate::presentation;
use crate::texts::texts;
use crate::tree::{self, Tree};
use crate::view::GridView;

use chips::{chips_of, clear_chips, draw_chips, remove_chip, sync_chrome, toggle_filter_row};
use column_menu::{activate_menu_item, on_menu_key, open_column_menu};
use column_ops::{
    Arrangement, adopt_sheets, announce, announce_refusals, apply_widths, arrangement, bands_of,
    close_columns, columns_of, move_band, move_column, observe_columns, open_columns, pin_column,
    pin_of, pool_of, rearrange, resize_column, retree, set_column_hidden, step_pin,
};
pub(crate) use editing::{CHOICES_KEY, write_row_state};
use editing::{
    add_record, after_new_row_enter, begin_edit, build_editor, dispatch_row_add, editor_text,
    end_edit, focus_editor, formatter,
};
use facets::{
    apply_facets, clear_selection, draw_facets, effective_filter, facet_chip_text, on_facet_change,
    page_filter, reader_filter, refresh_facets, reset_facets, show_source, toggle_facet_value,
};
use filter_row::{
    Decorated, OperatorControl, ValueControl, apply_filters, fix_operator_choices,
    fix_presentation, is_filter_control, on_filter_change, on_filter_clear, operator_word,
    read_filter_entries,
};
use group_path::{active_group, run_grouped, toggle_group};
use presentation_columns::recolumn;
pub(crate) use presentation_columns::set_columns;
use quick_doors::{
    apply_filter_dialog, close_operator_menu, close_quick, fill_conditions, on_dialog_key,
    on_group_menu_key, on_operator_menu_key, open_filter_dialog, open_group_menu,
    open_operator_menu, pick_grouping, pick_operator, quick_button, show_value_field, shown_fields,
    sync_quick_buttons,
};
use search::{on_search_input, on_search_key, take_suggestion};
use tree_path::{active_tree, run_tree, toggle_tree};
use view::{Restore, apply_view, applying_view, current_query, restore_then_query, to_js};
pub(crate) use view::{
    current_view, dispatch_view, prepare_print, read_query, read_view, write_view,
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
    // Another provider may answer the same source name with other types.
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().known.clear();
    }
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
    /// A focus request that a newer query superseded. The answer that does land
    /// carries it, or nobody would have the focus (issue #223).
    pending_focus: bool,
    /// The grouping, while `group-by` is set and valid (point 62).
    grouping: Option<Grouping>,
    /// The filter the loaded groups were counted under — a different one means
    /// the counts are stale and the group query has to run again.
    groups_filter: Option<Option<FilterExpr>>,
    /// The selection while grouped (issue #322): records by `row-key`. The
    /// state shows it at the positions the records sit at in the window.
    picked: grouping::Picked,
    /// A view's selection, until the source has said which of its keys name
    /// a record (issue #322).
    restore: Option<Restore>,
    /// What the check of a view's selection found — keys that name no record,
    /// or that it failed — to be said with the result that follows.
    restore_said: Option<String>,
    /// Bumped by every view applied: an older view's check is void.
    view_round: u64,
    /// The tree, while `tree` is set and allowed (plan point 123, E38).
    tree: Option<Tree>,
    /// The filter and order the loaded levels were asked under: another one
    /// makes them stale.
    tree_asked: Option<String>,
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
    /// Queries asked for the rows or groups shown now (issue #33); the footer
    /// adds the facets' to it.
    queries: usize,
    /// Bumped per count round; an older round's answers are dropped.
    facet_generation: u64,
    /// The free text the search field applied (point 67), or empty.
    search_text: String,
    /// The nested filter (issue #324): the one tree of conditions joined by
    /// `and` and `or`, in the wire form — written by the search expression
    /// and by the builder, and-ed onto the filter row's. A rebuild keeps it,
    /// like the facets.
    nested: Option<FilterExpr>,
    /// The pinned columns take more than their share of the width and scroll
    /// with the others (issue #316) — and whether that still has to be said.
    pins_released: bool,
    release_unsaid: bool,
    /// How far the viewport is scrolled sideways, as of its last scroll event:
    /// where it stood before a browser scrolled to a pinned cell's place in
    /// the row ([`focus_cell`]).
    scroll_left: i32,
    /// Every column a result has typed, by name (plan point 88). Kept across
    /// rebuilds — a view, a shown column — so a filter meets its real type;
    /// forgotten when the `datasource` changes.
    known: std::collections::BTreeMap<String, opengrid_types::Field>,
    /// A view's filter was typed before its columns' types were known; the
    /// next data query asks for the schema first (plan point 88).
    retype_filter: bool,
    /// The viewport's scroll offset, as the reader left it. A browser forgets
    /// the offset of an element that leaves the document, even for a move;
    /// this is what puts it back (point 74).
    scroll_top: i32,
    /// How many pixels the rows sit higher than `row * row_height` while the
    /// result is taller than the sizer may be ([`grid::MAX_SIZER_PX`]); the
    /// virtual scroll position is `scroll_top + shift`. 0 otherwise.
    shift: u64,
    /// Which of viewport (`true`) and filter row (`false`) was just scrolled
    /// to follow the other, and to where (issue #62). A scroll event from it
    /// at exactly that position is the echo; passing it back would pull the
    /// other to where it cannot go — the filter row reaches further than the
    /// rows, past its "Clear". Anywhere else, the reader moved it.
    scroll_echo: Option<(bool, i32)>,
    /// The types and markers [`fix_operator_choices`] and [`fix_presentation`]
    /// last wrote into the skeleton. They belong to a result (the types arrive
    /// with it) and to the pool's cells, which exist from the skeleton on, so a
    /// frame that finds the same ones writes nothing; a new skeleton clears it.
    decorated: Option<Decorated>,
    /// The source was replaced (issue #182) and the next result is the new
    /// one: the key of the focused record, if it had one. The pinned slot would
    /// keep showing the old row, so the result drops the pin and the focus
    /// follows the record.
    replaced: Option<Option<Value>>,
    /// A paste shown and not yet applied (issue #184).
    paste: Option<paste::Pending>,
    /// Bumped by every result of the whole query (not by a scroll's window): a
    /// paste prepared against an earlier one lands on other rows (issue #228).
    epoch: u64,
    /// The finding the live region has said for the cell being edited, so it is
    /// said once (issue #188).
    validation_said: Option<String>,
    /// The block of cells marked by `Shift`+arrows or the pointer (issue #327).
    range: Option<opengrid_grid::CellRange>,
    /// Where a drag with the pointer started, while the button is down.
    drag_from: Option<CellRef>,
    /// Bumped by every `set_cell_templates`: a cell drawn under another round
    /// is drawn again (issue #317).
    templates_round: u64,
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
    let runtime = Rc::new(RefCell::new(fresh_runtime(host, &Default::default())));
    attach_runtime(host, &runtime);
    runtime
}

/// A fresh runtime: display schema from `columns`, window at offset 0 and the
/// top-left header cell active.
fn fresh_runtime(
    host: &HtmlElement,
    known: &std::collections::BTreeMap<String, opengrid_types::Field>,
) -> GridRuntime {
    let columns = columns_of(host);
    let pool = pool_of(host);
    let row_height = resolve_row_height(host);
    let mut state = GridState::new(grid::known_schema(&columns, known));
    state.set_row_key(row_key_of(host));
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
        pending_focus: false,
        grouping: grouping_of(host).ok().flatten(),
        groups_filter: None,
        picked: Default::default(),
        restore: None,
        restore_said: None,
        view_round: 0,
        tree: tree_of(host).ok().flatten(),
        tree_asked: None,
        aggregate_choice: Default::default(),
        filter_row: true,
        facets: Vec::new(),
        facet_domain: Default::default(),
        facet_counts: Default::default(),
        facet_queries: 0,
        queries: 0,
        facet_generation: 0,
        search_text: String::new(),
        nested: None,
        pins_released: false,
        release_unsaid: false,
        scroll_left: 0,
        known: known.clone(),
        retype_filter: false,
        scroll_top: 0,
        shift: 0,
        scroll_echo: None,
        decorated: None,
        replaced: None,
        paste: None,
        epoch: 0,
        validation_said: None,
        range: None,
        drag_from: None,
        templates_round: 0,
    }
}

/// The `row-key` field, if the attribute names a valid one.
fn row_key_of(host: &HtmlElement) -> Option<opengrid_types::FieldName> {
    host.get_attribute(grid::ROW_KEY_ATTRIBUTE)
        .and_then(|name| opengrid_types::FieldName::new(name.trim()).ok())
}

/// Whether the host pages: a `page-size` that reads as a size. `page-size="0"`
/// or a word is no paging, so it refuses nothing (issue #233).
fn is_paging(host: &HtmlElement) -> bool {
    grid::parse_page_size(host.get_attribute(PAGE_SIZE_ATTRIBUTE).as_deref()).is_some()
}

/// Whether the host names its records: a `row-key` that is a field name.
fn has_row_key(host: &HtmlElement) -> bool {
    row_key_of(host).is_some()
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
        let known = runtime.borrow().known.clone();
        let mut fresh = fresh_runtime(host, &known);
        let mut runtime = runtime.borrow_mut();
        // What the page said about cells names records, and a rebuild keeps
        // the records (issue #153).
        fresh.state.restore_saves(runtime.state.take_saves());
        // The rows the reader added and the history are the same: they wait
        // for the page, and a column moving is not their end (issue #226). A
        // tree or a grouping has no end to add after (see `sync_add_row`).
        let edits = runtime.state.take_edits();
        if fresh.tree.is_none() && fresh.grouping.is_none() {
            fresh.state.restore_edits(edits);
        }
        fresh.state.set_row_key(row_key_of(host));
        runtime.state = fresh.state;
        runtime.set_active(fresh.active);
        runtime.view = None;
        runtime.dom = None;
        runtime.viewport = None;
        runtime.slots = fresh.slots;
        runtime.pending_offset = None;
        runtime.raf_pending = false;
        runtime.scroll_top = 0;
        runtime.scroll_left = 0;
        runtime.shift = 0;
        // The preview went with the DOM (`clear_root`); what it would apply
        // names cells of a layout that is gone.
        runtime.paste = None;
        runtime.range = None;
        runtime.epoch += 1;
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
        // A tree the same way: the same key and parent keep what is open.
        let keep = matches!(
            (&runtime.tree, &fresh.tree),
            (Some(old), Some(new)) if old.key() == new.key() && old.parent() == new.parent()
        );
        if keep {
            if let Some(tree) = runtime.tree.as_mut() {
                tree.invalidate();
            }
        } else {
            runtime.tree = fresh.tree;
        }
        runtime.tree_asked = None;
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
        adopt_sheets(&root);
        add_listeners(&root);
        // A provider that replaced the source this grid shows (issue #182).
        let replaced = Closure::<dyn FnMut(Event)>::new(on_source_replaced).into_js_value();
        let _ = host.add_event_listener_with_callback(
            crate::grid_element_events::SOURCE_REPLACED_EVENT,
            replaced.unchecked_ref(),
        );
        runtime.borrow_mut().row_height = height;
        // A `group-by` in the markup is read before anything else happens; one
        // the grid refuses is said with the first result, and it runs ungrouped.
        announce_refusals(&host, &runtime);
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

/// The source this grid shows was replaced: the same query runs again over the
/// new rows, and the view — sort, filter, focus — stays (issue #182).
fn on_source_replaced(event: Event) {
    let Some(host) = event
        .current_target()
        .and_then(|target| target.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    if let Some(runtime) = runtime(&host) {
        let focused_in_grid = host
            .shadow_root()
            .and_then(|root| root.active_element())
            .is_some_and(|element| element.tag_name().eq_ignore_ascii_case("td"));
        let selection_dropped;
        let keys_dropped;
        {
            let mut runtime = runtime.borrow_mut();
            let key = runtime
                .active
                .row()
                .and_then(|row| runtime.state.row_key(row).cloned());
            runtime.replaced = Some(key);
            runtime.state.added_mut().drop_saved();
            // What the reader did was on the source that is replaced.
            runtime.state.history_mut().clear();
            // A selection names positions, and other records sit in them now.
            selection_dropped = !runtime.state.invalidate_selection().is_empty();
            // One by key goes the same way (issue #322): whether its records
            // are in the new source is not known, and the rule is one.
            keys_dropped = runtime.picked.clear();
            if keys_dropped {
                runtime.state.note_selection_dropped();
            }
            // The aggregates of a group come from the fresh rows too.
            runtime.groups_filter = None;
            if let Some(grouping) = runtime.grouping.as_mut() {
                grouping.invalidate();
            }
            // The levels of a tree are the old source's: without this the
            // query finds them loaded and asks for nothing.
            if let Some(tree) = runtime.tree.as_mut() {
                tree.invalidate();
            }
            runtime.tree_asked = None;
            // So are the values a facet lists and their counts.
            runtime.facet_domain.clear();
            runtime.facet_counts.clear();
        }
        if keys_dropped {
            dispatch_keys(&host, &[]);
        } else if selection_dropped {
            announce_if_cleared(&host, &runtime);
        }
        start_following(&host, focused_in_grid);
        return;
    }
    start(&host);
}

/// [`start`], and the focus goes back to the active cell once the result lands.
fn start_following(host: &HtmlElement, focus: bool) {
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().known.clear();
    }
    ensure_skeleton(host);
    render(host, false);
    run_query(host, QueryKind::Data, focus);
}

/// The new rows are in: the pin is dropped and the active cell follows the
/// record it stood on, or stays at its place where the record is gone.
fn follow_replaced(runtime: &mut GridRuntime, key: Option<Value>) {
    runtime.slots.iter_mut().for_each(|slot| *slot = None);
    let Some(row) = runtime.active.row() else {
        return;
    };
    let total = runtime.state.row_count();
    let found = key.and_then(|key| runtime.state.row_of_key(&key));
    let row = found.unwrap_or(row.min(total.saturating_sub(1)));
    let active = match runtime.active {
        ActiveCell::Data(cell) if total > 0 => ActiveCell::Data(CellRef::new(row, cell.col)),
        ActiveCell::Select { .. } if total > 0 => ActiveCell::Select { row },
        _ => ActiveCell::Header { col: 0 },
    };
    runtime.set_active(active);
}

/// The empty row at the end is there while `add-row` is set and the grid
/// shows plain rows: a group header or a tree node has no end to add after, and
/// a page is not the end of the list (issue #183).
fn sync_add_row(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let on = host.has_attribute(grid::ADD_ROW_ATTRIBUTE)
        && !is_paging(host)
        && !host.has_attribute(grid::TREE_ATTRIBUTE)
        && runtime.borrow().grouping.is_none();
    let mut borrowed = runtime.borrow_mut();
    if borrowed.state.added().enabled() != on {
        borrowed.state.set_add_row(on);
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
    let virtual_top = grid::Placement {
        scroll_top: borrowed.scroll_top.max(0) as u64,
        shift: borrowed.shift,
    }
    .virtual_top();
    let offset = grid::window_offset(
        grid::visible_start(virtual_top, borrowed.row_height),
        borrowed.state.row_count(),
        window.count,
    );
    borrowed.state.set_window(Window::new(offset, window.count));
}

/// How a result taller than the sizer may be is scrolled, measured now: `None`
/// while its rows fit (nothing is scaled then) or the viewport has no scroll
/// range yet.
fn scroll_scale(runtime: &GridRuntime, viewport: &Element) -> Option<grid::ScrollScale> {
    // Asked on every scroll event: the layout is not read for a result that fits.
    if !grid::is_capped(runtime.state.row_count(), runtime.row_height) {
        return None;
    }
    let client = viewport.client_height();
    grid::ScrollScale::new(
        runtime.state.row_count(),
        runtime.row_height,
        (viewport.scroll_height() - client).max(0) as u64,
        client.max(0) as u64,
    )
}

/// Puts the scroll position where `row` is, when it is further from the viewport
/// than the browser's own scrolling (`scrollIntoView` of the focus) should be
/// trusted with: past [`grid::MAX_SIZER_PX`] a row far away sits outside the
/// sizer until the position is placed (issue #284). Nothing happens for a row
/// close by, or while the rows fit the sizer.
fn scroll_to_far_row(runtime: &Rc<RefCell<GridRuntime>>, row: u64) {
    let (viewport, top) = {
        let mut borrowed = runtime.borrow_mut();
        let Some(viewport) = borrowed.viewport.clone() else {
            return;
        };
        let Some(scale) = scroll_scale(&borrowed, &viewport) else {
            return;
        };
        let wanted = row.saturating_mul(borrowed.row_height);
        let current = grid::Placement {
            scroll_top: borrowed.scroll_top.max(0) as u64,
            shift: borrowed.shift,
        }
        .virtual_top();
        if wanted.abs_diff(current) <= scale.near() {
            return;
        }
        let placed = scale.place_virtual(wanted);
        let top = i32::try_from(placed.scroll_top).unwrap_or(i32::MAX);
        borrowed.scroll_top = top;
        borrowed.shift = placed.shift;
        (viewport, top)
    };
    viewport.set_scroll_top(top);
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
    if is_paging(host) {
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
        let scale = borrowed
            .viewport
            .as_ref()
            .and_then(|viewport| scroll_scale(&borrowed, viewport));
        let placed = match scale {
            Some(scale) => scale.place_virtual(top),
            None => grid::Placement {
                scroll_top: top,
                shift: 0,
            },
        };
        borrowed.scroll_top = i32::try_from(placed.scroll_top).unwrap_or(i32::MAX);
        borrowed.shift = placed.shift;
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
/// takes the focus back — if the grid had it.
pub(crate) fn retext(host: &HtmlElement) {
    if host.shadow_root().is_none() {
        return;
    }
    let Some(runtime) = runtime(host) else {
        return;
    };
    let had_focus = reskeleton(host, &runtime);
    run_query(host, QueryKind::Data, had_focus);
}

/// A new skeleton around the **same** state: the words or the markers of the
/// columns changed, not what is shown (points 48, 60, 88).
///
/// The grid lets go of its skeleton the way a parked grid does, and
/// [`ensure_skeleton`] carries the filter row and the search into the new one;
/// sort, filter, selection and the window stay in the state.
///
/// The focus comes back to the active cell only if it was in the grid, and the
/// answer says whether it was. Texts and a presentation arrive while a page
/// loads — a grid that took the focus then would take it from the page, and
/// a control of the page that sets them keeps the focus (the rule of
/// `set_view`).
fn reskeleton(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) -> bool {
    let had_focus = host
        .shadow_root()
        .is_some_and(|root| root.active_element().is_some());
    {
        let mut runtime = runtime.borrow_mut();
        runtime.view = None;
        runtime.dom = None;
        runtime.viewport = None;
    }
    follow_active_row(host, runtime);
    render(host, false);
    restore_scroll(runtime);
    if had_focus {
        focus_active(host);
    }
    had_focus
}

/// Writes `entries` back into the filter row after a rebuild.
///
/// The counterpart of [`read_filter_entries`]: the state keeps the filter as an
/// expression, but the controls are what the user reads, and an empty row above
/// filtered data is a lie.
fn write_filter_entries(root: &ShadowRoot, entries: &[FilterEntry]) {
    for (col, entry) in entries.iter().enumerate() {
        if let Some(operator) = OperatorControl::of(root, col) {
            // A column without a filter shows its type's default comparison,
            // not the entry's placeholder `eq` — "is" over a text column
            // that nobody filtered (issue #96).
            if grid::takes_value(entry.op.as_str()) && entry.value.trim().is_empty() {
                operator.reset();
            } else {
                operator.set_value(entry.op.as_str());
            }
        }
        if let Some(control) = ValueControl::of(root, col) {
            control.set_value(&entry.value);
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
        // A tree refuses `page-size`: either way round, it is asked again.
        PAGE_SIZE_ATTRIBUTE if host.has_attribute(grid::TREE_ATTRIBUTE) => retree(&host),
        grid::TREE_ATTRIBUTE | grid::TREE_KEY_ATTRIBUTE => retree(&host),
        grid::ADD_ROW_ATTRIBUTE => {
            render(&host, false);
            run_query(&host, QueryKind::Window, false);
        }
        grid::ROW_KEY_ATTRIBUTE => {
            if let Some(runtime) = runtime(&host) {
                // Keys of another field name nothing, and positions do not
                // turn into keys (issue #322): the selection is dropped, and
                // said. A tree's is by `tree-key` and stays.
                let (had_keys, had_rows) = {
                    let mut borrowed = runtime.borrow_mut();
                    borrowed.state.set_row_key(row_key_of(&host));
                    let had_keys = borrowed.picked.clear();
                    let had_rows =
                        borrowed.tree.is_none() && !borrowed.state.selection().is_empty();
                    if had_keys || had_rows {
                        borrowed.state.set_selection(Vec::new());
                        borrowed
                            .state
                            .set_notice(texts(&host).selection_cleared.clone());
                    }
                    (had_keys, had_rows)
                };
                render(&host, false);
                if had_keys {
                    dispatch_keys(&host, &[]);
                } else if had_rows {
                    dispatch_selection(&host, &[]);
                }
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
        // The page's filter (issue #330): other rows exist, as after a filter
        // of the reader's — the first window, no selection, the facets' values
        // listed anew. It is not the reader's, so the view has not changed.
        grid::FILTER_ATTRIBUTE => {
            let Some(runtime) = runtime(&host) else {
                return;
            };
            let pool = pool_of(&host);
            let selection_dropped;
            let keys_dropped;
            {
                let mut runtime = runtime.borrow_mut();
                runtime.page = 0;
                runtime.state.set_window(Window::new(0, pool));
                // A selection names positions, and other records sit in them now.
                selection_dropped = !runtime.state.invalidate_selection().is_empty();
                // One by key goes too (issue #322): the page's filter says
                // which records this grid has, as a replaced source does.
                keys_dropped = runtime.picked.clear();
                if keys_dropped {
                    runtime.state.note_selection_dropped();
                }
                runtime.facet_domain.clear();
                runtime.facet_counts.clear();
            }
            if keys_dropped {
                dispatch_keys(&host, &[]);
            } else if selection_dropped {
                announce_if_cleared(&host, &runtime);
            }
            run_query(&host, QueryKind::Data, false);
        }
        // `write_view` sets it on its way and owns the single query after.
        grid::GROUP_BY_ATTRIBUTE if applying_view() => {}
        grid::GROUP_BY_ATTRIBUTE => regroup(&host),
        // The selection column is part of the one-time skeleton, so switching
        // it means a new skeleton — the same as changing the columns.
        grid::SELECTION_ATTRIBUTE
        | grid::COLUMN_MENU_ATTRIBUTE
        | grid::FILTER_BUILDER_ATTRIBUTE
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
            // Another source: the types this grid learned belong to the old
            // one, and so do the records the page said something about.
            if name == DATASOURCE_ATTRIBUTE
                && let Some(runtime) = runtime(&host)
            {
                let mut runtime = runtime.borrow_mut();
                runtime.known.clear();
                runtime.state.take_saves();
                // The rows the reader added and what they did belong to it too,
                // and so do the values the facets list.
                let _ = runtime.state.take_edits();
                runtime.facet_domain.clear();
                runtime.facet_counts.clear();
            }
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
            filter_builder: host.has_attribute(grid::FILTER_BUILDER_ATTRIBUTE),
            facets: host.has_attribute(grid::FACETS_ATTRIBUTE),
            search: host.has_attribute(grid::SEARCH_ATTRIBUTE),
            actions: &actions_of(host),
            bands: &bands_of(host),
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
        borrowed.decorated = None;
    }
    // The reader's widths live on the header cells and survive the frames that
    // follow; a fresh skeleton has to get them back (point 36).
    apply_widths(host);
    // The filter row follows the columns' widths as the table changes size
    // (issue #62); the observer fires once at once, which lines it up.
    observe_columns(&root);
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
    show_source(host);
    let Some(runtime) = runtime(host) else {
        return;
    };
    sync_add_row(host, &runtime);

    let paging = paging_of(host, &runtime);
    // Past the sizer's limit the rows are lifted by the shift, which cannot be
    // more than the rows are taller than the sizer (the result may have shrunk).
    let paging = {
        let mut borrowed = runtime.borrow_mut();
        let shift = match paging.size {
            None => borrowed
                .shift
                .min(grid::squeezed_px(paging.rows, borrowed.row_height)),
            Some(_) => 0,
        };
        borrowed.shift = shift;
        paging.shifted(shift)
    };
    let (sorts, slots, pinned_slot) = {
        let borrowed = runtime.borrow();
        let Some(view) = borrowed.view.as_ref() else {
            return;
        };
        let pool = view.pool();
        let total = borrowed.state.row_count();
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
        let slots = grid::assign_pool(&borrowed.slots, focus, &rows, pool, total);
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
            &formatter(host, borrowed.state.schema()),
            paging,
            borrowed.grouping.as_ref(),
            borrowed.tree.as_ref(),
            keyed(borrowed).then_some(&borrowed.picked),
        );
        if let Some(dom) = borrowed.dom.as_mut() {
            dom.apply_buffer(&buffer);
        }
    }

    if let Some(root) = host.shadow_root() {
        let schema = runtime.borrow().state.schema().clone();
        let styles = presentation::styles(host);
        let decorated = Decorated::of(&schema, &styles);
        if runtime.borrow().decorated.as_ref() != Some(&decorated) {
            fix_operator_choices(&root, &schema, &styles);
            fix_presentation(&root, &schema, &styles);
            runtime.borrow_mut().decorated = Some(decorated);
        }
    }
    sync_chrome(host);
    sync_descriptions(host);
    // The lists of list columns are chips, drawn over the text the frame wrote.
    if let Some(root) = host.shadow_root() {
        combo::draw_cells(host, &root);
        template::draw_cells(host, &root);
    }

    range::paint(host);
    keep_pager_focus(host);
    if focus_after {
        focus_active(host);
    }
}

/// The paging buttons in the order they stand in the pager.
const PAGER_BUTTONS: [&str; 4] = ["page-first", "page-previous", "page-next", "page-last"];

/// Moves the focus off a paging button the render has just disabled.
///
/// Pressing "Next" on the way to the last page disables the button the focus
/// stands on, and a browser then drops the focus to the document (WCAG 2.4.3).
/// The focus goes to the nearest button that still works, on the side the
/// reader was heading away from: "next" and "last" hand it back to "previous",
/// "first" and "previous" to "next". With none left it goes to the active cell.
fn keep_pager_focus(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(focused) = root.active_element() else {
        return;
    };
    if !focused.has_attribute("disabled") {
        return;
    }
    let Some(position) = focused
        .get_attribute("part")
        .and_then(|part| PAGER_BUTTONS.iter().position(|name| *name == part))
    else {
        return;
    };
    let candidates: Vec<&str> = if position >= 2 {
        PAGER_BUTTONS[..position].iter().rev().copied().collect()
    } else {
        PAGER_BUTTONS[position + 1..].to_vec()
    };
    let target = candidates.into_iter().find_map(|name| {
        root.query_selector(&format!("[part=\"{name}\"]:not([disabled])"))
            .ok()
            .flatten()
            .and_then(|button| button.dyn_into::<HtmlElement>().ok())
    });
    match target {
        Some(button) => {
            let _ = button.focus();
        }
        None => focus_active(host),
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

/// What a display list's result is settled as (issue #342).
///
/// A group or a node that opens or closes is asked for as a window, to keep
/// the status line quiet, yet it puts other rows at the positions: that is a
/// new result. A scroll or a key leaves the list as long as it was, and then
/// the result is the same rows moved — the focused row keeps its slot, as in
/// an ungrouped grid.
fn settled_kind(asked: QueryKind, shown: u64, total: u64) -> QueryKind {
    if asked == QueryKind::Window && shown == total {
        QueryKind::Window
    } else {
        QueryKind::Data
    }
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
    // A view's selection is checked against the source first (issue #322).
    if grid_runtime.borrow().restore.is_some() {
        restore_then_query(host, &grid_runtime, kind, focus);
        return;
    }
    if grid_runtime.borrow().retype_filter {
        probe_then_query(host, &grid_runtime, kind, focus);
        return;
    }
    if grid_runtime.borrow().grouping.is_some() {
        run_grouped(host, &grid_runtime, kind, focus);
        return;
    }
    if grid_runtime.borrow().tree.is_some() {
        run_tree(host, &grid_runtime, kind, focus);
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
    let started = now();
    let promise = provider.execute(&query, &mode);
    let provider_kind = provider.kind();
    // One query for what is shown: a scroll or a new page replaces the rows.
    grid_runtime.borrow_mut().queries = 1;
    if kind == QueryKind::Data {
        refresh_facets(host);
    }

    let host = host.clone();
    spawn_local(async move {
        let answered = JsFuture::from(promise).await;
        let outcome = measured(&host, provider_kind, started, answered);
        settle(&host, generation, outcome, focus, kind);
    });
}

/// Remembers the types a result brought (plan point 88).
fn learn_types(runtime: &mut GridRuntime, schema: &opengrid_types::Schema) {
    for field in schema.fields() {
        runtime
            .known
            .insert(field.name.as_str().to_owned(), field.clone());
    }
}

/// Asks for the schema, then types the filter row against it, then runs the
/// data query (plan point 88).
///
/// Only when a view put a filter on a column whose type no result has told yet
/// — a view applied before the first result, which is what `connect` does on
/// mount. The probe is the query with `limit 0` and no filter: it cannot fail
/// on a literal, and it answers nothing but the schema. It is not drawn and not
/// announced; the status line is already "loading".
fn probe_then_query(
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
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let (generation, sorts) = {
        let mut runtime = grid_runtime.borrow_mut();
        // `retype_filter` stays set until the filter is typed: a query that
        // comes in meanwhile — a density, a key, an update from a page — must
        // not send the text literal; it probes again instead.
        runtime.generation += 1;
        runtime.state.set_status(GridStatus::Loading);
        (runtime.generation, runtime.state.sort_keys())
    };
    render(host, false);
    // With the grid's sort: an `offset` needs a total order (S6), even at 0.
    let probe = grid::query_json(&source, &columns, &sorts, None, 0, 0);
    let host = host.clone();
    let grid_runtime = grid_runtime.clone();
    spawn_local(async move {
        let schema = match ask(&host, &provider, &grid_runtime, &probe, &mode).await {
            Ok(result) => result.schema,
            Err(message) => return settle(&host, generation, Err(message), focus, kind),
        };
        // The types are the source's, whichever query asked for them.
        learn_types(&mut grid_runtime.borrow_mut(), &schema);
        if superseded(&grid_runtime, generation, focus) {
            return;
        }
        let Some(root) = host.shadow_root() else {
            return;
        };
        let entries = read_filter_entries(&root, &columns_of(&host));
        let typed = grid::filter_expr(&entries, &schema);
        {
            let mut runtime = grid_runtime.borrow_mut();
            runtime.retype_filter = false;
            runtime.state.adopt_schema(schema.clone());
            match typed {
                Ok(filter) => runtime.state.set_filter(filter),
                Err(problems) => {
                    // Said, not dropped: a view that looks applied and is not
                    // is the worse failure (the rule of set_view).
                    let texts = texts(&host);
                    let message = problems
                        .iter()
                        .map(|problem| texts.filter_invalid(&problem.column, &problem.value))
                        .collect::<Vec<_>>()
                        .join(" ");
                    runtime.state.set_status(GridStatus::Error(message));
                    drop(runtime);
                    render(&host, false);
                    return;
                }
            };
        }
        run_query(&host, kind, focus);
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
    kind: QueryKind,
) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let mut paste_dropped = false;
    // Rows of the selection that the result no longer has (issue #226).
    let mut shrunk = None;
    let mut unkeyed = false;
    {
        let mut runtime = runtime.borrow_mut();
        // A newer scroll, key or filter superseded this query; its focus request
        // is not lost with it.
        if runtime.generation != generation {
            runtime.pending_focus |= focus;
            return;
        }
        // Another result for the whole query: the rows a shown paste would
        // land on are other rows now (issue #228).
        if kind == QueryKind::Data {
            runtime.epoch += 1;
            // The positions a range marked are other rows now.
            runtime.range = None;
            paste_dropped = runtime.paste.take().is_some();
        }
        match outcome {
            Ok(result) => {
                learn_types(&mut runtime, &result.schema);
                // The selection by key ends where the grid stops selecting by
                // key — without its `row-key`, or as a tree (issue #322):
                // said with this result, like every dropped selection.
                if !keyed(&runtime) {
                    if runtime.picked.clear() {
                        unkeyed = true;
                        runtime.state.note_selection_dropped();
                    }
                } else if !runtime.picked.is_empty() {
                    // A sort or a filter dropped the positions the records
                    // sat at, not the selection: that is not said.
                    runtime.state.set_selection(Vec::new());
                }
                // A window is the same result moved, so what the reader is in
                // the middle of — an editor, unsaved marks — stays (issue #224).
                let patches = if kind == QueryKind::Window {
                    runtime.state.apply_window_result(result)
                } else {
                    runtime.state.apply_result(result)
                };
                if runtime.tree.is_none()
                    && !keyed(&runtime)
                    && patches
                        .iter()
                        .any(|patch| matches!(patch, GridPatch::Selection(_)))
                {
                    shrunk = Some(runtime.state.selection().to_vec());
                }
                show_picked(&mut runtime);
                if kind == QueryKind::Data
                    && let Some(said) = runtime.restore_said.take()
                {
                    runtime.state.append_notice(said);
                }
                if let Some(key) = runtime.replaced.take() {
                    follow_replaced(&mut runtime, key);
                } else if kind == QueryKind::Data {
                    // Other rows sit in the positions now. The renderer keeps
                    // the focused row's slot as it was, which for a scroll is
                    // right and for a new result shows the old record — or
                    // one past the end (issue #223).
                    follow_replaced(&mut runtime, None);
                }
            }
            Err(cause) => {
                runtime.replaced = None;
                runtime
                    .state
                    .set_status(GridStatus::Error(texts(host).error(&cause)));
            }
        }
    }
    if paste_dropped {
        if let Some(root) = host.shadow_root() {
            paste::close_preview(&root);
        }
        runtime
            .borrow_mut()
            .state
            .set_notice(texts(host).paste_discarded.clone());
    }
    let focus = {
        let mut runtime = runtime.borrow_mut();
        let asked = focus || paste_dropped || std::mem::take(&mut runtime.pending_focus);
        asked && may_take_focus(host)
    };
    // The first result brings the real schema, and a `set_columns` made before
    // the provider was attached has only met the display schema so far — where
    // every column is `Utf8` and `sum` on a number would read as a sum on text
    // (point 60). Re-checking here is cheap and terminates: it only rebuilds
    // when the checked presentation actually changed.
    recolumn(host);
    render(host, focus);
    if let Some(rows) = shrunk {
        dispatch_selection(host, &rows);
    }
    if unkeyed {
        dispatch_keys(host, &[]);
    }
}

/// Whether the grid selects by key right now (issue #322): its records have a
/// `row-key` among the columns — grouped or not. A tree selects by key too,
/// but by its own (`tree-key`), and keeps that selection itself.
fn keyed(runtime: &GridRuntime) -> bool {
    runtime.tree.is_none() && runtime.state.row_key_type().is_some()
}

/// The record at row `row`: its key as an id and where it sits — grouped, its
/// innermost group and its place in it; ungrouped, the row itself. `None` on a
/// group header, on the total, without a `row-key` and for a row that is not
/// loaded.
fn record_at(runtime: &GridRuntime, row: u64) -> Option<(String, Vec<opengrid_json::Json>, u64)> {
    let (keys, offset) = match runtime.grouping.as_ref() {
        Some(grouping) => match grouping.item_at(row)? {
            grouping::Item::Row { keys, offset, .. } => (keys, offset),
            _ => return None,
        },
        None => (Vec::new(), row),
    };
    let key = runtime.state.row_key(row)?;
    Some((grouping::key_id(key), keys, offset))
}

/// Shows the selection by key at the positions its records sit at in the
/// loaded window — the tree does the same with its nodes. A sort or a filter
/// has dropped those positions and not the selection, so it is not said.
fn show_picked(runtime: &mut GridRuntime) {
    if !keyed(runtime) || (runtime.picked.is_empty() && runtime.state.selection().is_empty()) {
        return;
    }
    let offset = runtime.state.window().offset;
    let loaded = runtime.slots.len().max(runtime.state.loaded_rows()) as u64;
    let rows = (offset..offset + loaded)
        .filter(|row| record_at(runtime, *row).is_some_and(|(id, ..)| runtime.picked.contains(&id)))
        .collect();
    runtime.state.set_selection(rows);
}

/// Draws a changed selection by key and tells the page, with the keys.
fn settle_picked(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let keys = {
        let mut borrowed = runtime.borrow_mut();
        show_picked(&mut borrowed);
        borrowed.picked.keys()
    };
    render(host, false);
    dispatch_keys(host, &keys);
}

/// The ids of the records in these slices — `(keys, offset, limit)`: rows of
/// the group with these keys, as a window's [`grouping::Fetch`], or with no
/// keys rows of the whole result — asked of the source in the grid's order.
/// `all` is every row of the filter: any order will do, as long as it is
/// total. `None` when the source fails (said in the status line) or when
/// another result has come meanwhile: the slices were cut from the old one.
async fn ids_of(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    slices: Vec<(Vec<opengrid_json::Json>, u64, u64)>,
    all: bool,
) -> Option<Vec<String>> {
    let provider = provider(host)?;
    let source = host.get_attribute(DATASOURCE_ATTRIBUTE)?;
    let key_name = row_key_of(host)?;
    let columns = columns_of(host);
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let (mut sorts, filter, epoch) = {
        let borrowed = runtime.borrow();
        (
            borrowed.state.sort_keys(),
            effective_filter(host, &borrowed, None).ok()?,
            borrowed.epoch,
        )
    };
    // The same total order the rows of a group are paged under (S6).
    if sorts.is_empty() {
        sorts.push((columns.first()?.clone(), "asc"));
    }
    let mut ids = Vec::new();
    for (keys, offset, limit) in slices {
        let scoped = match runtime.borrow().grouping.as_ref() {
            Some(grouping) => grouping.filter_for(&keys, filter.as_ref()),
            None => filter.clone(),
        };
        // Every row of the filter comes in pieces, and pieces need a total
        // order: the key ends every tie. A range keeps the order its rows
        // are shown in.
        let mut sorts = sorts.clone();
        if all && !sorts.iter().any(|(field, _)| field == key_name.as_str()) {
            sorts.push((key_name.as_str().to_owned(), "asc"));
        }
        let mut at = offset;
        while at < offset + limit {
            // The largest `limit` a query may ask.
            let piece = (offset + limit - at).min(crate::tree::MAX_LEVEL);
            let query = grid::query_json(&source, &columns, &sorts, scoped.as_ref(), at, piece);
            let result = match ask_uncounted(host, &provider, &query, &mode).await {
                Ok(result) => result,
                Err(message) => {
                    announce(host, &texts(host).error(&message));
                    return None;
                }
            };
            if runtime.borrow().epoch != epoch {
                return None;
            }
            let col = result
                .schema
                .fields()
                .iter()
                .position(|field| field.name == key_name)?;
            ids.extend(result.columns.get(col)?.iter().map(grouping::key_id));
            at += piece;
        }
    }
    Some(ids)
}

/// `Space` on a data row of a grid that selects by key (issue #322): selects
/// or deselects its record. With `extend` — `Shift` — every data row from the
/// anchor to this one is selected; grouped, those of the display list: group
/// headers in between are passed over, and a closed group has no rows there.
/// Without an anchor in the list — nothing toggled yet, its group closed, or
/// another record in its place since — it is a plain toggle.
fn pick_row(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, row: u64, extend: bool) {
    let (target, range) = {
        let borrowed = runtime.borrow();
        let Some(target) = record_at(&borrowed, row) else {
            return;
        };
        let anchor = borrowed.picked.anchor().filter(|_| extend);
        let range = anchor.and_then(|(id, keys, offset)| {
            let from = match borrowed.grouping.as_ref() {
                Some(grouping) => grouping.row_position(keys, *offset)?,
                None => *offset,
            };
            Some((id.clone(), from))
        });
        (target, range)
    };
    let Some((anchor, from)) = range else {
        let (id, keys, offset) = target;
        runtime.borrow_mut().picked.toggle(id, keys, offset);
        settle_picked(host, runtime);
        return;
    };
    let (low, high) = (from.min(row), from.max(row));
    let slices = match runtime.borrow().grouping.as_ref() {
        Some(grouping) => grouping
            .fetches(low, high - low + 1)
            .into_iter()
            .map(|fetch| (fetch.keys, fetch.offset, fetch.limit))
            .collect(),
        None => vec![(Vec::new(), low, high - low + 1)],
    };
    let (host, runtime) = (host.clone(), runtime.clone());
    spawn_local(async move {
        let Some(ids) = ids_of(&host, &runtime, slices, false).await else {
            return;
        };
        // The anchor's row holds another record now (a sort, a filter, a
        // change of the data): there is no range to speak of.
        let at_anchor = if from <= row { ids.first() } else { ids.last() };
        let changed = if at_anchor == Some(&anchor) {
            runtime.borrow_mut().picked.add(ids)
        } else {
            let (id, keys, offset) = target;
            runtime.borrow_mut().picked.toggle(id, keys, offset);
            true
        };
        if changed {
            settle_picked(&host, &runtime);
        }
    });
}

/// Selects **every row of the filter** by key — grouped, whether its group is
/// open or not (issue #322): their keys are asked of the source. `toggle`
/// is the selection header: with all of them selected already, it clears the
/// selection instead, and it says the count either way.
fn pick_all(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, toggle: bool) {
    let rows = {
        let borrowed = runtime.borrow();
        if !keyed(&borrowed) {
            return;
        }
        match borrowed.grouping.as_ref() {
            Some(grouping) => grouping.row_count(),
            None => borrowed.state.total_count(),
        }
    };
    let (host, runtime) = (host.clone(), runtime.clone());
    spawn_local(async move {
        let Some(ids) = ids_of(&host, &runtime, vec![(Vec::new(), 0, rows)], true).await else {
            return;
        };
        let changed = {
            let mut borrowed = runtime.borrow_mut();
            let all = !ids.is_empty() && ids.iter().all(|id| borrowed.picked.contains(id));
            if toggle && all {
                borrowed.picked.clear()
            } else {
                borrowed.picked.add(ids)
            }
        };
        if changed {
            settle_picked(&host, &runtime);
        }
        if toggle {
            let count = runtime.borrow().picked.len() as u64;
            announce(&host, &texts(&host).selected_all(count));
        }
    });
}

/// Whether a newer query has replaced this one; a focus request it carried is
/// kept for the answer that lands (issue #223).
fn superseded(runtime: &Rc<RefCell<GridRuntime>>, generation: u64, focus: bool) -> bool {
    let Ok(mut runtime) = runtime.try_borrow_mut() else {
        return false;
    };
    let stale = runtime.generation != generation;
    if stale && focus {
        runtime.pending_focus = true;
    }
    stale
}

/// Whether an answer may move the focus: from the table itself — a header or a
/// cell — yes; not when the reader has gone to a filter field or a toolbar
/// button while it was on its way (issue #223).
fn may_take_focus(host: &HtmlElement) -> bool {
    let Some(root) = host.shadow_root() else {
        return true;
    };
    root.active_element()
        .is_none_or(|focused| focused.closest("td, th").ok().flatten().is_some())
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
    if let (ActiveCell::Header { .. } | ActiveCell::Band { .. }, Some(viewport)) = (to, viewport) {
        viewport.set_scroll_top(0);
    }
    let Ok(Some(target)) = root.query_selector(&selector(to)) else {
        return;
    };
    let Ok(target) = target.dyn_into::<HtmlElement>() else {
        return;
    };
    // Where the viewport stands sideways before anything scrolls. A cell that
    // has the focus already took it from a click, and the browser has scrolled
    // for it: then the last scroll event knows.
    let sideways = viewport.map(|viewport| {
        if root.active_element().as_ref() == Some(target.as_ref()) {
            scrolled_sideways(root).unwrap_or_else(|| viewport.scroll_left())
        } else {
            viewport.scroll_left()
        }
    });
    // A pinned cell is in sight wherever the grid is scrolled to, and WebKit
    // scrolls to the place it has in the row — in the next frame, after anything
    // this function can put right. So the focus does not scroll at all; the
    // scroll below, which is made at once, takes care of the row (#379).
    if is_pinned(&target) {
        let focus = web_sys::FocusOptions::new();
        focus.set_prevent_scroll(true);
        let _ = target.focus_with_options(&focus);
    } else {
        let _ = target.focus();
    }
    let options = ScrollIntoViewOptions::new();
    // `nearest` scrolls the minimum. For a cell that unfolded past the height of
    // the viewport (point 47) that still aligns its **top**, and the cells'
    // `scroll-margin-top` keeps that top clear of the sticky header — so the
    // value is read from its first line and the rest is scrolled to. Measured,
    // not assumed: without the scroll margin the first lines end up above the
    // header, with it they start right below it.
    options.set_block(ScrollLogicalPosition::Nearest);
    target.scroll_into_view_with_scroll_into_view_options(&options);
    // A pinned cell is in sight wherever the grid is scrolled to, and a
    // browser scrolls to the place it has in the row all the same (issue
    // #316): the grid would jump sideways under the reader.
    if let (Some(viewport), Some(sideways)) = (viewport, sideways)
        && is_pinned(&target)
    {
        viewport.set_scroll_left(sideways);
    }
}

/// Whether an element is kept in place by the pinned columns' sheet.
fn is_pinned(element: &Element) -> bool {
    web_sys::window()
        .and_then(|window| window.get_computed_style(element).ok().flatten())
        .and_then(|style| style.get_property_value("position").ok())
        .is_some_and(|position| position == "sticky")
}

/// How far the grid of `root` was scrolled sideways at its last scroll event.
fn scrolled_sideways(root: &ShadowRoot) -> Option<i32> {
    let host = root.host().dyn_into::<HtmlElement>().ok()?;
    Some(runtime(&host)?.borrow().scroll_left)
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
        // A band's cell lists the columns it spans, and is found by any.
        ActiveCell::Band { col } => format!("th[{}~=\"{col}\"]", grid::BAND_ATTRIBUTE),
        ActiveCell::Data(cell) => {
            format!("td[data-row=\"{}\"][data-col=\"{}\"]", cell.row, cell.col)
        }
    }
}

/// Reads the active cell back from a DOM cell (click or `focusin`).
fn active_from_element(element: &Element) -> Option<ActiveCell> {
    // The button inside an action cell stands for the cell (issue #189).
    if let Ok(Some(cell)) = element.closest("td[part=\"action-cell\"]")
        && !cell.is_same_node(Some(element))
    {
        return active_from_element(&cell);
    }
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
    // A band's cell (issue #319): the first of the columns it spans.
    if let Some(spanned) = element.get_attribute(grid::BAND_ATTRIBUTE) {
        let col = spanned.split(' ').next()?.parse().ok()?;
        return Some(ActiveCell::Band { col });
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
    // WebKit fires `focus` and no `focusin` when script gives a date or time
    // field the focus (#379). `focus` does not bubble, so it is heard on the way
    // down, and only for those fields: every other control fires both, and
    // would be handled twice.
    let focus = Closure::<dyn FnMut(Event)>::new(on_focus_without_focusin).into_js_value();
    let _ = root.add_event_listener_with_callback_and_bool("focus", focus.unchecked_ref(), true);
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
    // What the page's validators say while the reader types (issue #188).
    let validating = Closure::<dyn FnMut(Event)>::new(validate::on_editor_event).into_js_value();
    let _ = root.add_event_listener_with_callback("input", validating.unchecked_ref());
    let _ = root.add_event_listener_with_callback("change", validating.unchecked_ref());
    let typing = Closure::<dyn FnMut(Event)>::new(combo::on_input).into_js_value();
    let _ = root.add_event_listener_with_callback("input", typing.unchecked_ref());
    let copied = Closure::<dyn FnMut(Event)>::new(range::on_copy).into_js_value();
    let _ = root.add_event_listener_with_callback("copy", copied.unchecked_ref());
    for (kind, handler) in [
        ("mousedown", range::on_mouse_down as fn(Event)),
        ("mouseover", range::on_mouse_over as fn(Event)),
        ("mouseup", range::on_mouse_up as fn(Event)),
    ] {
        let listener = Closure::<dyn FnMut(Event)>::new(handler).into_js_value();
        let _ = root.add_event_listener_with_callback(kind, listener.unchecked_ref());
    }
    let focusout = Closure::<dyn FnMut(Event)>::new(template::on_focus_out).into_js_value();
    let _ = root.add_event_listener_with_callback("focusout", focusout.unchecked_ref());
    let pasted = Closure::<dyn FnMut(Event)>::new(paste::on_paste).into_js_value();
    let _ = root.add_event_listener_with_callback("paste", pasted.unchecked_ref());
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
    // The column list (issue #101): `Escape` closes it and the focus goes back
    // to its button, which a light-dismissed popover would not do by itself.
    if event.key() == "Escape"
        && event
            .target()
            .and_then(|node| node.dyn_into::<Element>().ok())
            .and_then(|target| target.closest("[part=\"columns\"]").ok().flatten())
            .is_some()
    {
        event.prevent_default();
        close_columns(&root, true);
        return;
    }
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
        // The operator button is a menu button (issue #96): the arrows open
        // its menu, Enter and Space are its native click.
        if target.get_attribute("part").as_deref() == Some("filter-operator")
            && matches!(event.key().as_str(), "ArrowDown" | "ArrowUp")
            && let Some(col) = target
                .get_attribute("data-col")
                .and_then(|col| col.parse::<usize>().ok())
            && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        {
            event.prevent_default();
            open_operator_menu(&host, col, event.key() == "ArrowUp");
            return;
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
        // The search field sits in the toolbar's row (issue #33), and its keys
        // are the combobox's below.
        && target.closest("[part=\"search\"]").ok().flatten().is_none()
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
    // The quick doors of issue #34: the dialog's and the menu's own keys.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
    {
        if target
            .closest("[part=\"filter-dialog\"]")
            .ok()
            .flatten()
            .is_some()
        {
            on_dialog_key(&host, &event, &target);
            return;
        }
        if target
            .closest("[part=\"grouping-menu\"]")
            .ok()
            .flatten()
            .is_some()
        {
            on_group_menu_key(&host, &event, &target);
            return;
        }
        if target
            .closest("[part=\"operator-menu\"]")
            .ok()
            .flatten()
            .is_some()
        {
            on_operator_menu_key(&host, &event, &target);
            return;
        }
    }

    // The combobox of an editor (issue #185) takes its own keys first: the arrows,
    // `Backspace` on its chips, `Enter` on an option.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && combo::on_key(&host, &event, &target)
    {
        return;
    }
    // The record form (issue #325) is a modal dialog: its keys are the
    // platform's — `Enter` submits, `Escape` closes — and the grid's matrix
    // stays out of it.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && record::in_dialog(&target)
    {
        return;
    }
    // A control inside a cell template (issue #317) keeps its own keys, bar
    // `Escape`, which comes back to the cell.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && template::on_key(&host, &event, &target)
    {
        return;
    }
    // The bulk panel (issue #187) answers `Enter` and `Escape`.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && bulk::in_panel(&target)
    {
        bulk::on_key(&host, &event, &target);
        return;
    }
    // The preview of a paste (issue #184) answers `Enter` and `Escape`.
    if let Some(target) = event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        && paste::in_preview(&target)
    {
        paste::on_preview_key(&host, &event, &target);
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
                let cell = runtime.borrow().state.editing();
                end_edit(&host, true);
                if let Some(cell) = cell {
                    after_new_row_enter(&host, cell, event.ctrl_key() || event.meta_key());
                }
            }
            "Escape" => {
                event.prevent_default();
                end_edit(&host, false);
            }
            _ => {}
        }
        return;
    }

    // Everything below is the grid's matrix, and it acts on the *remembered*
    // active cell — so it may only run for a key pressed inside the table. The
    // viewport is focusable by mouse (`tabindex="-1"`, so that Firefox does not
    // make it a tab stop): a click on the blank space under the rows, or on the
    // empty state's text, focuses it, and `Space` there selected a row, `Enter`
    // sorted. A key on the viewport itself does what the browser does: scroll.
    if !event
        .target()
        .and_then(|node| node.dyn_into::<Element>().ok())
        .is_some_and(|target| target.closest("table").ok().flatten().is_some())
    {
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
    // `←` and `→` by what the reader sees (issue #320): in a right-to-left
    // layout the next cell is on the left, so the two change places — for
    // moving, for a column's place and width, and for a group or a node.
    // `Home` and `End` name the row's ends and stay as they are.
    let pressed = event.key();
    let sideways = matches!(pressed.as_str(), "ArrowLeft" | "ArrowRight");
    let key = match pressed.as_str() {
        "ArrowLeft" if crate::element::is_rtl(&host) => "ArrowRight",
        "ArrowRight" if crate::element::is_rtl(&host) => "ArrowLeft",
        other => other,
    };
    if (event.ctrl_key() || event.meta_key())
        && sideways
        && let ActiveCell::Header { col } = active
    {
        event.prevent_default();
        let by = if key == "ArrowLeft" { -1 } else { 1 };
        if event.shift_key() {
            resize_column(&host, col, by * WIDTH_STEP as i32);
        } else {
            move_column(&host, col, by);
        }
        return;
    }
    // A band's cell (issue #319) moves the band as a whole. A band has no
    // width of its own — its columns have — so the resize key does nothing.
    if (event.ctrl_key() || event.meta_key())
        && sideways
        && let ActiveCell::Band { col } = active
    {
        event.prevent_default();
        if !event.shift_key() {
            move_band(&host, col, if key == "ArrowLeft" { -1 } else { 1 });
        }
        return;
    }
    // Pinning lives on the header cell as well (issue #316): one press takes
    // the column one step towards that side — from the other end to not
    // pinned, from not pinned to this end. On a band's cell, and on the header
    // of a column in a band, it is the band that is pinned.
    if event.alt_key()
        && event.shift_key()
        && sideways
        && let ActiveCell::Header { col } | ActiveCell::Band { col } = active
    {
        event.prevent_default();
        step_pin(&host, col, key == "ArrowRight");
        return;
    }

    // A group header (point 62): `Enter` and `Space` open and close it, and on
    // its first cell `→` opens and `←` closes — the treegrid keys (F2). `→` on
    // an open group and `←` on a closed one fall through to moving, so nothing
    // is a dead end.
    if let Some((position, expanded)) = active_group(&host) {
        let on_first = matches!(active, ActiveCell::Data(cell) if cell.col == 0);
        let toggles = match key {
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
    // A tree's node (issue #135), the WAI-ARIA treegrid keys on its first
    // cell: `→` opens a closed node and goes to the first child of an open
    // one; `←` closes an open node and goes to the parent of a closed one or a
    // leaf — so the reader climbs without counting rows. `Enter` and `Space`
    // keep their grid meaning; `Space` selects the node.
    if let Some((position, entry)) = active_tree(&host)
        && !event.ctrl_key()
        && !event.meta_key()
    {
        let on_first = matches!(active, ActiveCell::Data(cell) if cell.col == 0);
        let opens = entry.children > 0;
        match key {
            "ArrowRight" if on_first && opens && !entry.expanded => {
                event.prevent_default();
                toggle_tree(&host, position);
                return;
            }
            "ArrowRight" if on_first && entry.expanded => {
                event.prevent_default();
                let child = runtime
                    .borrow()
                    .tree
                    .as_ref()
                    .and_then(|tree| tree.first_child_of(position));
                // Still loading: the key waits for the children, not moves on.
                if let Some(child) = child {
                    move_to_row(&host, &runtime, child);
                }
                return;
            }
            "ArrowLeft" if on_first && entry.expanded => {
                event.prevent_default();
                toggle_tree(&host, position);
                return;
            }
            "ArrowLeft" if on_first => {
                let parent = runtime
                    .borrow()
                    .tree
                    .as_ref()
                    .and_then(|tree| tree.parent_of(position));
                if let Some(parent) = parent {
                    event.prevent_default();
                    move_to_row(&host, &runtime, parent);
                    return;
                }
            }
            _ => {}
        }
    }
    // Undo and redo of what the reader did (issue #186): `Ctrl`/`Cmd`+`Z`, and
    // forward with `Shift` or `Ctrl`+`Y`. An open editor has its own, above.
    if (event.ctrl_key() || event.meta_key()) && !event.alt_key() {
        match event.key().to_ascii_lowercase().as_str() {
            "z" => {
                event.prevent_default();
                if event.shift_key() {
                    history::redo(&host);
                } else {
                    history::undo(&host);
                }
                return;
            }
            "y" if event.ctrl_key() => {
                event.prevent_default();
                history::redo(&host);
                return;
            }
            _ => {}
        }
    }
    // An action cell (issue #189): `Enter` and `Space` press its button.
    if let ActiveCell::Data(cell) = active
        && let Some(index) = cell.col.checked_sub(runtime.borrow().state.schema().len())
        && (event.key() == "Enter" || event.key() == " ")
        && !event.ctrl_key()
        && !event.meta_key()
    {
        event.prevent_default();
        fire_action(&host, cell.row, index);
        return;
    }
    // Grouped, a position is a display position: a selection would name
    // headers as well as rows and move on every toggle. So a grouped grid
    // selects and edits by `row-key` (issues #322 and #190), on its data rows
    // only — and without one it does neither.
    if is_grouped(&host) {
        let borrowed = runtime.borrow();
        let keyed = keyed(&borrowed);
        // The selection header, or a data row — its cells or its selection
        // cell.
        let picks = keyed
            && match active {
                ActiveCell::SelectAll => true,
                ActiveCell::Header { .. } | ActiveCell::Band { .. } => false,
                _ => active
                    .row()
                    .is_some_and(|row| record_at(&borrowed, row).is_some()),
            };
        drop(borrowed);
        let refused = match event.key().as_str() {
            " " => !picks,
            "Enter" if matches!(active, ActiveCell::Data(_)) => !editable_group_row(&host, active),
            "Enter" => !picks,
            key => {
                key.eq_ignore_ascii_case("a") && (event.ctrl_key() || event.meta_key()) && !keyed
            }
        };
        if refused {
            event.prevent_default();
            return;
        }
    }
    // In a tree a node is selected by its key, but an edit would still be
    // reported against a display position: `Enter` on a leaf opens no editor.
    if is_tree(&host) && event.key() == "Enter" && matches!(active, ActiveCell::Data(_)) {
        event.prevent_default();
        return;
    }
    // `Shift`+arrows mark a block of cells; any other key that moves or edits
    // ends it (issue #327).
    if range::on_key(&host, &runtime, &event, active) {
        return;
    }
    // `Shift`+`F2` opens the record as a form (issue #325).
    if event.key() == "F2"
        && event.shift_key()
        && let ActiveCell::Data(cell) = active
        && record::applicable(&host, &runtime.borrow(), cell)
    {
        event.prevent_default();
        record::open(&host, cell);
        return;
    }
    // `F2` with rows selected sets one value for all of them (issue #187).
    if event.key() == "F2"
        && let ActiveCell::Data(cell) = active
        && bulk::applicable(&host, &runtime.borrow())
    {
        event.prevent_default();
        bulk::open(&host, cell.col);
        return;
    }
    // `F2` and a typed character open no editor there either; they are left
    // to the browser, as on any cell that edits nothing.
    let typed = grid::typed_character(&event.key(), event.ctrl_key(), event.meta_key())
        .filter(|_| !event.is_composing())
        .map(str::to_owned);
    let opens_editor = event.key() == "F2" || typed.is_some();
    if opens_editor && (is_tree(&host) || (is_grouped(&host) && !editable_group_row(&host, active)))
    {
        return;
    }

    // `Ctrl`/`Cmd`+`A` selects every matching row, not only the loaded page —
    // in a tree, every node shown.
    if event.key().eq_ignore_ascii_case("a") && (event.ctrl_key() || event.meta_key()) {
        event.prevent_default();
        if is_tree(&host) {
            select_in_tree(&host, &runtime, Tree::select_shown);
            return;
        }
        if is_grouped(&host) || keyed(&runtime.borrow()) {
            pick_all(&host, &runtime, false);
            return;
        }
        let patches = runtime.borrow_mut().state.select_all();
        settle_selection(&host, &runtime, patches);
        return;
    }

    // `Ctrl`/`Cmd`+`Enter` anywhere in the new row adds the record (issue #183).
    if event.key() == "Enter"
        && (event.ctrl_key() || event.meta_key())
        && let ActiveCell::Data(cell) = active
        && runtime.borrow().state.row_kind(cell.row) == RowKind::Draft
    {
        event.prevent_default();
        add_record(&host);
        return;
    }

    match (key, event.ctrl_key()) {
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
                // A band sorts nothing: its columns do, each on its own.
                ActiveCell::Band { .. } => {}
                ActiveCell::Data(cell) => {
                    if !template::enter(&host, cell) {
                        begin_edit(&host, cell, None);
                    }
                }
            }
        }
        // The other two ways into the editor of the WAI-ARIA grid pattern:
        // `F2`, as `Enter`, and a typed character, which also goes into it —
        // a marks sheet is filled by focusing a cell and typing the mark.
        // Only on a data cell; a header or the selection column takes neither.
        _ if opens_editor => {
            if let ActiveCell::Data(cell) = active {
                begin_edit(&host, cell, typed.as_deref());
                // Kept only if the editor opened: a read-only cell leaves the
                // key to the browser. Prevented, the typed character is not
                // typed a second time into the editor that now has the focus.
                if runtime.borrow().state.editing().is_some() {
                    event.prevent_default();
                }
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
                ActiveCell::Band { .. } => {}
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
    if is_tree(host) {
        select_in_tree(host, runtime, |tree| {
            if extend {
                tree.extend_selected(row)
            } else {
                tree.toggle_selected(row)
            }
        });
        return;
    }
    // By key wherever the records have one; grouped without one, not at all.
    if is_grouped(host) || keyed(&runtime.borrow()) {
        pick_row(host, runtime, row, extend);
        return;
    }
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
    if is_tree(host) {
        // Every node shown, or — when they all are — nothing at all.
        let all = runtime
            .borrow()
            .tree
            .as_ref()
            .is_some_and(Tree::all_shown_selected);
        if all {
            select_in_tree(host, runtime, Tree::clear_selected);
        } else {
            select_in_tree(host, runtime, Tree::select_shown);
        }
        let count = runtime.borrow().state.selection().len() as u64;
        announce(host, &texts(host).selected_all(count));
        return;
    }
    if is_grouped(host) || keyed(&runtime.borrow()) {
        pick_all(host, runtime, true);
        return;
    }
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
    let (active, ncols, total_count, window, pool, bands) = {
        let runtime = runtime.borrow();
        let pool = runtime
            .view
            .as_ref()
            .map(|view| view.pool() as u64)
            .unwrap_or_else(|| pool_of(host));
        // The bands the header shows (issue #319); none is the one-row header.
        let bands: Vec<(usize, usize)> = runtime.view.as_ref().map_or_else(Vec::new, |view| {
            view.bands
                .iter()
                .map(|band| (band.first, band.count))
                .collect()
        });
        (
            runtime.active,
            // The action columns are places on the row like any other.
            runtime.state.schema().len() + actions_of(host).len(),
            runtime.state.row_count(),
            runtime.state.window(),
            pool,
            bands,
        )
    };
    let next = grid::move_banded(
        active,
        key,
        ncols,
        total_count,
        viewport_rows,
        shows_selection(host),
        &bands,
    );
    let reload = grid::requested_window(key, next, window, total_count, pool);

    runtime.borrow_mut().set_active(next);
    match reload {
        Some(new_offset) => {
            if let Some(cell) = next.data() {
                scroll_to_far_row(runtime, cell.row);
            }
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
    // A selection by key names records and stays: a tree's (issue #135) and
    // that of every grid with a `row-key` (issue #322).
    if runtime.borrow().tree.is_some() || keyed(&runtime.borrow()) {
        return;
    }
    if runtime.borrow().state.selection().is_empty() {
        dispatch_selection(host, &[]);
    }
}

/// Changes a tree's selection (issue #135): the tree keeps it by key, the
/// state shows it at the positions the keys sit at now, and the page hears it
/// with the keys.
fn select_in_tree(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    change: impl FnOnce(&mut Tree) -> bool,
) {
    let keys = {
        let mut borrowed = runtime.borrow_mut();
        let borrowed = &mut *borrowed;
        let Some(tree) = borrowed.tree.as_mut() else {
            return;
        };
        if !change(tree) {
            return;
        }
        borrowed.state.set_selection(tree.selected_positions());
        tree.selected_keys()
    };
    render(host, false);
    dispatch_keys(host, &keys);
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
        borrowed.set_active(ActiveCell::Data(CellRef::new(next * size, 0)));
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
        // The rows the reader added count (issue #183).
        None => grid::Paging::whole(runtime.borrow().state.row_count()),
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

/// Where `set_actions` stores what the page supplied.
pub(crate) const ACTIONS_KEY: &str = "__opengridActions";

/// The actions the page supplied (issue #189): `[{ id, label }]`, in order. An
/// entry without both is left out — an action nobody can name has no button.
fn actions_of(host: &HtmlElement) -> Vec<grid::Action> {
    let Ok(list) = js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(ACTIONS_KEY)) else {
        return Vec::new();
    };
    let Ok(list) = list.dyn_into::<js_sys::Array>() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let text = |name: &str| {
                js_sys::Reflect::get(&entry, &JsValue::from_str(name))
                    .ok()
                    .and_then(|value| value.as_string())
                    .filter(|value| !value.is_empty())
            };
            Some(grid::Action {
                id: text("id")?,
                label: text("label")?,
            })
        })
        .collect()
}

/// Stores the page's actions and draws the columns (issue #189).
pub(crate) fn set_actions(host: &HtmlElement, actions: &JsValue) {
    let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(ACTIONS_KEY), actions);
    let Some(runtime) = runtime(host) else {
        return;
    };
    if host.shadow_root().is_none() {
        return;
    }
    // Part of the one-time skeleton: new columns are a new skeleton, around the
    // same state.
    let had_focus = reskeleton(host, &runtime);
    run_query(host, QueryKind::Data, had_focus);
}

/// Whether row `row` is a record an action may name: not a group header and
/// not the total.
fn is_record(runtime: &GridRuntime, row: u64) -> bool {
    runtime
        .grouping
        .as_ref()
        .is_none_or(|grouping| matches!(grouping.item_at(row), Some(grouping::Item::Row { .. })))
}

/// Fires `opengrid-row-action` for the action at `index` on `row`.
fn fire_action(host: &HtmlElement, row: u64, index: usize) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(action) = actions_of(host).into_iter().nth(index) else {
        return;
    };
    let key = {
        let borrowed = runtime.borrow();
        if !is_record(&borrowed, row) {
            return;
        }
        row_key_js(host, &borrowed.state, row)
    };
    let detail = js_sys::Object::new();
    if let Some(key) = key {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("key"), &key);
    }
    for (name, value) in [
        ("id", JsValue::from_str(&action.id)),
        ("row", JsValue::from_f64(row as f64)),
    ] {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(name), &value);
    }
    crate::element::dispatch(host, ROW_ACTION_EVENT, &detail);
}

/// A click on an action button.
fn on_action_click(host: &HtmlElement, target: &Element) -> bool {
    let Ok(Some(cell)) = target.closest("td[part=\"action-cell\"]") else {
        return false;
    };
    let (Some(row), Some(col)) = (
        cell.get_attribute("data-row")
            .and_then(|row| row.parse::<u64>().ok()),
        cell.get_attribute("data-col")
            .and_then(|col| col.parse::<usize>().ok()),
    ) else {
        return true;
    };
    if let Some(runtime) = runtime(host) {
        let first = runtime.borrow().state.schema().len();
        if let Some(index) = col.checked_sub(first) {
            fire_action(host, row, index);
        }
    }
    true
}

/// `key` of `opengrid-cell-change`: the `row-key` field's value in the row,
/// `null` when the result does not carry the field — and `None`, no `key` at
/// all, without the attribute.
fn row_key_js(host: &HtmlElement, state: &GridState, row: u64) -> Option<JsValue> {
    has_row_key(host).then(|| {
        state.row_key(row).map_or(JsValue::NULL, |value| {
            to_js(&opengrid_json::ToJson::to_json(value))
        })
    })
}

/// A key the page gave, read as a value of the `row-key` field's type.
fn key_from_js(state: &GridState, key: &JsValue) -> Option<Value> {
    let data_type = state.row_key_type()?;
    if key.is_undefined() {
        return None;
    }
    let text = js_sys::JSON::stringify(key).ok().map(String::from)?;
    let json = opengrid_json::Json::parse(&text).ok()?;
    Value::from_json_typed(&json, &data_type).ok()
}

/// Makes what a cell says about itself a node it is described by.
///
/// A validation sentence and a tree's "context" were `aria-description`, which
/// is ARIA 1.3 draft with uneven support, and one attribute for two things: a
/// finding on a tree's first cell overwrote the context. They are hidden text
/// nodes now, in one hidden box in the shadow root, and a cell lists the ones
/// it has in `aria-describedby` — the established pair (ARIA 1.1). The frame
/// writes the cell's marks as patches; this runs after it, since a patch cannot
/// make a node of its own.
fn sync_descriptions(host: &HtmlElement) {
    let (Some(runtime), Some(root), Some(document)) =
        (runtime(host), host.shadow_root(), host.owner_document())
    else {
        return;
    };
    let borrowed = runtime.borrow();
    let tree_context = texts(host);
    let Some(store) = description_store(&root, &document) else {
        return;
    };
    let mut wanted: Vec<String> = Vec::new();
    // Every cell that carries a finding is described by it — and by the
    // context, when the frame put that on it.
    if let Ok(cells) = root.query_selector_all("td[data-validation]") {
        for index in 0..cells.length() {
            let Some(td) = cells
                .item(index)
                .and_then(|node| node.dyn_into::<HtmlElement>().ok())
            else {
                continue;
            };
            let (Some(row), Some(col)) = (
                td.get_attribute("data-row")
                    .and_then(|row| row.parse().ok()),
                td.get_attribute("data-col")
                    .and_then(|col| col.parse().ok()),
            ) else {
                continue;
            };
            let Some(finding) = borrowed.state.finding(CellRef::new(row, col)) else {
                continue;
            };
            let id = format!("{}{row}-{col}", grid::FINDING_ID_PREFIX);
            description_node(&document, &store, &id, &finding.message, None);
            let mut ids: Vec<String> = td
                .get_attribute("aria-describedby")
                .unwrap_or_default()
                .split_whitespace()
                .filter(|token| !token.starts_with(grid::FINDING_ID_PREFIX))
                .map(str::to_owned)
                .collect();
            ids.push(id.clone());
            let _ = td.set_attribute("aria-describedby", &ids.join(" "));
            wanted.push(id);
        }
    }
    // A cell whose finding is gone loses its token — a tree's context stays.
    if let Ok(cells) = root.query_selector_all(r#"td[aria-describedby]:not([data-validation])"#) {
        for index in 0..cells.length() {
            let Some(td) = cells
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let kept: Vec<String> = td
                .get_attribute("aria-describedby")
                .unwrap_or_default()
                .split_whitespace()
                .filter(|token| !token.starts_with(grid::FINDING_ID_PREFIX))
                .map(str::to_owned)
                .collect();
            if kept.is_empty() {
                let _ = td.remove_attribute("aria-describedby");
            } else {
                let _ = td.set_attribute("aria-describedby", &kept.join(" "));
            }
        }
    }
    // A further sort key's header is described by its place.
    if let Ok(headers) = root.query_selector_all("th[aria-describedby]") {
        for index in 0..headers.length() {
            let Some(th) = headers
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some(id) = th.get_attribute("aria-describedby") else {
                continue;
            };
            let Some(position) = id
                .strip_prefix(grid::SORT_KEY_ID_PREFIX)
                .and_then(|position| position.parse().ok())
            else {
                continue;
            };
            description_node(
                &document,
                &store,
                &id,
                &tree_context.sort_key(position),
                Some(&tree_context.lang),
            );
            wanted.push(id);
        }
    }
    let context = root
        .query_selector(&format!(
            r#"[aria-describedby~="{}"]"#,
            grid::TREE_CONTEXT_ID
        ))
        .ok()
        .flatten()
        .is_some();
    if context {
        description_node(
            &document,
            &store,
            grid::TREE_CONTEXT_ID,
            &tree_context.tree_context,
            Some(&tree_context.lang),
        );
        wanted.push(grid::TREE_CONTEXT_ID.to_owned());
    }
    // Nodes nothing refers to any more.
    let mut child = store.first_element_child();
    while let Some(node) = child {
        child = node.next_element_sibling();
        if !wanted.contains(&node.id()) {
            node.remove();
        }
    }
}

/// The hidden box the description nodes live in, made on first need.
fn description_store(root: &web_sys::ShadowRoot, document: &web_sys::Document) -> Option<Element> {
    if let Ok(Some(store)) = root.query_selector("[data-descriptions]") {
        return Some(store);
    }
    let store = document.create_element("div").ok()?;
    store.set_attribute("data-descriptions", "").ok()?;
    store.set_attribute("hidden", "").ok()?;
    root.append_child(&store).ok()?;
    Some(store)
}

/// One description node: made if it is not there, its words and language set.
fn description_node(
    document: &web_sys::Document,
    store: &Element,
    id: &str,
    text: &str,
    lang: Option<&str>,
) {
    let node = match store
        .query_selector(&format!(r#"[id="{id}"]"#))
        .ok()
        .flatten()
    {
        Some(node) => node,
        None => {
            let Ok(node) = document.create_element("span") else {
                return;
            };
            node.set_id(id);
            let _ = store.append_child(&node);
            node
        }
    };
    if node.text_content().as_deref() != Some(text) {
        node.set_text_content(Some(text));
    }
    match lang.filter(|lang| !lang.trim().is_empty()) {
        Some(lang) => {
            let _ = node.set_attribute("lang", lang);
        }
        None => {
            let _ = node.remove_attribute("lang");
        }
    }
}

/// Redraws one cell in place: the focused row's slot is pinned and the
/// renderer leaves it alone (point 17), so the page's word on a cell there
/// would otherwise wait for the focus to move.
fn redraw_cell(host: &HtmlElement, cell: CellRef) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) else {
        return;
    };
    let borrowed = runtime.borrow();
    if borrowed.state.editing() != Some(cell) {
        let text = borrowed
            .state
            .cell(cell)
            .map(|value| formatter(host, borrowed.state.schema()).text(cell.col, value))
            .unwrap_or_default();
        td.set_text_content(Some(&text));
        let _ = td.remove_attribute("data-validating");
        combo::draw_cell(host, &td, cell);
    }
    if borrowed.state.is_changed(cell) {
        let _ = td.set_attribute("data-changed", "true");
    } else {
        let _ = td.remove_attribute("data-changed");
    }
    let _ = td.set_attribute(
        "part",
        borrowed
            .state
            .cell_state(cell)
            .map_or("cell", |state| state.part()),
    );
    match borrowed.state.cell_state(cell) {
        Some(state) => {
            let _ = td.set_attribute("data-state", state.as_str());
        }
        None => {
            let _ = td.remove_attribute("data-state");
        }
    }
    let lacking = borrowed.state.row_kind(cell.row) == RowKind::Draft
        && borrowed.state.added().missing().contains(&cell.col);
    // What a validator or the page said about the value (issue #188).
    let finding = borrowed.state.finding(cell);
    match &finding {
        Some(finding) => {
            let _ = td.set_attribute("data-validation", finding.level.as_str());
        }
        None => {
            let _ = td.remove_attribute("data-validation");
            // A tree's context is named by the same attribute (issue #135).
            if !is_tree(host) {
                let _ = td.remove_attribute("aria-describedby");
            }
        }
    }
    let found_error = finding
        .as_ref()
        .is_some_and(|finding| finding.level == opengrid_grid::Level::Error);
    if matches!(
        borrowed.state.cell_state(cell),
        Some(opengrid_grid::CellState::Error | opengrid_grid::CellState::Invalid)
    ) || lacking
        || found_error
    {
        let _ = td.set_attribute("aria-invalid", "true");
    } else {
        let _ = td.remove_attribute("aria-invalid");
    }
    drop(borrowed);
    sync_descriptions(host);
}

/// [`crate::element::set_cell_state`] (issue #153).
pub(crate) fn write_cell_state(
    host: &HtmlElement,
    key: &JsValue,
    column: &str,
    state: opengrid_grid::CellState,
    message: Option<String>,
) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Ok(column) = opengrid_types::FieldName::new(column) else {
        return;
    };
    let Some(key) = key_from_js(&runtime.borrow().state, key) else {
        return;
    };
    let cell = {
        let mut borrowed = runtime.borrow_mut();
        // The sentence of an `invalid` cell stays with the record (issue #188).
        borrowed.state.set_cell_message(
            key.clone(),
            column.clone(),
            message
                .clone()
                .filter(|_| state == opengrid_grid::CellState::Invalid),
        );
        borrowed.state.set_cell_state(key, column, state)
    };
    // One live region (point 41): a failure is said in the status line,
    // politely, and the focus stays where the reader is.
    if matches!(
        state,
        opengrid_grid::CellState::Error | opengrid_grid::CellState::Invalid
    ) && let Some(message) = message
    {
        runtime.borrow_mut().state.set_notice(message);
    }
    render(host, false);
    if let Some(cell) = cell {
        redraw_cell(host, cell);
    }
}

/// [`crate::element::set_cell_templates`] (issue #317).
pub(crate) fn set_cell_templates(host: &HtmlElement, templates: &JsValue) {
    template::store(host, templates);
}

/// [`crate::element::set_validators`] (issue #188).
pub(crate) fn set_validators(host: &HtmlElement, validators: &JsValue) {
    validate::store(host, validators);
}

/// [`crate::element::set_values`] (issue #153).
pub(crate) fn write_values(host: &HtmlElement, values: &JsValue) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let Some(list) = values.dyn_ref::<js_sys::Array>() else {
        return;
    };
    let entries: Vec<(Value, opengrid_types::FieldName, Value)> = {
        let borrowed = runtime.borrow();
        let schema = borrowed.state.schema();
        list.iter()
            .filter_map(|entry| {
                let field =
                    |name: &str| js_sys::Reflect::get(&entry, &JsValue::from_str(name)).ok();
                let key = key_from_js(&borrowed.state, &field("key")?)?;
                let column = opengrid_types::FieldName::new(field("column")?.as_string()?).ok()?;
                let data_type = schema
                    .fields()
                    .iter()
                    .find(|candidate| candidate.name == column)?
                    .data_type;
                let value = field("value").filter(|value| !value.is_undefined())?;
                let text = js_sys::JSON::stringify(&value).ok().map(String::from)?;
                let json = opengrid_json::Json::parse(&text).ok()?;
                let value = Value::from_json_typed(&json, &data_type).ok()?;
                Some((key, column, value))
            })
            .collect()
    };
    let cells = runtime.borrow_mut().state.set_values(entries);
    render(host, false);
    for cell in cells {
        redraw_cell(host, cell);
    }
}

/// Writes an edited value into its cell, tells the page and — for a real cell,
/// not the new row's — remembers it for undo (issue #186).
#[allow(clippy::too_many_arguments)]
fn commit_cell(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    cell: CellRef,
    value: Value,
    key: Option<&JsValue>,
    column: &str,
    text: &str,
    in_draft: bool,
) {
    let (before, record_key) = {
        let borrowed = runtime.borrow();
        (
            borrowed.state.cell(cell).cloned().unwrap_or(Value::Null),
            borrowed.state.row_key(cell.row).cloned(),
        )
    };
    let previous = crate::formats::event_text(&before);
    let changed = !runtime
        .borrow_mut()
        .state
        .set_cell(cell, value.clone())
        .is_empty();
    if !changed || in_draft {
        return;
    }
    dispatch_cell_change(host, cell, key, column, text, &previous, None);
    runtime
        .borrow_mut()
        .state
        .history_mut()
        .record(opengrid_grid::Step::Cell(opengrid_grid::CellEdit {
            row: cell.row,
            col: cell.col,
            key: record_key,
            before,
            after: value,
        }));
}

/// Fires `opengrid-cell-change` on the host — the contract of point 35. `step`
/// says `undo` or `redo` when the change is one of those (issue #186).
fn dispatch_cell_change(
    host: &HtmlElement,
    cell: CellRef,
    key: Option<&JsValue>,
    column: &str,
    value: &str,
    previous: &str,
    step: Option<&str>,
) {
    let detail = js_sys::Object::new();
    if let Some(key) = key {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("key"), key);
    }
    if let Some(step) = step {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(step), &JsValue::TRUE);
    }
    for (key, value) in [
        ("row", JsValue::from_f64(cell.row as f64)),
        ("column", JsValue::from_str(column)),
        ("value", JsValue::from_str(value)),
        ("previous", JsValue::from_str(previous)),
    ] {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(key), &value);
    }

    crate::element::dispatch(host, CELL_EVENT, &detail);
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "
export function __opengrid_numbers(values) {
    const list = new Array(values.length);
    for (let at = 0; at < list.length; at += 1) {
        list[at] = values[at];
    }
    return list;
}
")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = __opengrid_numbers)]
    fn numbers_to_array(values: &[f64]) -> js_sys::Array;
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
///   `{ rows: [u64], ranges: [{ from, to }], count: number }` — a selection
///   by position, in a grid without a `row-key`. One by key is
///   [`dispatch_keys`]'s `{ count, keys }`.
fn dispatch_selection(host: &HtmlElement, rows: &[u64]) {
    // One call and a loop in JavaScript, not one call per row: "select all"
    // over 100 000 rows names them all. The event still carries a plain array
    // of numbers (`Array.from` over a typed array was slower than the calls).
    let numbers: Vec<f64> = rows.iter().map(|row| *row as f64).collect();
    let list = numbers_to_array(&numbers);
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("rows"), &list);
    // The same rows as runs of adjacent ones: `Ctrl`+`A` over 100 000 rows is
    // one `{ from, to }`, for a page that must not walk the list.
    let ranges = js_sys::Array::new();
    for (from, to) in grid::selection_ranges(rows) {
        let range = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &range,
            &JsValue::from_str("from"),
            &JsValue::from_f64(from as f64),
        );
        let _ = js_sys::Reflect::set(
            &range,
            &JsValue::from_str("to"),
            &JsValue::from_f64(to as f64),
        );
        ranges.push(&range);
    }
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("ranges"), &ranges);
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("count"),
        &JsValue::from_f64(rows.len() as f64),
    );
    crate::element::dispatch(host, SELECTION_EVENT, &detail);
}

/// Fires `opengrid-selection-change` for a selection by key (issue #322) —
/// a tree's nodes, or the records of a grid with a `row-key`: `{ count, keys }`,
/// and neither `rows` nor `ranges`, because a position names no record.
fn dispatch_keys(host: &HtmlElement, keys: &[opengrid_json::Json]) {
    let detail = js_sys::Object::new();
    let list = js_sys::Array::new();
    for key in keys {
        list.push(&to_js(key));
    }
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("count"),
        &JsValue::from_f64(keys.len() as f64),
    );
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("keys"), &list);
    crate::element::dispatch(host, SELECTION_EVENT, &detail);
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
    // The filter row and the viewport share one horizontal position, so a
    // filter field stays under its column (issue #62). Written only where it
    // differs, so the echo of the other's scroll event stops at once.
    if let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        && let Some(runtime) = runtime(&host)
    {
        let part = target.get_attribute("part").unwrap_or_default();
        let from_viewport = part.split_whitespace().any(|name| name == "viewport");
        let other = if from_viewport {
            root.query_selector("[part=\"filter\"]").ok().flatten()
        } else if part == "filter" {
            root.query_selector("[part~=\"viewport\"]").ok().flatten()
        } else {
            None
        };
        if let Some(other) = other {
            let echo = runtime.borrow_mut().scroll_echo.take();
            let is_echo = echo == Some((from_viewport, target.scroll_left()));
            // Scrolled sideways, a group's label keeps to its pinned column
            // (issue #316, `pins::pins_css`). Written when it changes.
            if from_viewport && target.has_attribute("data-scrolled") != (target.scroll_left() != 0)
            {
                let _ = target.toggle_attribute("data-scrolled");
            }
            runtime.borrow_mut().scroll_left = target.scroll_left();
            if !is_echo && other.scroll_left() != target.scroll_left() {
                let before = other.scroll_left();
                other.set_scroll_left(target.scroll_left());
                // Only a scroll that happened sends an event back.
                if other.scroll_left() != before {
                    runtime.borrow_mut().scroll_echo = Some((!from_viewport, other.scroll_left()));
                }
            }
        }
    }
    // While paging there is nothing to scroll into: the sizer is the page, and
    // the window belongs to the pager (point 38).
    if is_paging(&host) {
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
    let mut scroll_top = viewport.scroll_top().max(0);
    // Past the sizer's limit the position is read through the scale (issue
    // #284); a change of the shift moves every row, window or not.
    let shifted = {
        let mut borrowed = runtime.borrow_mut();
        let previous = grid::Placement {
            scroll_top: borrowed.scroll_top.max(0) as u64,
            shift: borrowed.shift,
        };
        let placed = match scroll_scale(&borrowed, &viewport) {
            Some(scale) => scale.place(previous, scroll_top as u64),
            None => grid::Placement {
                scroll_top: scroll_top as u64,
                shift: 0,
            },
        };
        if placed.scroll_top != scroll_top as u64 {
            scroll_top = i32::try_from(placed.scroll_top).unwrap_or(i32::MAX);
            viewport.set_scroll_top(scroll_top);
        }
        borrowed.shift = placed.shift;
        // Remembered for the return: a browser forgets it (point 74).
        borrowed.scroll_top = scroll_top;
        placed.shift != previous.shift
    };
    if shifted {
        render(&host, false);
    }
    let scroll_top = grid::Placement {
        scroll_top: scroll_top as u64,
        shift: runtime.borrow().shift,
    }
    .virtual_top();

    let (total_count, pool, offset, row_height) = {
        let runtime = runtime.borrow();
        let pool = runtime
            .view
            .as_ref()
            .map(|view| view.pool() as u64)
            .unwrap_or(0);
        (
            runtime.state.row_count(),
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

/// Runs `f` once the browser has drawn `frames` more frames.
fn after_frames(frames: u32, f: impl FnOnce() + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::once_into_js(move || {
        if frames <= 1 {
            f();
        } else {
            after_frames(frames - 1, f);
        }
    });
    let _ = window.request_animation_frame(callback.unchecked_ref::<js_sys::Function>());
}

/// Puts the filter row right after a control in it took the focus (#379).
///
/// WebKit scrolls to a focused control in the next frame, not at once, and
/// neither with the row's `scroll-padding` in mind — a control ends up under
/// the pinned fields — nor leaving a pinned field alone — the row jumps to the
/// place the field has in it. So what the handler did at once is looked at
/// again two frames later, when that scroll has happened:
///
/// * a pinned field is in sight already: the row goes back to where it was
///   (`sideways`);
/// * any other control stands clear of the pinned fields, between those at
///   either end.
///
/// Only while the control still has the focus: a reader who moved on is left
/// alone.
fn settle_filter_focus(root: &ShadowRoot, row: &Element, control: &Element, sideways: Option<i32>) {
    let (root, row, control) = (root.clone(), row.clone(), control.clone());
    after_frames(2, move || {
        let Some(active) = root.active_element() else {
            return;
        };
        let focused = if control.get_attribute("data-filter-field").is_some() {
            control.contains(Some(&active))
        } else {
            active == control
        };
        if !focused {
            return;
        }
        if let Some(sideways) = sideways {
            if row.scroll_left() != sideways {
                row.set_scroll_left(sideways);
            }
            // The row had moved the table with it, by its scroll event.
            if let Ok(Some(viewport)) = root.query_selector("[part~=\"viewport\"]")
                && viewport.scroll_left() != sideways
            {
                viewport.set_scroll_left(sideways);
            }
            return;
        }
        // The room between the pinned fields at either end, where they stand
        // now: the row's `scroll-padding` is meant to say the same, but is a
        // computed number that has to agree with the fields to the pixel.
        let scrollport_left = row.get_bounding_client_rect().left() + f64::from(row.client_left());
        let width = f64::from(row.client_width());
        let (mut allowed_left, mut allowed_right) = (scrollport_left, scrollport_left + width);
        if let Ok(fields) = row.query_selector_all("[data-filter-track] > span") {
            for index in 0..fields.length() {
                let Some(field) = fields
                    .item(index)
                    .and_then(|node| node.dyn_into::<Element>().ok())
                else {
                    continue;
                };
                if !is_pinned(&field) {
                    continue;
                }
                let rect = field.get_bounding_client_rect();
                if rect.left() + rect.width() / 2.0 < scrollport_left + width / 2.0 {
                    allowed_left = allowed_left.max(rect.right());
                } else {
                    allowed_right = allowed_right.min(rect.left());
                }
            }
        }
        let rect = control.get_bounding_client_rect();
        if rect.left() < allowed_left {
            row.set_scroll_left(row.scroll_left() - (allowed_left - rect.left()).ceil() as i32);
        } else if rect.right() > allowed_right {
            row.set_scroll_left(row.scroll_left() + (rect.right() - allowed_right).ceil() as i32);
        }
    });
}

/// [`on_focus_in`] for the fields that fire no `focusin` in WebKit.
fn on_focus_without_focusin(event: Event) {
    let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
        return;
    };
    if target
        .matches("input[type=\"date\"], input[type=\"time\"], input[type=\"datetime-local\"]")
        .unwrap_or(false)
    {
        on_focus_in(event);
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
    // The filter row and the toolbar's row (issue #77) scroll sideways when
    // narrow and the facets scroll down, and a browser does not reliably bring
    // a control that takes the focus into view inside them — Chromium leaves
    // one that shows a few pixels at the edge where it is. Tabbing to one hidden past the edge is losing sight of
    // the focus (WCAG 2.4.11). The scrollers themselves are only focused by a
    // click, and a click needs no scrolling.
    if target
        .closest("[part=\"filter\"], [part=\"facets\"], [data-toolbar-row]")
        .ok()
        .flatten()
        .is_some_and(|scroller| scroller != target)
    {
        let options = ScrollIntoViewOptions::new();
        options.set_block(ScrollLogicalPosition::Nearest);
        options.set_inline(ScrollLogicalPosition::Nearest);
        // The operator button sits inside its field (issue #96): the whole
        // field comes into view, so the Tab from the button to the value lands
        // on a control that is already in sight.
        let into = if target.get_attribute("part").as_deref() == Some("filter-operator") {
            target.parent_element().unwrap_or_else(|| target.clone())
        } else {
            target.clone()
        };
        // A pinned column's field is in sight already; the row stays where
        // it was instead of scrolling to the field's place in it.
        let row = target.closest("[part=\"filter\"]").ok().flatten();
        let sideways = scrolled_sideways(&root);
        if let Ok(Some(field)) = target.closest("[data-filter-track] > span")
            && is_pinned(&field)
        {
            if let (Some(row), Some(sideways)) = (&row, sideways) {
                row.set_scroll_left(sideways);
                settle_filter_focus(&root, row, &target, Some(sideways));
            }
            return;
        }
        into.scroll_into_view_with_scroll_into_view_options(&options);
        if let Some(row) = &row {
            settle_filter_focus(&root, row, &into, None);
        }
        return;
    }
    let Some(active) = active_from_element(&target) else {
        return;
    };
    let (from, viewport) = {
        let runtime = runtime.borrow();
        (runtime.active, runtime.viewport.clone())
    };
    // A band's cell that has the focus already keeps the column the reader
    // came up from (issue #319): `↓` goes back to it.
    let same_band =
        matches!(from, ActiveCell::Band { .. }) && target.matches(&selector(from)).unwrap_or(false);
    if from == active || same_band {
        return;
    }
    range::on_focus(&host, &runtime, active);
    runtime.borrow_mut().set_active(active);
    focus_from(&root, viewport.as_ref(), from, active);
}

/// The shadow root a delegated listener was installed on.
/// The shadow root the event's target sits in.
///
/// A listener that outlives its popup finds the grid from the event instead of
/// holding it: a closure that captures the host, the root or a button in the
/// grid is a root of its own and keeps a removed grid from being collected
/// (issue #227).
fn root_of_target(event: &Event) -> Option<ShadowRoot> {
    event
        .target()?
        .dyn_into::<Element>()
        .ok()?
        .get_root_node()
        .dyn_into::<ShadowRoot>()
        .ok()
}

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
// Grouping (point 62)
// ---------------------------------------------------------------------------

/// The grouping the attributes ask for, or why it cannot be had.
///
/// `Ok(None)`: no `group-by`. `Err`: a sentence for the status line — more than
/// three levels, a column the grid does not show, or paging, which grouping
/// cannot live beside (paging excludes virtualization since point 38, and the
/// group arithmetic presupposes it).
fn grouping_of(host: &HtmlElement) -> Result<Option<Grouping>, String> {
    let raw = host.get_attribute(grid::GROUP_BY_ATTRIBUTE);
    let texts = texts(host);
    let by = grouping::parse_group_by(raw.as_deref()).map_err(|all| texts.group_invalid(&all))?;
    if by.is_empty() {
        return Ok(None);
    }
    // A tree is its own hierarchy: grouping beside it is not used, and said.
    if tree_of(host).is_ok_and(|tree| tree.is_some()) {
        return Err(texts.tree_group_ignored.clone());
    }
    let shown = columns_of(host);
    if let Some(missing) = by.iter().find(|name| !shown.contains(name)) {
        return Err(texts.group_invalid(missing));
    }
    if is_paging(host) {
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

/// Whether `active` is a data row of a grouped grid that may be edited: the
/// grid has a `row-key` (an edit names its record, not a display position) and
/// the position is a row, not a group header or the total (issue #190).
fn editable_group_row(host: &HtmlElement, active: ActiveCell) -> bool {
    let Some(runtime) = runtime(host) else {
        return false;
    };
    let borrowed = runtime.borrow();
    has_row_key(host)
        && matches!(active, ActiveCell::Data(_))
        && active.row().is_some_and(|row| {
            matches!(
                borrowed
                    .grouping
                    .as_ref()
                    .and_then(|grouping| grouping.item_at(row)),
                Some(grouping::Item::Row { .. })
            )
        })
}

/// Whether the grid is a tree right now.
fn is_tree(host: &HtmlElement) -> bool {
    runtime(host).is_some_and(|runtime| runtime.borrow().tree.is_some())
}

/// The tree the attributes ask for, or why it cannot be had.
///
/// `Ok(None)`: no `tree`. `Err`: beside `page-size` — a level is loaded whole
/// and the display list scrolls, so there is no page to cut (issue #135).
fn tree_of(host: &HtmlElement) -> Result<Option<Tree>, String> {
    let Some(parent) = host
        .get_attribute(grid::TREE_ATTRIBUTE)
        .filter(|parent| !parent.trim().is_empty())
    else {
        return Ok(None);
    };
    if is_paging(host) {
        return Err(texts(host).tree_refused.clone());
    }
    let key = host
        .get_attribute(grid::TREE_KEY_ATTRIBUTE)
        .filter(|key| !key.trim().is_empty())
        .unwrap_or_else(|| "id".to_owned());
    Ok(Some(Tree::new(key.trim(), parent.trim())))
}

/// Takes a new `group-by`: starts at the top, drops the selection, and says it
/// when the grouping is refused — once, riding with the next result.
fn regroup(host: &HtmlElement) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let pool = pool_of(host);
    let grouping = grouping_of(host);
    // The focus goes with the active cell, but only if it was in the table:
    // from a toolbar button or a closing menu it stays where it is (#345).
    let in_table = host.shadow_root().is_some_and(|root| {
        root.active_element()
            .is_some_and(|focused| focused.closest("td, th").ok().flatten().is_some())
    });
    let dropped = {
        let mut borrowed = runtime.borrow_mut();
        borrowed.grouping = grouping.clone().ok().flatten();
        borrowed.groups_filter = None;
        borrowed.state.set_window(Window::new(0, pool));
        // A row means something else once the list is grouped differently:
        // the cell goes to its column's header. A header or a band is still
        // there, and stays the active cell — the one a column menu came from.
        let active = match borrowed.active {
            ActiveCell::Data(cell) => ActiveCell::Header { col: cell.col },
            ActiveCell::Select { .. } => ActiveCell::Header { col: 0 },
            kept
            @ (ActiveCell::Header { .. } | ActiveCell::Band { .. } | ActiveCell::SelectAll) => kept,
        };
        borrowed.set_active(active);
        // A selection by key names records and stays from one grouping to the
        // next (issue #322); the result shows it where they sit then — or
        // drops it, when the grid is ungrouped now ([`settle`]). A selection
        // of positions cannot follow into a display list, where a position
        // is a header as often as a row: it is dropped, and said.
        let dropped = if borrowed.picked.is_empty() {
            !borrowed.state.invalidate_selection().is_empty()
        } else {
            borrowed.state.set_selection(Vec::new());
            false
        };
        if let Err(message) = &grouping {
            borrowed.state.set_notice(message.clone());
        }
        dropped
    };
    if dropped {
        dispatch_selection(host, &[]);
    }
    let viewport = runtime.borrow().viewport.clone();
    if let Some(viewport) = viewport {
        viewport.set_scroll_top(0);
    }
    run_query(host, QueryKind::Data, in_table);
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
        // A lookup column is counted and nothing else (issue #326).
        let lookup = configured.lookup(name).is_some();
        if presentation::aggregates_of(field.data_type, lookup).contains(&function) {
            chosen.push((name.to_owned(), function));
        } else {
            refused.push(format!(
                "{name}: {} is not an aggregate for {}",
                function.as_str(),
                if lookup {
                    "a lookup column"
                } else {
                    "this type"
                }
            ));
        }
    }
    (chosen, refused)
}

/// A provider's answer as a result, whichever form it came in (E35).
fn read_answer(value: &JsValue) -> Result<opengrid_datasource::QueryResult, String> {
    match crate::element::answer(value) {
        Some(crate::element::Answer::Json(json)) => grid::parse_result(&json),
        Some(crate::element::Answer::Binary(bytes)) => grid::parse_result_bytes(&bytes),
        None => Err(crate::element::NOT_AN_ANSWER.to_owned()),
    }
}

/// Runs one query through the provider and reads its result.
///
/// Counted for the footer (issue #33); the facets count theirs themselves and
/// ask through [`ask_uncounted`].
async fn ask(
    host: &HtmlElement,
    provider: &Rc<dyn opengrid_web_core::provider::DataProvider>,
    runtime: &Rc<RefCell<GridRuntime>>,
    query: &str,
    mode: &str,
) -> Result<opengrid_datasource::QueryResult, String> {
    runtime.borrow_mut().queries += 1;
    ask_uncounted(host, provider, query, mode).await
}

/// [`ask`] without counting the query.
///
/// Every query of the grid passes here, so here it is measured (issue #70):
/// the round trip, and the answer's size as it arrived — reported on the
/// element as `opengrid-query`.
async fn ask_uncounted(
    host: &HtmlElement,
    provider: &Rc<dyn opengrid_web_core::provider::DataProvider>,
    query: &str,
    mode: &str,
) -> Result<opengrid_datasource::QueryResult, String> {
    let started = now();
    let answered = JsFuture::from(provider.execute(query, mode)).await;
    measured(host, provider.kind(), started, answered)
}

/// Reads an answer and reports it as `opengrid-query` (issue #70) — for
/// [`ask_uncounted`] and for the window query, which starts its promise
/// before it awaits it.
fn measured(
    host: &HtmlElement,
    kind: Option<String>,
    started: f64,
    answered: Result<JsValue, JsValue>,
) -> Result<opengrid_datasource::QueryResult, String> {
    let value = match answered {
        Ok(value) => value,
        Err(value) => return Err(describe(&value)),
    };
    let ms = now() - started;
    let (bytes, form) = answer_size(&value);
    let result = read_answer(&value)?;
    dispatch_query(
        host,
        kind,
        ms,
        result.row_count() as u64,
        result.total_count,
        bytes,
        form,
    );
    Ok(result)
}

/// What a grouped or a tree run starts from.
struct RunStart {
    provider: Rc<dyn opengrid_web_core::provider::DataProvider>,
    source: String,
    columns: Vec<String>,
    pool: u64,
    mode: String,
    filter: Option<FilterExpr>,
    /// The number of this run: it supersedes every run that began before it.
    generation: u64,
}

/// The start the grouped and the tree runs share (#355): the provider and the
/// source, the columns, the filter, and the new generation.
///
/// `None` when there is nothing to ask — no provider, no source — or when a
/// bound is not a value of its column, which is the same sentence for the
/// status line as ungrouped ([`run_query`]).
fn begin_run(host: &HtmlElement, grid_runtime: &Rc<RefCell<GridRuntime>>) -> Option<RunStart> {
    let provider = provider(host)?;
    let source = host.get_attribute(DATASOURCE_ATTRIBUTE)?;
    let columns = columns_of(host);
    let pool = pool_of(host);
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();

    // The filter row *and* the facets (point 66): a facet change makes the
    // group counts stale exactly like a filter change does. This fell back to
    // the filter row alone once — silently dropping the facets and the search,
    // and showing rows that `get_query` (which answers `null` here) would not
    // export.
    let effective = effective_filter(host, &grid_runtime.borrow(), None);
    let filter = match effective {
        Ok(filter) => filter,
        Err(message) => {
            grid_runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(message));
            render(host, false);
            return None;
        }
    };
    let generation = {
        let mut runtime = grid_runtime.borrow_mut();
        runtime.generation += 1;
        runtime.generation
    };
    Some(RunStart {
        provider,
        source,
        columns,
        pool,
        mode,
        filter,
        generation,
    })
}

/// The lines before a run that asks again: "loading" in the status line — not
/// when only the window moves — and the count of the queries it asks, from here
/// (issue #33).
fn announce_loading(host: &HtmlElement, grid_runtime: &Rc<RefCell<GridRuntime>>, kind: QueryKind) {
    if kind == QueryKind::Data {
        grid_runtime
            .borrow_mut()
            .state
            .set_status(GridStatus::Loading);
        render(host, false);
        refresh_facets(host);
    }
    grid_runtime.borrow_mut().queries = 0;
}

/// The sentence that says a group or a node opened or closed: its value as the
/// grid shows it, how many rows or children it holds, and which way it went.
fn toggled_message(
    host: &HtmlElement,
    schema: &opengrid_types::Schema,
    value: &Value,
    column: &str,
    count: u64,
    open: bool,
) -> String {
    let texts = texts(host);
    let label = grid::group_value_text(
        value,
        column,
        schema.fields(),
        &texts,
        &formatter(host, schema),
    );
    texts.group_toggled(&label, count, open)
}

/// Says a toggle once, riding with the result that follows, and asks for the
/// window again — as a window, so the status line stays quiet — with the focus
/// kept.
fn announce_toggle(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, message: String) {
    runtime.borrow_mut().state.set_notice(message);
    run_query(host, QueryKind::Window, true);
    dispatch_view(host);
}

/// Moves the focus to the first cell of the row at `row` — a node's parent
/// (issue #135) — loading the window around it when it is not in view.
fn move_to_row(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, row: u64) {
    let (from, window, total) = {
        let runtime = runtime.borrow();
        (
            runtime.active,
            runtime.state.window(),
            runtime.state.total_count(),
        )
    };
    let next = ActiveCell::Data(CellRef { row, col: 0 });
    runtime.borrow_mut().set_active(next);
    if row < window.offset || row >= window.offset + window.count {
        let pool = pool_of(host);
        let offset = grid::window_offset_for_row(row, total, pool);
        scroll_to_far_row(runtime, row);
        runtime
            .borrow_mut()
            .state
            .set_window(Window::new(offset, pool));
        run_query(host, QueryKind::Window, true);
    } else if let Some(root) = host.shadow_root() {
        let viewport = runtime.borrow().viewport.clone();
        focus_from(&root, viewport.as_ref(), from, next);
    }
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
        // Rows the reader added are rows: the empty state would cover them.
        let empty = matches!(borrowed.state.status(), GridStatus::Empty)
            && borrowed.state.added().is_empty();
        // The reader's filters alone: the reset takes back nothing of the page's.
        let filtered = reader_filter(host, &borrowed, None).is_ok_and(|filter| filter.is_some());
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
