//! The page's own items in the toolbar (issue #531): `set_toolbar` declares
//! buttons, menu buttons and disabled items, which the grid draws after its own
//! controls and keys like them; the `toolbar-end` slot (built with the toolbar)
//! takes whatever the grid cannot know. Choosing an item fires
//! `opengrid-toolbar-action` with `{ id, selection }`.
//!
//! A menu button opens a menu built when it opens, with the column menu's keys:
//! the arrows, `Home`/`End`, `Enter`/`Space`, `Escape` back to the button,
//! `Tab` on. The buttons are drawn into the toolbar's row by the element, as
//! the chips are, so a changed list does not rebuild the grid.

use super::*;
use crate::column_menu::Entry as MenuEntry;
use crate::grid_element_events::TOOLBAR_ACTION_EVENT;
use crate::toolbar::{self, Given, Item, RawEntry, RawItem};
use quick_doors::show_quick;

const MENU: &str = "[part=\"toolbar-menu\"]";
/// The prefix of a menu entry's action; its place in the menu follows.
const ENTRY: &str = "entry:";

thread_local! {
    static ITEMS: RefCell<HashMap<u32, Rc<Vec<Item>>>> = RefCell::new(HashMap::new());
}

fn release(id: u32) {
    ITEMS.with(|items| items.borrow_mut().remove(&id));
}

/// The page's items, checked: what `set_toolbar` last accepted.
fn items_of(host: &HtmlElement) -> Rc<Vec<Item>> {
    host::existing_id(host)
        .and_then(|id| ITEMS.with(|items| items.borrow().get(&id).cloned()))
        .unwrap_or_default()
}

fn store(host: &HtmlElement, items: Vec<Item>) {
    host::on_release(release);
    let id = host::id(host);
    ITEMS.with(|map| map.borrow_mut().insert(id, Rc::new(items)));
}

fn read_string(value: &JsValue, key: &str) -> Given<String> {
    match js_sys::Reflect::get(value, &JsValue::from_str(key)) {
        Ok(found) if !found.is_undefined() && !found.is_null() => {
            found.as_string().map_or(Given::Wrong, Given::Is)
        }
        _ => Given::Absent,
    }
}

fn read_bool(value: &JsValue, key: &str) -> Given<bool> {
    match js_sys::Reflect::get(value, &JsValue::from_str(key)) {
        Ok(found) if !found.is_undefined() && !found.is_null() => {
            found.as_bool().map_or(Given::Wrong, Given::Is)
        }
        _ => Given::Absent,
    }
}

/// The keys of an object that are not among `known`.
fn unknown_keys(value: &JsValue, known: &[&str]) -> Vec<String> {
    js_sys::Object::keys(value.unchecked_ref::<js_sys::Object>())
        .iter()
        .filter_map(|key| key.as_string())
        .filter(|key| !known.contains(&key.as_str()))
        .collect()
}

fn read_entry(value: &JsValue) -> Option<RawEntry> {
    if !value.is_object() {
        return None;
    }
    Some(RawEntry {
        id: read_string(value, "id"),
        label: read_string(value, "label"),
        unknown: unknown_keys(value, &["id", "label"]),
    })
}

fn read_item(value: &JsValue) -> Option<RawItem> {
    if !value.is_object() {
        return None;
    }
    let items = match js_sys::Reflect::get(value, &JsValue::from_str("items")) {
        Ok(found) if !found.is_undefined() && !found.is_null() => {
            match found.dyn_into::<js_sys::Array>() {
                Ok(list) => Given::Is(list.iter().map(|entry| read_entry(&entry)).collect()),
                Err(_) => Given::Wrong,
            }
        }
        _ => Given::Absent,
    };
    Some(RawItem {
        id: read_string(value, "id"),
        label: read_string(value, "label"),
        disabled: read_bool(value, "disabled"),
        reason: read_string(value, "reason"),
        items,
        unknown: unknown_keys(value, &["id", "label", "items", "disabled", "reason"]),
    })
}

