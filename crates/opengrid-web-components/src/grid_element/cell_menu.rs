//! The cell menu (issue #524): what a data cell offers — edit, copy, select its
//! row, open its record, its column's menu — and after those the page's own
//! items (`set_cell_actions`), which fire `opengrid-cell-action`.
//!
//! A long-press, a right click, `Shift`+`F10` and the context-menu key open it;
//! it is built when it opens and lists only what applies to that cell. The keys
//! inside are the column menu's (point 64): the arrows, `Home`/`End`,
//! `Enter`/`Space`, `Escape` back to the cell, `Tab` on.

use super::*;
use crate::column_menu::Entry;

/// Where `set_cell_actions` stores what the page supplied.
pub(crate) const CELL_ACTIONS_KEY: &str = "__opengridCellActions";

const MENU: &str = "[part=\"cell-menu\"]";
/// The prefix of a page item's action; its place in the page's list follows.
const PAGE: &str = "page:";

/// One item of the page's: `{ id, label, column? }`.
struct CellAction {
    id: String,
    label: String,
    /// The one column it is offered on; on every column without.
    column: Option<String>,
}

/// The page's items, in order. One without `id` or `label` is left out — an
/// item nobody can name is no item.
fn cell_actions_of(host: &HtmlElement) -> Vec<CellAction> {
    let Ok(list) = js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(CELL_ACTIONS_KEY)) else {
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
            Some(CellAction {
                id: text("id")?,
                label: text("label")?,
                column: text("column"),
            })
        })
        .collect()
}

/// Stores the page's items (issue #524). The menu is built when it opens, so
/// nothing is drawn now.
pub(crate) fn set_cell_actions(host: &HtmlElement, actions: &JsValue) {
    let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(CELL_ACTIONS_KEY), actions);
}

/// Whether the cell's editor would open: the same questions `begin_edit` asks,
/// without asking them out loud.
fn editable(host: &HtmlElement, runtime: &GridRuntime, cell: CellRef) -> bool {
    if edit_locked(host) {
        return false;
    }
    let Some(field) = runtime.state.schema().fields().get(cell.col) else {
        return false;
    };
    !readonly_column(host, field)
        && (!field.data_type.is_list() || combo::choices_of(host, cell.col).is_some())
        && (!is_grouped(host) || editable_group_row(host, ActiveCell::Data(cell)))
}

/// Whether a row can be selected from here: a record, in a grid that selects
/// records — grouped, only by key.
fn selectable(host: &HtmlElement, runtime: &GridRuntime, row: u64) -> bool {
    is_record(runtime, row)
        && runtime.state.row_kind(row) == RowKind::Source
        && (is_tree(host) || !is_grouped(host) || keyed(runtime))
}

/// Whether the row at `row` says it is selected.
fn row_selected(root: &ShadowRoot, row: u64) -> bool {
    root.query_selector(&format!("td[data-row=\"{row}\"]"))
        .ok()
        .flatten()
        .and_then(|td| td.closest("tr").ok().flatten())
        .and_then(|tr| tr.get_attribute("aria-selected"))
        .as_deref()
        == Some("true")
}

