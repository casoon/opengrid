//! The pointer on data cells (issue #524): a mouse, a finger and a pen.
//!
//! - A **double-click**, a **double-tap** and a **tap on the focused cell** open
//!   the editor, the way spreadsheets do. The editor gets the focus inside the
//!   tap's own `click`, so a phone raises its keyboard.
//! - A **long-press** (and `contextmenu`: a right click, Android's long-press)
//!   opens the [cell menu](super::cell_menu) — when the finger or the button
//!   lifts: a popover opened while it is down is light-dismissed by its
//!   `pointerup`.
//! - The **range** follows the pointer: a mouse drags it out at once; a finger
//!   only after a long-press, because a plain swipe scrolls.
//!
//! Every way into an edit goes through `begin_edit`, which asks the lock, the
//! read-only columns and the grouping (issues #494, #523) like the keys do.

use super::*;
use web_sys::PointerEvent;

/// How long a finger rests on a cell before it is a long-press, in ms — the
/// platforms' own figure.
const LONG_PRESS_MS: i32 = 500;
/// How far a finger may wander, in CSS pixels, and still be a tap or a press.
const SLOP: f64 = 10.0;

/// A finger or a pen on a cell, until it lifts.
pub(super) struct Press {
    pointer: i32,
    cell: CellRef,
    x: f64,
    y: f64,
    /// Whether the cell had the focus before the finger came down.
    focused: bool,
    /// The long-press timer, until it fires or the press ends.
    timer: Option<i32>,
    /// The press lasted: the menu opened, and a move now draws a range.
    long: bool,
    /// The finger moved past the slop.
    moved: bool,
}

/// A tap that ended on the cell it began on, for the `click` that follows.
pub(super) struct Tap {
    cell: CellRef,
    focused: bool,
}

/// A drag on a header's edge, until the pointer lifts (issue #563).
struct WidthDrag {
    pointer: i32,
    name: String,
    x: f64,
    width: u32,
    /// The pointer went anywhere: a press that did not is not a resize.
    moved: bool,
}

/// A press on a header with a mouse or a pen, until it lifts: past the slop it
/// is a drag that moves the column (issue #565).
struct HeaderDrag {
    pointer: i32,
    col: usize,
    x: f64,
    /// The pointer went past the slop: the press is a drag now.
    started: bool,
    /// The column the pointer is over, where the drop would put this one.
    over: Option<usize>,
}

thread_local! {
    static HEADER_DRAG: RefCell<Option<HeaderDrag>> = const { RefCell::new(None) };
    static WIDTH_DRAG: RefCell<Option<WidthDrag>> = const { RefCell::new(None) };
    /// A width drag has just ended: the `click` after it is not a sort.
    static DRAG_ENDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Listens on the shadow root, once.
pub(super) fn add_listeners(root: &ShadowRoot) {
    for (kind, handler) in [
        ("pointerdown", on_pointer_down as fn(Event)),
        ("pointermove", on_pointer_move as fn(Event)),
        ("pointerup", on_pointer_up as fn(Event)),
        ("pointercancel", on_pointer_cancel as fn(Event)),
        ("click", on_click as fn(Event)),
        ("dblclick", on_double_click as fn(Event)),
        ("contextmenu", on_context_menu as fn(Event)),
        ("pointerover", on_header_over as fn(Event)),
    ] {
        let listener = Closure::<dyn FnMut(Event)>::new(handler).into_js_value();
        let _ = root.add_event_listener_with_callback(kind, listener.unchecked_ref());
    }
    // While a finger draws a range the page must not scroll under it. Only a
    // `touchmove` that is not passive can say so; it lets every other move be.
    let options = web_sys::AddEventListenerOptions::new();
    options.set_passive(false);
    let listener = Closure::<dyn FnMut(Event)>::new(on_touch_move).into_js_value();
    let _ = root.add_event_listener_with_callback_and_add_event_listener_options(
        "touchmove",
        listener.unchecked_ref(),
        &options,
    );
}

fn is_mouse(event: &PointerEvent) -> bool {
    event.pointer_type() == "mouse"
}

fn on_pointer_down(event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    if begin_width_drag_on(&event, pointer) {
        return;
    }
    begin_header_press(&event, pointer);
    let Some((host, cell, td)) = range::data_cell(&event) else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    runtime.borrow_mut().tap = None;
    if is_mouse(pointer) {
        range::press(&host, &runtime, cell, pointer);
        return;
    }
    if !pointer.is_primary() {
        return;
    }
    end_press(&runtime);
    let focused = host
        .shadow_root()
        .and_then(|root| root.active_element())
        .is_some_and(|active| active.is_same_node(Some(&td)));
    let id = pointer.pointer_id();
    let timer = {
        let host = host.clone();
        let callback = Closure::once_into_js(move || long_press(&host, id));
        web_sys::window().and_then(|window| {
            window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.unchecked_ref(),
                    LONG_PRESS_MS,
                )
                .ok()
        })
    };
    runtime.borrow_mut().press = Some(Press {
        pointer: id,
        cell,
        x: f64::from(pointer.client_x()),
        y: f64::from(pointer.client_y()),
        focused,
        timer,
        long: false,
        moved: false,
    });
}