/// `set_toolbar` (issue #531): checks the call, and draws it or — with a
/// problem — none of it, and says so in the status line.
pub(crate) fn set_toolbar(host: &HtmlElement, items: &JsValue) {
    let checked = if items.is_undefined() || items.is_null() {
        toolbar::validate(&[])
    } else if let Ok(list) = items.clone().dyn_into::<js_sys::Array>() {
        let raw: Vec<Option<RawItem>> = list.iter().map(|item| read_item(&item)).collect();
        toolbar::validate(&raw)
    } else {
        Err(vec![toolbar::Problem {
            subject: "items".to_owned(),
            reason: "is not a list".to_owned(),
        }])
    };
    match checked {
        Ok(items) => {
            store(host, items);
            fill(host);
        }
        Err(problems) => {
            let said = problems
                .iter()
                .map(toolbar::Problem::sentence)
                .collect::<Vec<_>>()
                .join(" ");
            announce(host, &texts(host).toolbar_refused(&said));
        }
    }
}

/// Draws the page's buttons into the toolbar's row, in front of the slot, and
/// leaves the focus on the button it was on.
pub(super) fn fill(host: &HtmlElement) {
    let (Some(root), Some(document)) = (host.shadow_root(), host.owner_document()) else {
        return;
    };
    let Ok(Some(slot)) = root.query_selector(&format!("slot[name=\"{}\"]", grid::TOOLBAR_SLOT))
    else {
        return;
    };
    let Some(row) = slot.parent_node() else {
        return;
    };
    close(host, false);
    let focused = root
        .active_element()
        .filter(|active| active.has_attribute("data-toolbar-item"))
        .and_then(|active| active.get_attribute("data-toolbar-item"));
    if let Ok(old) = root.query_selector_all("[data-toolbar-item]") {
        for index in 0..old.length() {
            if let Some(node) = old
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            {
                node.remove();
            }
        }
    }
    for (index, item) in items_of(host).iter().enumerate() {
        let Ok(button) = document.create_element("button") else {
            continue;
        };
        for (name, value) in [
            ("type", "button"),
            ("part", "toolbar-item"),
            ("data-toolbar", &format!("page-{index}")),
            ("data-toolbar-item", &item.id),
            // The page's word, not ours.
            ("lang", ""),
        ] {
            let _ = button.set_attribute(name, value);
        }
        if item.entries.is_some() {
            let _ = button.set_attribute("aria-haspopup", "menu");
            let _ = button.set_attribute("aria-expanded", "false");
        }
        if let Some(reason) = &item.disabled {
            let _ = button.set_attribute("aria-disabled", "true");
            let _ =
                button.set_attribute("aria-label", &crate::texts::reasoned(&item.label, reason));
        }
        button.set_text_content(Some(&item.label));
        let _ = row.insert_before(&button, Some(&slot));
        if focused.as_deref() == Some(item.id.as_str())
            && let Ok(button) = button.dyn_into::<HtmlElement>()
        {
            let _ = button.focus();
        }
    }
}

/// The place in the page's list of the item `button` draws.
fn index_of(host: &HtmlElement, button: &Element) -> Option<usize> {
    let id = button.get_attribute("data-toolbar-item")?;
    items_of(host).iter().position(|item| item.id == id)
}

/// A click on one of the page's buttons: a plain one fires the event, a menu
/// button opens or closes its menu, a disabled one says why.
pub(super) fn on_click(host: &HtmlElement, button: &Element) {
    let Some(index) = index_of(host, button) else {
        return;
    };
    let items = items_of(host);
    let item = &items[index];
    if let Some(reason) = &item.disabled {
        announce(host, reason);
    } else if item.entries.is_some() {
        if button.get_attribute("aria-expanded").as_deref() == Some("true") {
            close(host, true);
        } else {
            open(host, index, false);
        }
    } else {
        fire(host, &item.id);
    }
}

/// The arrows open a menu button's menu, as they open the filter row's operator
/// menu; `Enter` and `Space` are its native click. Answers whether it was one.
pub(super) fn on_button_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) -> bool {
    let Ok(Some(button)) = target.closest("[data-toolbar-item]") else {
        return false;
    };
    if !matches!(event.key().as_str(), "ArrowDown" | "ArrowUp")
        || button.get_attribute("aria-haspopup").is_none()
    {
        return false;
    }
    event.prevent_default();
    if let Some(index) = index_of(host, &button)
        && button.get_attribute("aria-disabled").as_deref() != Some("true")
    {
        open(host, index, event.key() == "ArrowUp");
    }
    true
}

