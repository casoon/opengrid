//! What the reader did to the columns: width, order, visibility (plan point 36).
//!
//! # Why this is not in `GridState`
//!
//! The layout decides **which query is sent** — a hidden column is not selected,
//! a reordered one is selected in a different order. Keeping it beside the
//! `columns` attribute rather than inside the grid state means the query, the
//! skeleton and the render all read the same list and cannot disagree; the state
//! keeps holding what a *result* said.
//!
//! # The hard part is the keyboard, not the layout
//!
//! WCAG 2.2 **2.5.7 Dragging Movements** (AA) requires that anything done by
//! dragging can also be done without. A column handle you can only grab with a
//! mouse fails it. So the keyboard protocol comes first and dragging, if it ever
//! arrives, is the addition.

use std::collections::{HashMap, HashSet};

/// The narrowest a key press will take a column, in pixels.
///
/// A column resized into nothing is a column nobody finds again, and its header
/// would fall under the 24 px of WCAG 2.5.8.
const MIN_COLUMN_WIDTH: u32 = 48;
/// The widest a key press will take a column.
const MAX_COLUMN_WIDTH: u32 = 800;
/// How much one resize key press changes the width.
pub const WIDTH_STEP: u32 = 24;

/// The reader's column layout, on top of the `columns` attribute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnLayout {
    /// Columns the reader moved, in their new order. Names not in here keep
    /// their place from the attribute.
    order: Vec<String>,
    hidden: HashSet<String>,
    widths: HashMap<String, u32>,
}

impl ColumnLayout {
    /// The columns to select and render: the attribute's, reordered and
    /// filtered.
    ///
    /// A name in `order` that the attribute does not have is ignored — the
    /// attribute is the source of truth about which columns exist at all.
    pub fn effective(&self, declared: &[String]) -> Vec<String> {
        let mut ordered: Vec<String> = self
            .order
            .iter()
            .filter(|name| declared.contains(name))
            .cloned()
            .collect();
        for name in declared {
            if !ordered.contains(name) {
                ordered.push(name.clone());
            }
        }
        ordered.retain(|name| !self.hidden.contains(name));
        ordered
    }

    /// Whether a column is hidden.
    pub fn is_hidden(&self, name: &str) -> bool {
        self.hidden.contains(name)
    }

    /// Hides or shows a column. Answers whether anything changed.
    pub fn set_hidden(&mut self, name: &str, hidden: bool) -> bool {
        if hidden {
            self.hidden.insert(name.to_owned())
        } else {
            self.hidden.remove(name)
        }
    }

    /// The width a column was given, if any.
    pub fn width(&self, name: &str) -> Option<u32> {
        self.widths.get(name).copied()
    }

    /// Changes a column's width by `step` pixels, clamped.
    ///
    /// The clamp is what keeps a column from being resized into nothing: a
    /// column of zero width is a column nobody can find again, and its header
    /// would fall under the 24 px of WCAG 2.5.8.
    pub fn resize(&mut self, name: &str, step: i32, current: u32) -> u32 {
        let base = self.widths.get(name).copied().unwrap_or(current);
        let next = (base as i32 + step).clamp(MIN_COLUMN_WIDTH as i32, MAX_COLUMN_WIDTH as i32);
        let next = next as u32;
        self.widths.insert(name.to_owned(), next);
        next
    }

    // ---------------------------------------------------------------------
    // Reading and writing the layout as a whole (point 59)
    // ---------------------------------------------------------------------
    //
    // A view is the reader's layout plus the rest of what they chose, so the
    // layout has to be sayable in one piece and settable in one piece. The
    // per-column setters above stay: they are what a key press uses, and they
    // announce.

    /// The order the reader arranged, if they arranged one.
    pub fn order(&self) -> &[String] {
        &self.order
    }

    /// Replaces the order wholesale (restoring a view).
    pub fn set_order(&mut self, order: Vec<String>) {
        self.order = order;
    }

    /// The hidden columns, sorted so the same layout always says the same
    /// thing — a `HashSet` iterates in whatever order it likes, and a view that
    /// serialised differently each time would look changed when it was not.
    pub fn hidden(&self) -> Vec<String> {
        let mut hidden: Vec<String> = self.hidden.iter().cloned().collect();
        hidden.sort_unstable();
        hidden
    }

    /// The widths the reader set, sorted by column for the same reason.
    pub fn widths(&self) -> Vec<(String, u32)> {
        let mut widths: Vec<(String, u32)> = self
            .widths
            .iter()
            .map(|(name, width)| (name.clone(), *width))
            .collect();
        widths.sort_unstable();
        widths
    }

