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
}

/// Opens the menu of `cell` once the event that lifted the pointer is done.
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
    let Some(runtime) = runtime(&host) else {
        return;
    };
    if is_mouse(pointer) {
        range::end_drag(&host, &runtime);
        if let Some(cell) = PENDING.with(std::cell::Cell::take) {
            open_menu_after(&host, cell);
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