thread_local! {
    /// A menu asked for while a mouse button was down, opened when it lifts.
    static PENDING: std::cell::Cell<Option<CellRef>> = const { std::cell::Cell::new(None) };
    /// The column whose menu a right click on its header waits to open.
    static PENDING_COLUMN: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Opens the menu of `cell` once the event that lifted the pointer is done.
/// Gives a header's label a `title` while it is cut off, and takes it away when
/// it fits again (issue #570): measured on hover, as the width changes with the
/// column, the marks beside it and the window.
fn on_header_over(event: Event) {
    let Some(label) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .and_then(|target| target.closest("th[part=\"header\"]").ok().flatten())
        .and_then(|th| th.first_element_child())
    else {
        return;
    };
    if label.scroll_width() > label.client_width() {
        if label.get_attribute("title").is_none() {
            let text = label.text_content().unwrap_or_default();
            let _ = label.set_attribute("title", &text);
        }
    } else {
        let _ = label.remove_attribute("title");
    }
}

/// A right click on a header, with `column-menu`: the column's menu, the
/// pointer path to what `Alt`+`↓` opens (issue #569). `true` when it was one.
fn on_header_context_menu(event: &Event) -> bool {
    let Some(root) = current_shadow_root(event) else {
        return false;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return false;
    };
    if !host.has_attribute(grid::COLUMN_MENU_ATTRIBUTE) {
        return false;
    }
    let Some(th) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .and_then(|target| {
            target
                .closest("th[part=\"header\"][data-col]")
                .ok()
                .flatten()
        })
    else {
        return false;
    };
    let Some(col) = th
        .get_attribute("data-col")
        .and_then(|col| col.parse::<usize>().ok())
    else {
        return false;
    };
    let Some(runtime) = runtime(&host) else {
        return false;
    };
    event.prevent_default();
    runtime.borrow_mut().set_active(ActiveCell::Header { col });
    // As for a cell: a button still down waits for it to lift, or the menu
    // would be light-dismissed by that `pointerup`.
    let held = event
        .dyn_ref::<web_sys::MouseEvent>()
        .is_some_and(|mouse| mouse.buttons() != 0);
    if held {
        PENDING_COLUMN.with(|pending| pending.set(Some(col)));
    } else {
        open_column_menu(&host, col);
    }
    true
}

fn open_column_menu_after(host: &HtmlElement, col: usize) {
    let host = host.clone();
    let callback = Closure::once_into_js(move || open_column_menu(&host, col));
    if let Some(window) = web_sys::window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), 0);
    }
}

/// How near a header's end a pointer is on its edge, in CSS pixels: a mouse
/// needs a thin one, a finger the 24 px of WCAG 2.5.8.
const EDGE: f64 = 8.0;
const EDGE_TOUCH: f64 = 24.0;

/// The header cell a pointer is on the edge of, with its column. Not from its
/// menu button, which stands near the same end.
fn header_edge(
    host: &HtmlElement,
    target: &Element,
    x: f64,
    touch: bool,
) -> Option<(Element, usize)> {
    if target
        .closest("[part=\"column-menu-button\"]")
        .ok()
        .flatten()
        .is_some()
    {
        return None;
    }
    let th = target
        .closest("th[part=\"header\"][data-col]")
        .ok()
        .flatten()?;
    let col = th.get_attribute("data-col")?.parse().ok()?;
    let rect = th.get_bounding_client_rect();
    let near = if crate::element::is_rtl(host) {
        x - rect.left()
    } else {
        rect.right() - x
    };
    (near >= 0.0 && near <= if touch { EDGE_TOUCH } else { EDGE }).then_some((th, col))
}

