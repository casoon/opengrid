//! A source that changes under the grid (issue #328).
//!
//! The provider says so with a cancelable `opengrid-source-changed`. **Rows that
//! move under the focus are a barrier**, so the default is a notice, not a
//! reload: the status line says once, politely, that the data changed, and a
//! "Refresh" button sits above the rows. Nothing moves until the reader asks (or
//! the page calls `refresh`), and then the same query runs again with the view,
//! the focus (by record), the selection by key and the history kept. Another
//! change while the notice stands says nothing more.
//!
//! A page that wants its own way — refresh at once, a toast of its own —
//! calls `preventDefault()` on the event and `refresh(grid)` when it likes.

use super::*;

const BUTTON: &str = "[part=\"refresh\"]";

/// The provider said the source changed.
///
/// The grid's listener is the first on the element, the page's come after: the
/// notice waits for them (a microtask) to see whether one kept the default.
pub(super) fn on_changed(event: Event) {
    let Some(host) = event
        .current_target()
        .and_then(|target| target.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    let later = Closure::once(move |_: JsValue| {
        if !event.default_prevented() {
            notice(&host);
        }
    });
    let _ = js_sys::Promise::resolve(&JsValue::UNDEFINED).then(&later);
    later.forget();
}

fn notice(host: &HtmlElement) {
    let (Some(runtime), Some(root), Some(document)) =
        (runtime(host), host.shadow_root(), host.owner_document())
    else {
        return;
    };
    let host = host.clone();
    // Said once per batch: while the notice stands, more changes are the same one.
    if matches!(root.query_selector(BUTTON), Ok(Some(_))) {
        return;
    }
    let texts = texts(&host);
    let Ok(Some(viewport)) = root.query_selector("[part=\"viewport\"]") else {
        return;
    };
    let Some(layout) = viewport.parent_element() else {
        return;
    };
    let Ok(button) = document.create_element("button") else {
        return;
    };
    let _ = button.set_attribute("type", "button");
    let _ = button.set_attribute("part", "refresh");
    if !texts.lang.trim().is_empty() {
        let _ = button.set_attribute("lang", &texts.lang);
    }
    button.set_text_content(Some(&texts.source_refresh));
    let _ = layout.insert_before(&button, Some(&viewport));
    runtime
        .borrow_mut()
        .state
        .set_notice(texts.source_changed.clone());
    // The focused row is the one the renderer leaves alone, and nothing else
    // moves: only the status line is written.
    render(&host, false);
}

pub(super) fn is_button(target: &Element) -> bool {
    target.closest(BUTTON).ok().flatten().is_some()
}

/// Runs the query again over the changed source and keeps what the reader has:
/// the view, the focus (by record), the selection by key, the history.
pub(super) fn refresh(host: &HtmlElement) {
    // The focus is in the grid, or on the button that is about to go: either
    // way it goes back to the record it stood on.
    let focused_in_grid = host
        .shadow_root()
        .and_then(|root| root.active_element())
        .is_some_and(|element| {
            element.tag_name().eq_ignore_ascii_case("td") || is_button(&element)
        });
    if let Some(root) = host.shadow_root()
        && let Ok(Some(button)) = root.query_selector(BUTTON)
    {
        button.remove();
    }
    let Some(runtime) = runtime(host) else {
        return;
    };
    {
        let mut runtime = runtime.borrow_mut();
        let key = runtime
            .active
            .row()
            .and_then(|row| runtime.state.row_key(row).cloned());
        runtime.replaced = Some(key);
        // Without a key a selection names positions, and other records may sit
        // in them now; one by key stays.
        if !keyed(&runtime) {
            runtime.state.invalidate_selection();
        }
        runtime.groups_filter = None;
        if let Some(grouping) = runtime.grouping.as_mut() {
            grouping.invalidate();
        }
        if let Some(tree) = runtime.tree.as_mut() {
            tree.invalidate();
        }
        runtime.tree_asked = None;
        runtime.facet_domain.clear();
        runtime.facet_counts.clear();
        runtime.state.set_notice(String::new());
    }
    start_following(host, focused_in_grid);
}
