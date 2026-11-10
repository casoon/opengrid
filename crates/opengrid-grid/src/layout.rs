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

/// The share of the grid's width the pinned columns may take together, in
/// percent (issue #316). Above it the pin lets go and they scroll with the
/// rest, so the columns that scroll keep at least the other half — a grid
/// 320 px wide (WCAG 1.4.10) would otherwise show pinned columns and nothing
/// else. The setting stays, and holds again when there is room.
pub const PINNED_SHARE: u32 = 50;

/// Where a column is kept in place while the others scroll sideways
/// (issue #316): at the start of the row or at its end, by reading direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    Start,
    End,
}

impl Pin {
    /// The name in `set_columns` and in a view.
    pub fn as_str(self) -> &'static str {
        match self {
            Pin::Start => "start",
            Pin::End => "end",
        }
    }

    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "start" => Some(Pin::Start),
            "end" => Some(Pin::End),
            _ => None,
        }
    }
}

/// Whether the pinned columns, `pinned` pixels wide together, hold in a grid
/// `width` pixels wide ([`PINNED_SHARE`]).
pub fn pins_hold(pinned: f64, width: f64) -> bool {
    pinned * 100.0 <= width * f64::from(PINNED_SHARE)
}

/// The reader's column layout, on top of the `columns` attribute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnLayout {
    /// Columns the reader moved, in their new order. Names not in here keep
    /// their place from the attribute.
    order: Vec<String>,
    hidden: HashSet<String>,
    widths: HashMap<String, u32>,
    /// Where the reader pinned a column, or took a pin of the page's away
    /// (`None`). Names not in here are pinned as `set_columns` says. Kept
    /// sorted by column, so the same pins are the same layout and a view
    /// always writes them the same way.
    pins: Vec<(String, Option<Pin>)>,
}

impl ColumnLayout {
    /// The columns to select and render: the attribute's, reordered and
    /// filtered, the ones pinned at the start first and the ones pinned at the
    /// end last (issue #316) — each of the three runs in the order the columns
    /// had. `page` is where `set_columns` pins.
    ///
    /// A name in `order` that the attribute does not have is ignored — the
    /// attribute is the source of truth about which columns exist at all.
    pub fn effective(&self, declared: &[String], page: &[(String, Pin)]) -> Vec<String> {
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
        // One pass per run, so each keeps the order its columns had.
        let mut arranged = Vec::with_capacity(ordered.len());
        for run in [Some(Pin::Start), None, Some(Pin::End)] {
            arranged.extend(
                ordered
                    .iter()
                    .filter(|name| self.pin(name, page) == run)
                    .cloned(),
            );
        }
        arranged
    }

    /// Where a column is pinned: the reader's choice, or else the page's.
    pub fn pin(&self, name: &str, page: &[(String, Pin)]) -> Option<Pin> {
        match self.pins.iter().find(|(column, _)| column == name) {
            Some((_, choice)) => *choice,
            None => page
                .iter()
                .find(|(column, _)| column == name)
                .map(|(_, pin)| *pin),
        }
    }

    /// Pins a column, or lets it go, as the reader asked. A choice that is the
    /// page's own is not kept: the layout holds what the reader changed.
    /// Answers whether the column is pinned differently now.
    pub fn set_pin(&mut self, name: &str, pin: Option<Pin>, page: &[(String, Pin)]) -> bool {
        if self.pin(name, page) == pin {
            return false;
        }
        self.pins.retain(|(column, _)| column != name);
        if self.pin(name, page) != pin {
            self.restore_pin(name, pin);
        }
        true
    }

    /// The reader's pins, sorted by column like the widths; `None` is a pin of
    /// the page's the reader took away.
    pub fn pins(&self) -> &[(String, Option<Pin>)] {
        &self.pins
    }