/// Notes on the header the mouse is on the edge of, and takes the note from
/// the one it left, so the stylesheet shows the cursor (issue #563).
fn mark_edge(host: &HtmlElement, root: &ShadowRoot, pointer: &PointerEvent, target: &Element) {
    let now = header_edge(host, target, f64::from(pointer.client_x()), false).map(|(th, _)| th);
    if let Ok(Some(old)) = root.query_selector("[part=\"header\"][data-edge]")
        && !now.as_ref().is_some_and(|th| th.is_same_node(Some(&old)))
        && !WIDTH_DRAG.with(|drag| drag.borrow().is_some())
    {
        let _ = old.remove_attribute("data-edge");
    }
    if let Some(th) = now {
        let _ = th.set_attribute("data-edge", "");
    }
}

/// The primary button, a finger or a pen went down on a header's edge: a drag
/// that changes the column's width begins. `true` when it did.
fn begin_width_drag_on(event: &Event, pointer: &PointerEvent) -> bool {
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return false;
    };
    if !pointer.is_primary() || (is_mouse(pointer) && pointer.button() != 0) {
        return false;
    }
    let Some(root) = current_shadow_root(event) else {
        return false;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return false;
    };
    let Some((th, col)) = header_edge(
        &host,
        &target,
        f64::from(pointer.client_x()),
        !is_mouse(pointer),
    ) else {
        return false;
    };
    let Some((name, width)) = begin_width_drag(&host, col) else {
        return false;
    };
    // The header keeps the pointer wherever it goes, and the page keeps still.
    let _ = th.set_pointer_capture(pointer.pointer_id());
    let _ = th.set_attribute("data-edge", "");
    event.prevent_default();
    WIDTH_DRAG.with(|drag| {
        *drag.borrow_mut() = Some(WidthDrag {
            pointer: pointer.pointer_id(),
            name,
            x: f64::from(pointer.client_x()),
            width,
            moved: false,
        });
    });
    true
}

/// The pointer moved while a width drag is on: the column follows it. The
/// direction of the text decides which way is wider.
fn move_width_drag(host: &HtmlElement, pointer: &PointerEvent) -> bool {
    let target = WIDTH_DRAG.with(|drag| {
        drag.borrow_mut()
            .as_mut()
            .filter(|drag| drag.pointer == pointer.pointer_id())
            .map(|drag| {
                drag.moved |= f64::from(pointer.client_x()) != drag.x;
                (drag.name.clone(), drag.x, drag.width)
            })
    });
    let Some((name, x, width)) = target else {
        return false;
    };
    let mut by = f64::from(pointer.client_x()) - x;
    if crate::element::is_rtl(host) {
        by = -by;
    }
    drag_width(host, &name, width as i32 + by.round() as i32);
    true
}

/// The pointer lifted, or was taken away: the drag ends and the width is said.
fn end_width_drag_on(host: &HtmlElement, root: &ShadowRoot, pointer: &PointerEvent) -> bool {
    let done = WIDTH_DRAG.with(|drag| {
        let mut drag = drag.borrow_mut();
        if drag
            .as_ref()
            .is_some_and(|drag| drag.pointer == pointer.pointer_id())
        {
            drag.take()
        } else {
            None
        }
    });
    let Some(done) = done else {
        return false;
    };
    if let Ok(Some(edge)) = root.query_selector("[part=\"header\"][data-edge]") {
        let _ = edge.remove_attribute("data-edge");
    }
    // A press that went nowhere said nothing: the width is as it was.
    if done.moved {
        end_width_drag(host, &done.name);
    }
    // The `click` that follows on the header is the drag's, not a sort.
    DRAG_ENDED.with(|ended| ended.set(true));
    let clear = Closure::once_into_js(|| DRAG_ENDED.with(|ended| ended.set(false)));
    if let Some(window) = web_sys::window() {
        let _ =
            window.set_timeout_with_callback_and_timeout_and_arguments_0(clear.unchecked_ref(), 0);
    }
    true
}

/// Whether a `click` is the end of a width drag, or on a header's edge: it
/// does not sort.
pub(super) fn click_is_on_edge(host: &HtmlElement, target: &Element, x: f64) -> bool {
    DRAG_ENDED.with(std::cell::Cell::get) || header_edge(host, target, x, false).is_some()
}