    /// Sets a width directly (restoring a view), clamped like a key press.
    pub fn set_width(&mut self, name: &str, width: u32) {
        self.widths.insert(
            name.to_owned(),
            width.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH),
        );
    }

    /// Moves a column one place left (`-1`) or right (`1`) among the visible
    /// ones. Answers the new order, or `None` when it would fall off an end.
    pub fn move_column(&mut self, declared: &[String], name: &str, by: i32) -> Option<Vec<String>> {
        let mut visible = self.effective(declared);
        let from = visible.iter().position(|column| column == name)?;
        let to = from.checked_add_signed(by as isize)?;
        if to >= visible.len() {
            return None;
        }
        visible.swap(from, to);
        // Hidden columns keep their relative place at the end, so unhiding one
        // does not scramble what the reader just arranged.
        let mut order = visible.clone();
        for column in declared {
            if !order.contains(column) {
                order.push(column.clone());
            }
        }
        self.order = order;
        Some(visible)
    }
}

#[cfg(target_arch = "wasm32")]
pub use host::{layout, update};

/// Per-host storage, mirroring the texts and formats seams.
#[cfg(target_arch = "wasm32")]
mod host {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use wasm_bindgen::JsValue;
    use web_sys::HtmlElement;

    use super::ColumnLayout;

    thread_local! {
        static NEXT_ID: RefCell<u32> = const { RefCell::new(1) };
        static LAYOUTS: RefCell<HashMap<u32, Rc<RefCell<ColumnLayout>>>> =
            RefCell::new(HashMap::new());
    }

    fn id_symbol() -> js_sys::Symbol {
        js_sys::Symbol::for_("opengrid.columns_id")
    }

    /// The layout of `host`, creating an empty one on first use.
    pub fn layout(host: &HtmlElement) -> Rc<RefCell<ColumnLayout>> {
        let id = js_sys::Reflect::get(host.as_ref(), id_symbol().as_ref())
            .ok()
            .and_then(|value| value.as_f64())
            .map(|id| id as u32)
            .unwrap_or_else(|| {
                let id = NEXT_ID.with(|next| {
                    let mut next = next.borrow_mut();
                    let id = *next;
                    *next += 1;
                    id
                });
                let _ = js_sys::Reflect::set(
                    host.as_ref(),
                    id_symbol().as_ref(),
                    &JsValue::from_f64(f64::from(id)),
                );
                id
            });
        LAYOUTS.with(|map| {
            map.borrow_mut()
                .entry(id)
                .or_insert_with(|| Rc::new(RefCell::new(ColumnLayout::default())))
                .clone()
        })
    }

    /// Changes the layout of `host` and answers whether anything changed.
    pub fn update(host: &HtmlElement, change: impl FnOnce(&mut ColumnLayout) -> bool) -> bool {
        let layout = layout(host);
        let mut layout = layout.borrow_mut();
        change(&mut layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> Vec<String> {
        ["id", "customer", "amount", "qty"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// Without any change the attribute decides, in its own order.
    #[test]
    fn an_untouched_layout_is_the_attribute() {
        assert_eq!(ColumnLayout::default().effective(&declared()), declared());
    }

    /// A hidden column is not selected at all — the query does not ask for it.
    #[test]
    fn a_hidden_column_leaves_the_list() {
        let mut layout = ColumnLayout::default();
        assert!(layout.set_hidden("customer", true));
        assert_eq!(
            layout.effective(&declared()),
            ["id".to_owned(), "amount".to_owned(), "qty".to_owned()]
        );
        assert!(!layout.set_hidden("customer", true), "already hidden");
        assert!(layout.set_hidden("customer", false));
        assert_eq!(layout.effective(&declared()), declared());
    }

    /// Moving is among the **visible** columns: a hidden one is not a place a
    /// column can land on.
    #[test]
    fn a_column_moves_among_the_visible_ones() {
        let mut layout = ColumnLayout::default();
        layout.set_hidden("customer", true);
        let moved = layout.move_column(&declared(), "id", 1).expect("moved");
        assert_eq!(
            moved,
            ["amount".to_owned(), "id".to_owned(), "qty".to_owned()]
        );

        // Unhiding puts the column back without scrambling the arrangement.
        layout.set_hidden("customer", false);
        assert_eq!(
            layout.effective(&declared()),
            [
                "amount".to_owned(),
                "id".to_owned(),
                "qty".to_owned(),
                "customer".to_owned()
            ]
        );
    }

    /// At either end the move does nothing and says so, instead of wrapping —
    /// a column that jumps from one end to the other is a column the reader
    /// has to hunt for.
    #[test]
    fn a_move_off_the_end_is_refused() {
        let mut layout = ColumnLayout::default();
        assert!(layout.move_column(&declared(), "id", -1).is_none());
        assert!(layout.move_column(&declared(), "qty", 1).is_none());
        assert_eq!(layout.effective(&declared()), declared());
    }

    /// A width is clamped: not into nothing, not off the screen.
    #[test]
    fn a_width_stays_usable() {
        let mut layout = ColumnLayout::default();
        for _ in 0..50 {
            layout.resize("id", -(WIDTH_STEP as i32), 120);
        }
        assert_eq!(layout.width("id"), Some(MIN_COLUMN_WIDTH));
        for _ in 0..100 {
            layout.resize("id", WIDTH_STEP as i32, 120);
        }
        assert_eq!(layout.width("id"), Some(MAX_COLUMN_WIDTH));
    }
}