/// What the menu of `cell` lists: the grid's entries that apply, then the
/// page's for its column.
fn entries(host: &HtmlElement, root: &ShadowRoot, cell: CellRef) -> Option<(String, Vec<Entry>)> {
    let runtime = runtime(host)?;
    let borrowed = runtime.borrow();
    let name = borrowed
        .state
        .schema()
        .fields()
        .get(cell.col)?
        .name
        .as_str()
        .to_owned();
    let texts = texts(host);
    let item = |action: &str, label: &str| Entry::Item {
        action: action.to_owned(),
        label: label.to_owned(),
        checked: None,
    };
    let mut entries = Vec::new();
    if editable(host, &borrowed, cell) {
        entries.push(item("edit", &texts.cell_edit));
    }
    entries.push(item("copy", &texts.cell_copy));
    if selectable(host, &borrowed, cell.row) {
        let label = if row_selected(root, cell.row) {
            &texts.cell_deselect_row
        } else {
            &texts.cell_select_row
        };
        entries.push(item("select", label));
    }
    if !edit_locked(host) && record::applicable(host, &borrowed, cell) {
        entries.push(item("record", &texts.cell_open_record));
    }
    if host.has_attribute(grid::COLUMN_MENU_ATTRIBUTE) {
        entries.push(item("column", &texts.cell_column_menu));
    }
    let page: Vec<Entry> = if is_record(&borrowed, cell.row) {
        cell_actions_of(host)
            .into_iter()
            .enumerate()
            .filter(|(_, action)| action.column.as_ref().is_none_or(|column| *column == name))
            .map(|(index, action)| item(&format!("{PAGE}{index}"), &action.label))
            .collect()
    } else {
        Vec::new()
    };
    if !page.is_empty() {
        entries.push(Entry::Separator);
        entries.extend(page);
    }
    Some((name, entries))
}

/// Opens the menu of `cell`, the focus on its first entry.
pub(super) fn open(host: &HtmlElement, cell: CellRef) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    close(host, false);
    let Some((name, entries)) = entries(host, &root, cell) else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    let texts = texts(host);
    for (attribute, value) in [
        ("part", "cell-menu"),
        ("role", "menu"),
        ("popover", "auto"),
        ("data-row", &cell.row.to_string()),
        ("data-col", &cell.col.to_string()),
    ] {
        let _ = menu.set_attribute(attribute, value);
    }
    let _ = menu.set_attribute("aria-label", &texts.cell_menu(&name));
    if !texts.lang.trim().is_empty() {
        let _ = menu.set_attribute("lang", &texts.lang);
    }
    column_menu::append_entries(&document, &menu, &entries);
    let _ = root.append_child(&menu);
    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = menu.show_popover();
    place(&root, &menu, cell);
    if let Ok(Some(first)) = menu.query_selector("[role^=\"menuitem\"]")
        && let Ok(first) = first.dyn_into::<HtmlElement>()
    {
        let _ = first.focus();
    }
}

/// Puts the menu under its cell — never over it (WCAG 2.2 §2.4.11), as the
/// column menu does — or above it where there is no room below.
fn place(root: &ShadowRoot, menu: &HtmlElement, cell: CellRef) {
    let Ok(Some(td)) = root.query_selector(&format!(
        "td[data-row=\"{}\"][data-col=\"{}\"]",
        cell.row, cell.col
    )) else {
        return;
    };
    let rect = td.get_bounding_client_rect();
    let own = menu.get_bounding_client_rect();
    let height = web_sys::window()
        .and_then(|window| window.inner_height().ok())
        .and_then(|value| value.as_f64())
        .unwrap_or(768.0);
    const GAP: f64 = 4.0;
    let left = crate::element::popup_left(&rect, &own, crate::element::is_rtl(&td));
    let below = height - rect.bottom() - 2.0 * GAP;
    let above = rect.top() - 2.0 * GAP;
    // Below where it fits; else above where it fits; else on the roomier side,
    // scrolling — a phone held sideways has room for neither.
    let top = if own.height() <= below || below >= above {
        rect.bottom() + GAP
    } else {
        (rect.top() - own.height().min(above) - GAP).max(GAP)
    };
    if own.height() > below.max(above) {
        let room = below.max(above);
        let _ = menu
            .style()
            .set_property("max-height", &format!("{room}px"));
        let _ = menu.style().set_property("overflow-y", "auto");
    }
    let _ = menu.style().set_property("left", &format!("{left}px"));
    let _ = menu.style().set_property("top", &format!("{top}px"));
}

/// Whether a menu of `cell` is open.
pub(super) fn is_open_on(host: &HtmlElement, cell: CellRef) -> bool {
    host.shadow_root()
        .and_then(|root| root.query_selector(MENU).ok().flatten())
        .and_then(|menu| menu.dyn_into::<HtmlElement>().ok())
        .is_some_and(|menu| {
            menu.matches(":popover-open").unwrap_or(false) && menu_cell(&menu) == Some(cell)
        })
}