/// A double-click on a header's edge fits the column to what it shows (issue
/// #564), the way a spreadsheet does. `true` when it was one.
fn autosize_on_edge(event: &Event) -> bool {
    let Some(click) = event.dyn_ref::<web_sys::MouseEvent>() else {
        return false;
    };
    let Some(root) = current_shadow_root(event) else {
        return false;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return false;
    };
    let Some((_, col)) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .and_then(|target| header_edge(&host, &target, f64::from(click.client_x()), false))
    else {
        return false;
    };
    event.prevent_default();
    autosize_column(&host, col);
    true
}

/// How far a mouse or a pen moves with a header held before it is a drag.
const HEADER_SLOP: f64 = 5.0;

/// A mouse button or a pen went down on a header: it may become a drag.
/// A finger scrolls instead, and the keys move a column without a pointer.
fn begin_header_press(event: &Event, pointer: &PointerEvent) {
    if !pointer.is_primary()
        || (is_mouse(pointer) && pointer.button() != 0)
        || pointer.pointer_type() == "touch"
    {
        return;
    }
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return;
    };
    if target
        .closest("[part=\"column-menu-button\"]")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let Some(col) = target
        .closest("th[part=\"header\"][data-col]")
        .ok()
        .flatten()
        .and_then(|th| th.get_attribute("data-col"))
        .and_then(|col| col.parse::<usize>().ok())
    else {
        return;
    };
    HEADER_DRAG.with(|drag| {
        *drag.borrow_mut() = Some(HeaderDrag {
            pointer: pointer.pointer_id(),
            col,
            x: f64::from(pointer.client_x()),
            started: false,
            over: None,
        });
    });
}

/// The header whose box holds `x`, and the side of it the pointer is nearer.
fn header_at(root: &ShadowRoot, x: f64) -> Option<(usize, Element)> {
    let headers = root
        .query_selector_all("th[part=\"header\"][data-col]")
        .ok()?;
    for index in 0..headers.length() {
        let th = headers.item(index)?.dyn_into::<Element>().ok()?;
        let rect = th.get_bounding_client_rect();
        if x >= rect.left() && x < rect.right() {
            let col = th.get_attribute("data-col")?.parse().ok()?;
            return Some((col, th));
        }
    }
    None
}

/// Takes the drop mark off every header.
fn clear_drop_marks(root: &ShadowRoot) {
    if let Ok(marked) = root.query_selector_all("[data-drop], [data-moving]") {
        for index in 0..marked.length() {
            if let Some(th) = marked
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                let _ = th.remove_attribute("data-drop");
                let _ = th.remove_attribute("data-moving");
            }
        }
    }
}

/// The pointer moved with a header held: past the slop the column is being
/// dragged, and the header it is over wears the drop mark. `true` while it is.
fn move_header_drag(root: &ShadowRoot, pointer: &PointerEvent) -> bool {
    let x = f64::from(pointer.client_x());
    // The button came up where this grid heard nothing of it: no drag.
    if pointer.buttons() & 1 == 0 {
        HEADER_DRAG.with(|drag| {
            let mut drag = drag.borrow_mut();
            if drag
                .as_ref()
                .is_some_and(|drag| drag.pointer == pointer.pointer_id())
            {
                *drag = None;
            }
        });
        return false;
    }
    let started = HEADER_DRAG.with(|drag| {
        let mut drag = drag.borrow_mut();
        let drag = drag
            .as_mut()
            .filter(|drag| drag.pointer == pointer.pointer_id())?;
        if !drag.started && (x - drag.x).abs() > HEADER_SLOP {
            drag.started = true;
        }
        drag.started.then_some(drag.col)
    });
    let Some(col) = started else {
        return false;
    };
    clear_drop_marks(root);
    if let Ok(Some(moving)) =
        root.query_selector(&format!("th[part=\"header\"][data-col=\"{col}\"]"))
    {
        let _ = moving.set_attribute("data-moving", "");
        // Held here, the header keeps the pointer wherever it goes.
        let _ = moving.set_pointer_capture(pointer.pointer_id());
    }
    let over = header_at(root, x);
    if let Some((target, th)) = &over
        && *target != col
    {
        let _ = th.set_attribute("data-drop", if *target < col { "before" } else { "after" });
    }
    HEADER_DRAG.with(|drag| {
        if let Some(drag) = drag.borrow_mut().as_mut() {
            drag.over = over.map(|(target, _)| target);
        }
    });
    true
}