    /// Sets a pin as a view holds it (restoring a view).
    pub fn restore_pin(&mut self, name: &str, pin: Option<Pin>) {
        self.pins.retain(|(column, _)| column != name);
        let at = self
            .pins
            .iter()
            .position(|(column, _)| column.as_str() > name)
            .unwrap_or(self.pins.len());
        self.pins.insert(at, (name.to_owned(), pin));
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
    /// ones. Answers the new order, or `None` when it would fall off an end —
    /// of the row, or of its run: a pinned column moves among the columns
    /// pinned at the same end, and one that is not pinned among the others.
    pub fn move_column(
        &mut self,
        declared: &[String],
        page: &[(String, Pin)],
        name: &str,
        by: i32,
    ) -> Option<Vec<String>> {
        let mut visible = self.effective(declared, page);
        let from = visible.iter().position(|column| column == name)?;
        let to = from.checked_add_signed(by as isize)?;
        if to >= visible.len() || self.pin(&visible[to], page) != self.pin(name, page) {
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
        assert_eq!(
            ColumnLayout::default().effective(&declared(), &[]),
            declared()
        );
    }

    /// A hidden column is not selected at all — the query does not ask for it.
    #[test]
    fn a_hidden_column_leaves_the_list() {
        let mut layout = ColumnLayout::default();
        assert!(layout.set_hidden("customer", true));
        assert_eq!(
            layout.effective(&declared(), &[]),
            ["id".to_owned(), "amount".to_owned(), "qty".to_owned()]
        );
        assert!(!layout.set_hidden("customer", true), "already hidden");
        assert!(layout.set_hidden("customer", false));
        assert_eq!(layout.effective(&declared(), &[]), declared());
    }

    /// Moving is among the **visible** columns: a hidden one is not a place a
    /// column can land on.
    #[test]
    fn a_column_moves_among_the_visible_ones() {
        let mut layout = ColumnLayout::default();
        layout.set_hidden("customer", true);
        let moved = layout
            .move_column(&declared(), &[], "id", 1)
            .expect("moved");
        assert_eq!(
            moved,
            ["amount".to_owned(), "id".to_owned(), "qty".to_owned()]
        );

        // Unhiding puts the column back without scrambling the arrangement.
        layout.set_hidden("customer", false);
        assert_eq!(
            layout.effective(&declared(), &[]),
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
        assert!(layout.move_column(&declared(), &[], "id", -1).is_none());
        assert!(layout.move_column(&declared(), &[], "qty", 1).is_none());
        assert_eq!(layout.effective(&declared(), &[]), declared());
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

    fn names(columns: &[&str]) -> Vec<String> {
        columns.iter().map(|name| (*name).to_owned()).collect()
    }

    /// Pinned at the start comes first and pinned at the end last, each run in
    /// the order the columns had (issue #316).
    #[test]
    fn pinned_columns_stand_at_their_end_of_the_row() {
        let mut layout = ColumnLayout::default();
        let page = [("qty".to_owned(), Pin::Start)];
        assert_eq!(
            layout.effective(&declared(), &page),
            names(&["qty", "id", "customer", "amount"])
        );
        assert!(layout.set_pin("id", Some(Pin::End), &page));
        assert!(layout.set_pin("amount", Some(Pin::Start), &page));
        assert_eq!(
            layout.effective(&declared(), &page),
            names(&["amount", "qty", "customer", "id"])
        );
        // A hidden column is not in the row, pinned or not.
        layout.set_hidden("qty", true);
        assert_eq!(
            layout.effective(&declared(), &page),
            names(&["amount", "customer", "id"])
        );
    }

    /// The reader's choice leads over the page's, and only what the reader
    /// changed is kept — a view holds a choice, not the page's configuration.
    #[test]
    fn the_reader_leads_and_only_a_change_is_kept() {
        let mut layout = ColumnLayout::default();
        let page = [("id".to_owned(), Pin::Start)];
        assert_eq!(layout.pin("id", &page), Some(Pin::Start));
        assert!(!layout.set_pin("id", Some(Pin::Start), &page), "as it is");
        assert!(layout.pins().is_empty());

        assert!(layout.set_pin("id", None, &page));
        assert_eq!(layout.pin("id", &page), None);
        assert_eq!(layout.pins(), [("id".to_owned(), None)]);

        // Back to what the page says: nothing of the reader's is left.
        assert!(layout.set_pin("id", Some(Pin::Start), &page));
        assert!(layout.pins().is_empty());

        assert!(layout.set_pin("qty", Some(Pin::End), &page));
        assert_eq!(layout.pins(), [("qty".to_owned(), Some(Pin::End))]);
        assert!(layout.set_pin("qty", None, &page));
        assert!(layout.pins().is_empty());

        // By column, in whatever order they were pinned: the same pins are the
        // same layout, and a view writes them the same way.
        let mut other = ColumnLayout::default();
        for name in ["qty", "amount"] {
            layout.set_pin(name, Some(Pin::End), &page);
        }
        for name in ["amount", "qty"] {
            other.set_pin(name, Some(Pin::End), &page);
        }
        assert_eq!(layout, other);
        assert_eq!(layout.pins()[0].0, "amount");
    }

    /// A column moves inside its run and stops at the run's end.
    #[test]
    fn a_column_moves_among_the_columns_pinned_like_it() {
        let mut layout = ColumnLayout::default();
        let page = [
            ("id".to_owned(), Pin::Start),
            ("customer".to_owned(), Pin::Start),
        ];
        // id, customer | amount, qty
        assert!(
            layout
                .move_column(&declared(), &page, "customer", 1)
                .is_none()
        );
        assert!(
            layout
                .move_column(&declared(), &page, "amount", -1)
                .is_none()
        );
        let moved = layout
            .move_column(&declared(), &page, "customer", -1)
            .expect("inside the run");
        assert_eq!(moved, names(&["customer", "id", "amount", "qty"]));
        assert_eq!(layout.effective(&declared(), &page), moved);
    }

    /// Half the width holds; a pixel more lets go.
    #[test]
    fn the_pins_hold_up_to_their_share() {
        assert!(pins_hold(160.0, 320.0));
        assert!(!pins_hold(161.0, 320.0));
    }
}