fn menu_cell(menu: &Element) -> Option<CellRef> {
    Some(CellRef::new(
        menu.get_attribute("data-row")?.parse().ok()?,
        menu.get_attribute("data-col")?.parse().ok()?,
    ))
}

/// Closes an open menu; with `refocus`, the focus goes back to the grid's
/// active cell — its cell, or the edge of the block it was opened over.
pub(super) fn close(host: &HtmlElement, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(menus) = root.query_selector_all(MENU) else {
        return;
    };
    let mut had = false;
    for index in 0..menus.length() {
        let Some(menu) = menus
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        else {
            continue;
        };
        had = true;
        let _ = menu.hide_popover();
        menu.remove();
    }
    if refocus && had {
        focus_active(host);
    }
}

/// Whether `target` is in the cell menu.
pub(super) fn contains(target: &Element) -> bool {
    target.closest(MENU).ok().flatten().is_some()
}

/// The keys inside the menu.
pub(super) fn on_menu_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    let Ok(Some(menu)) = target.closest(MENU) else {
        return;
    };
    let items = crate::element::menu_items(&menu, "[role^=\"menuitem\"]");
    if items.is_empty() {
        return;
    }
    match crate::element::menu_key(event, &items, target) {
        Some(crate::element::MenuAct::Pick(at)) => activate(host, &items[at]),
        Some(crate::element::MenuAct::Escape | crate::element::MenuAct::Tab) => close(host, true),
        None => {}
    }
}

/// Does what an entry says, after closing the menu.
pub(super) fn activate(host: &HtmlElement, item: &Element) {
    let Some(action) = item.get_attribute("data-action") else {
        return;
    };
    let Some(cell) = item
        .closest(MENU)
        .ok()
        .flatten()
        .and_then(|menu| menu_cell(&menu))
    else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    // The column menu opens under its header and takes the focus there.
    close(host, action != "column");
    match action.as_str() {
        "edit" => begin_edit(host, cell, None),
        "copy" => {
            let block = runtime
                .borrow()
                .range
                .filter(|range| range.contains(cell))
                .unwrap_or_else(|| opengrid_grid::CellRange::new(cell, cell));
            if let Some(text) = range::copy_text(host, &runtime, block) {
                range::write_clipboard(&text);
            }
        }
        "select" => toggle_row(host, &runtime, cell.row, false),
        "record" => record::open(host, cell),
        "column" => open_column_menu(host, cell.col),
        page if page.starts_with(PAGE) => {
            if let Ok(index) = page[PAGE.len()..].parse::<usize>() {
                fire(host, &runtime, cell, index);
            }
        }
        _ => {}
    }
}

/// Fires `opengrid-cell-action` for the page's item at `index` on `cell`:
/// `{ id, row, column, key }`, `key` the record's `row-key` value.
fn fire(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, cell: CellRef, index: usize) {
    let Some(action) = cell_actions_of(host).into_iter().nth(index) else {
        return;
    };
    let (key, column) = {
        let borrowed = runtime.borrow();
        if !is_record(&borrowed, cell.row) {
            return;
        }
        let Some(column) = borrowed
            .state
            .schema()
            .fields()
            .get(cell.col)
            .map(|field| field.name.as_str().to_owned())
        else {
            return;
        };
        (row_key_js(host, &borrowed.state, cell.row), column)
    };
    let detail = js_sys::Object::new();
    if let Some(key) = key {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("key"), &key);
    }
    for (name, value) in [
        ("id", JsValue::from_str(&action.id)),
        ("row", JsValue::from_f64(cell.row as f64)),
        ("column", JsValue::from_str(&column)),
    ] {
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(name), &value);
    }
    crate::element::dispatch(host, CELL_ACTION_EVENT, &detail);
}