/// The pointer lifted: a header that was dragged is dropped where it is, and
/// the click after it is not a sort. `true` when there was a drag.
fn end_header_drag_on(host: &HtmlElement, root: &ShadowRoot, pointer: &PointerEvent) -> bool {
    let done = HEADER_DRAG.with(|drag| {
        let mut drag = drag.borrow_mut();
        if drag
            .as_ref()
            .is_some_and(|drag| drag.pointer == pointer.pointer_id())
        {
            drag.take()
        } else {
            None
        }
    });
    let Some(done) = done.filter(|done| done.started) else {
        return false;
    };
    clear_drop_marks(root);
    suppress_next_click();
    if let Some(over) = done.over {
        move_column_to(host, done.col, over);
    }
    true
}

/// The `click` that follows a drag is the drag's, not a press on a header.
fn suppress_next_click() {
    DRAG_ENDED.with(|ended| ended.set(true));
    let clear = Closure::once_into_js(|| DRAG_ENDED.with(|ended| ended.set(false)));
    if let Some(window) = web_sys::window() {
        let _ =
            window.set_timeout_with_callback_and_timeout_and_arguments_0(clear.unchecked_ref(), 0);
    }
}

fn open_menu_after(host: &HtmlElement, cell: CellRef) {
    let host = host.clone();
    let callback = Closure::once_into_js(move || cell_menu::open(&host, cell));
    if let Some(window) = web_sys::window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(callback.unchecked_ref(), 0);
    }
}

/// The finger stayed: the cell takes the focus and a range may begin there;
/// the menu opens when the finger lifts without moving.
fn long_press(host: &HtmlElement, pointer: i32) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let cell = {
        let mut borrowed = runtime.borrow_mut();
        let Some(press) = borrowed
            .press
            .as_mut()
            .filter(|press| press.pointer == pointer && !press.moved)
        else {
            return;
        };
        press.timer = None;
        press.long = true;
        press.cell
    };
    focus_cell(host, &runtime, cell);
    range::begin_drag(host, &runtime, cell);
}

/// Gives `cell` the focus, unless it lies in the marked block: a menu over a
/// block copies the block, and the block ends when the focus moves.
pub(super) fn focus_cell(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, cell: CellRef) {
    let in_block = runtime
        .borrow()
        .range
        .is_some_and(|range| !range.is_single() && range.contains(cell));
    if in_block || runtime.borrow().active == ActiveCell::Data(cell) {
        return;
    }
    runtime.borrow_mut().set_active(ActiveCell::Data(cell));
    focus_active(host);
}