/// Opens the menu of the item at `index`, the focus on its first entry — or
/// its last, with `last` (`ArrowUp` on the button).
fn open(host: &HtmlElement, index: usize, last: bool) {
    let (Some(root), Some(document)) = (host.shadow_root(), host.owner_document()) else {
        return;
    };
    let items = items_of(host);
    let Some(item) = items.get(index) else {
        return;
    };
    let Some(entries) = &item.entries else {
        return;
    };
    close(host, false);
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    let key = format!("page-{index}");
    for (name, value) in [
        ("part", "toolbar-menu"),
        ("role", "menu"),
        ("popover", "auto"),
        ("aria-label", item.label.as_str()),
        ("data-toolbar-for", key.as_str()),
        // The page's words, not ours.
        ("lang", ""),
    ] {
        let _ = menu.set_attribute(name, value);
    }
    let entries: Vec<MenuEntry> = entries
        .iter()
        .enumerate()
        .map(|(at, entry)| MenuEntry::Item {
            action: format!("{ENTRY}{at}"),
            label: entry.label.clone(),
            checked: None,
        })
        .collect();
    column_menu::append_entries(&document, &menu, &entries);
    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    show_quick(&root, &menu, &key);
    let selector = if last {
        "[role=\"menuitem\"]:last-child"
    } else {
        "[role=\"menuitem\"]"
    };
    if let Ok(Some(target)) = menu.query_selector(selector)
        && let Ok(target) = target.dyn_into::<HtmlElement>()
    {
        let _ = target.focus();
    }
}

/// Closes an open menu; with `refocus`, the focus goes back to its button.
pub(super) fn close(host: &HtmlElement, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(Some(menu)) = root.query_selector(MENU) else {
        return;
    };
    let key = menu.get_attribute("data-toolbar-for").unwrap_or_default();
    close_quick(host, "toolbar-menu", &key, refocus);
}

/// Whether `target` is in a page menu.
pub(super) fn contains(target: &Element) -> bool {
    target.closest(MENU).ok().flatten().is_some()
}

/// The keys inside the menu.
pub(super) fn on_menu_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(menu)) = target.closest(MENU) else {
        return;
    };
    let entries = menu_items(&menu, "[role=\"menuitem\"]");
    if entries.is_empty() {
        return;
    }
    match menu_key(event, &entries, target) {
        Some(MenuAct::Pick(at)) => activate(host, &entries[at]),
        Some(MenuAct::Escape | MenuAct::Tab) => close(host, true),
        None => {}
    }
}

/// Chooses an entry of the menu: the menu closes, the focus back on its
/// button, and the event follows.
pub(super) fn activate(host: &HtmlElement, entry: &Element) {
    let Some(at) = entry
        .get_attribute("data-action")
        .and_then(|action| action.strip_prefix(ENTRY)?.parse::<usize>().ok())
    else {
        return;
    };
    let Some(index) = entry
        .closest(MENU)
        .ok()
        .flatten()
        .and_then(|menu| menu.get_attribute("data-toolbar-for"))
        .and_then(|key| key.strip_prefix("page-")?.parse::<usize>().ok())
    else {
        return;
    };
    close(host, true);
    let items = items_of(host);
    if let Some(chosen) = items
        .get(index)
        .and_then(|item| item.entries.as_ref())
        .and_then(|entries| entries.get(at))
    {
        fire(host, &chosen.id);
    }
}

/// Fires `opengrid-toolbar-action`: `{ id, selection }`, `selection` what
/// `opengrid-selection-change` says now — the keys of a selection by key.
fn fire(host: &HtmlElement, id: &str) {
    let detail = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("id"), &JsValue::from_str(id));
    let _ = js_sys::Reflect::set(
        &detail,
        &JsValue::from_str("selection"),
        &selection_detail(host),
    );
    crate::element::dispatch(host, TOOLBAR_ACTION_EVENT, &detail);
}