fn on_pointer_move(event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    if move_width_drag(&host, pointer) {
        return;
    }
    if move_header_drag(&root, pointer) {
        return;
    }
    if is_mouse(pointer)
        && let Some(target) = event
            .target()
            .and_then(|target| target.dyn_into::<Element>().ok())
    {
        mark_edge(&host, &root, pointer, &target);
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    if is_mouse(pointer) {
        if let Some((_, cell, _)) = range::data_cell(&event) {
            range::drag_over(&host, &runtime, cell, pointer.buttons() & 1 != 0);
        }
        return;
    }
    let (x, y) = (f64::from(pointer.client_x()), f64::from(pointer.client_y()));
    let drawing = {
        let mut borrowed = runtime.borrow_mut();
        let Some(press) = borrowed
            .press
            .as_mut()
            .filter(|press| press.pointer == pointer.pointer_id())
        else {
            return;
        };
        let first_move = !press.moved && (x - press.x).hypot(y - press.y) > SLOP;
        if first_move {
            press.moved = true;
            // Before the long-press it is a swipe, and the scroll is the
            // browser's.
            if let Some(timer) = press.timer.take()
                && let Some(window) = web_sys::window()
            {
                window.clear_timeout_with_handle(timer);
            }
        }
        press.long && press.moved
    };
    if !drawing {
        return;
    }
    // A touch is captured by the cell it began on, so the cell under the
    // finger is looked up, not read off the event.
    if let Some(cell) = root
        .element_from_point(x as f32, y as f32)
        .and_then(|target| range::cell_element(&target))
        .and_then(|td| range::cell_of(&td))
    {
        range::drag_over(&host, &runtime, cell, true);
    }
}

fn on_pointer_up(event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    let Ok(host) = root.host().dyn_into::<HtmlElement>() else {
        return;
    };
    if end_width_drag_on(&host, &root, pointer) || end_header_drag_on(&host, &root, pointer) {
        return;
    }
    let Some(runtime) = runtime(&host) else {
        return;
    };
    if is_mouse(pointer) {
        range::end_drag(&host, &runtime);
        if let Some(cell) = PENDING.with(std::cell::Cell::take) {
            open_menu_after(&host, cell);
        }
        if let Some(col) = PENDING_COLUMN.with(std::cell::Cell::take) {
            open_column_menu_after(&host, col);
        }
        return;
    }
    let Some(press) = end_press(&runtime) else {
        return;
    };
    if press.long {
        if press.moved {
            range::end_drag(&host, &runtime);
            focus_active(&host);
        } else {
            runtime.borrow_mut().drag_from = None;
            open_menu_after(&host, press.cell);
        }
        return;
    }
    if !press.moved {
        runtime.borrow_mut().tap = Some(Tap {
            cell: press.cell,
            focused: press.focused,
        });
    }
}

fn on_pointer_cancel(event: Event) {
    let Some(root) = current_shadow_root(&event) else {
        return;
    };
    if let Some(pointer) = event.dyn_ref::<PointerEvent>()
        && let Ok(host) = root.host().dyn_into::<HtmlElement>()
        && (end_width_drag_on(&host, &root, pointer) || end_header_drag_on(&host, &root, pointer))
    {
        return;
    }
    let Some(runtime) = root
        .host()
        .dyn_into::<HtmlElement>()
        .ok()
        .and_then(|host| runtime(&host))
    else {
        return;
    };
    end_press(&runtime);
    runtime.borrow_mut().drag_from = None;
}

/// Ends the press under way, its timer with it.
fn end_press(runtime: &Rc<RefCell<GridRuntime>>) -> Option<Press> {
    let press = runtime.borrow_mut().press.take()?;
    if let Some(timer) = press.timer
        && let Some(window) = web_sys::window()
    {
        window.clear_timeout_with_handle(timer);
    }
    Some(press)
}

fn on_touch_move(event: Event) {
    let Some(runtime) = current_shadow_root(&event)
        .and_then(|root| root.host().dyn_into::<HtmlElement>().ok())
        .and_then(|host| runtime(&host))
    else {
        return;
    };
    if runtime
        .borrow()
        .press
        .as_ref()
        .is_some_and(|press| press.long)
    {
        event.prevent_default();
    }
}

/// A tap on the cell that already had the focus opens its editor — in the
/// `click`, the last event of a tap, so nothing after it takes the focus back.
fn on_click(event: Event) {
    let Some((host, cell, _)) = range::data_cell(&event) else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    let tap = runtime.borrow_mut().tap.take();
    if tap.is_some_and(|tap| tap.cell == cell && tap.focused) {
        begin_edit(&host, cell, None);
    }
}

/// A double-click — or a double-tap, which some browsers report as one —
/// opens the editor. The second tap of a double-tap already did, on the
/// focused cell; the editor is then the target and this does nothing.
fn on_double_click(event: Event) {
    if autosize_on_edge(&event) {
        return;
    }
    let Some((host, cell, _)) = range::data_cell(&event) else {
        return;
    };
    if let Some(selection) =
        web_sys::window().and_then(|window| window.get_selection().ok().flatten())
    {
        let _ = selection.remove_all_ranges();
    }
    begin_edit(&host, cell, None);
}

/// A right click, Android's long-press and the context-menu key aimed at a
/// cell: the cell menu instead of the browser's.
fn on_context_menu(event: Event) {
    if on_header_context_menu(&event) {
        return;
    }
    let Some((host, cell, _)) = range::data_cell(&event) else {
        return;
    };
    let Some(runtime) = runtime(&host) else {
        return;
    };
    event.prevent_default();
    // Android's long-press sends one too: the press opens the menu as the
    // finger lifts.
    if runtime.borrow().press.is_some() || cell_menu::is_open_on(&host, cell) {
        return;
    }
    focus_cell(&host, &runtime, cell);
    // A right click whose button is still down (macOS, Linux) waits for it to
    // lift; the context-menu key and Windows, which sends it on release, do not.
    let held = event
        .dyn_ref::<web_sys::MouseEvent>()
        .is_some_and(|mouse| mouse.buttons() != 0);
    if held {
        PENDING.with(|pending| pending.set(Some(cell)));
    } else {
        cell_menu::open(&host, cell);
    }
}
