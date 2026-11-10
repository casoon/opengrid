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

/// What the page's `set_columns` says about where the columns stand: where it
/// pins (issue #316) and which columns share a band (issue #319).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageColumns {
    /// The columns the page pins, each with its end of the row.
    pub pins: Vec<(String, Pin)>,
    /// The columns the page puts under a band, each with the band's name.
    pub bands: Vec<(String, String)>,
}

impl PageColumns {
    /// The band a column stands under, if any.
    pub fn band(&self, name: &str) -> Option<&str> {
        self.bands
            .iter()
            .find(|(column, _)| column == name)
            .map(|(_, band)| band.as_str())
    }

    /// Whether two columns stand and are pinned together: the same column, or
    /// two of one band.
    fn together(&self, one: &str, other: &str) -> bool {
        one == other || (self.band(one).is_some() && self.band(one) == self.band(other))
    }

    /// The bands above `shown`, the columns as [`ColumnLayout::effective`]
    /// arranged them: each with the index of its first column and how many
    /// columns it spans.
    pub fn band_spans(&self, shown: &[String]) -> Vec<(String, usize, usize)> {
        let mut spans: Vec<(String, usize, usize)> = Vec::new();
        for (col, name) in shown.iter().enumerate() {
            let Some(band) = self.band(name) else {
                continue;
            };
            match spans.last_mut() {
                Some((last, first, count)) if last == band && *first + *count == col => *count += 1,
                _ => spans.push((band.to_owned(), col, 1)),
            }
        }
        spans
    }
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
    /// had. `page` is where `set_columns` pins, and its bands.
    ///
    /// A band is always in one piece (issue #319): its columns stand together
    /// where the first of them stands, in the order they had — whatever the
    /// attribute or a view's order says.
    ///
    /// A name in `order` that the attribute does not have is ignored — the
    /// attribute is the source of truth about which columns exist at all.
    pub fn effective(&self, declared: &[String], page: &PageColumns) -> Vec<String> {
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
        if !page.bands.is_empty() {
            let mut gathered: Vec<String> = Vec::with_capacity(ordered.len());
            for name in &ordered {
                if !gathered.contains(name) {
                    gathered.extend(
                        ordered
                            .iter()
                            .filter(|column| page.together(name, column))
                            .cloned(),
                    );
                }
            }
            ordered = gathered;
        }
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
    ///
    /// A band is pinned as a whole (issue #319), so a column of one is pinned
    /// where any column of its band is: the reader's choice for one of them,
    /// or else the first pin `set_columns` gives one of them.
    pub fn pin(&self, name: &str, page: &PageColumns) -> Option<Pin> {
        match self
            .pins
            .iter()
            .find(|(column, _)| page.together(name, column))
        {
            Some((_, choice)) => *choice,
            None => page
                .pins
                .iter()
                .find(|(column, _)| page.together(name, column))
                .map(|(_, pin)| *pin),
        }
    }

    /// Pins a column, or lets it go, as the reader asked. A choice that is the
    /// page's own is not kept: the layout holds what the reader changed.
    /// Answers whether the column is pinned differently now.
    ///
    /// A column of a band takes its band along (issue #319): the choice is
    /// kept for every column of the band, so a view says it of each.
    pub fn set_pin(&mut self, name: &str, pin: Option<Pin>, page: &PageColumns) -> bool {
        if self.pin(name, page) == pin {
            return false;
        }
        self.pins.retain(|(column, _)| !page.together(name, column));
        if self.pin(name, page) != pin {
            self.restore_pin(name, pin);
            for (column, _) in &page.bands {
                if column != name && page.together(name, column) {
                    self.restore_pin(column, pin);
                }
            }
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
    ///
    /// A column of a band moves inside its band and stops at the band's edge
    /// (issue #319). A column without one steps over a band as over one
    /// column: it never lands inside.
    pub fn move_column(
        &mut self,
        declared: &[String],
        page: &PageColumns,
        name: &str,
        by: i32,
    ) -> Option<Vec<String>> {
        let mut visible = self.effective(declared, page);
        let from = visible.iter().position(|column| column == name)?;
        if page.band(name).is_some() {
            let to = from.checked_add_signed(by as isize)?;
            if to >= visible.len() || !page.together(name, &visible[to]) {
                return None;
            }
            visible.swap(from, to);
        } else {
            self.move_run(&mut visible, page, from, 1, by)?;
        }
        Some(self.arranged(declared, visible))
    }

    /// Moves a whole band one place left (`-1`) or right (`1`): past the
    /// column or the band beside it (issue #319). Answers the new order, or
    /// `None` at an end of the row or of the band's run of pinned columns —
    /// or when no column of `band` is shown.
    pub fn move_band(
        &mut self,
        declared: &[String],
        page: &PageColumns,
        band: &str,
        by: i32,
    ) -> Option<Vec<String>> {
        let mut visible = self.effective(declared, page);
        let (_, first, count) = page
            .band_spans(&visible)
            .into_iter()
            .find(|(name, _, _)| name == band)?;
        self.move_run(&mut visible, page, first, count, by)?;
        Some(self.arranged(declared, visible))
    }

    /// Moves the `count` columns from `first` past what stands beside them —
    /// a column, or a band as a whole — when that is pinned like them.
    fn move_run(
        &self,
        visible: &mut [String],
        page: &PageColumns,
        first: usize,
        count: usize,
        by: i32,
    ) -> Option<()> {
        let beside = if by < 0 {
            first.checked_sub(1)?
        } else {
            first + count
        };
        let neighbour = visible.get(beside)?;
        if self.pin(neighbour, page) != self.pin(&visible[first], page) {
            return None;
        }
        // How many columns stand beside: one, or the neighbour's whole band.
        let width = visible
            .iter()
            .filter(|column| page.together(neighbour, column))
            .count();
        if by < 0 {
            visible[first - width..first + count].rotate_left(width);
        } else {
            visible[first..first + count + width].rotate_left(count);
        }
        Some(())
    }

    /// Keeps `visible` as the reader's order and answers it.
    fn arranged(&mut self, declared: &[String], visible: Vec<String>) -> Vec<String> {
        // Hidden columns keep their relative place at the end, so unhiding one
        // does not scramble what the reader just arranged.
        let mut order = visible.clone();
        for column in declared {
            if !order.contains(column) {
                order.push(column.clone());
            }
        }
        self.order = order;
        visible
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
            ColumnLayout::default().effective(&declared(), &PageColumns::default()),
            declared()
        );
    }

    /// A hidden column is not selected at all — the query does not ask for it.
    #[test]
    fn a_hidden_column_leaves_the_list() {
        let mut layout = ColumnLayout::default();
        assert!(layout.set_hidden("customer", true));
        assert_eq!(
            layout.effective(&declared(), &PageColumns::default()),
            ["id".to_owned(), "amount".to_owned(), "qty".to_owned()]
        );
        assert!(!layout.set_hidden("customer", true), "already hidden");
        assert!(layout.set_hidden("customer", false));
        assert_eq!(
            layout.effective(&declared(), &PageColumns::default()),
            declared()
        );
    }

    /// Moving is among the **visible** columns: a hidden one is not a place a
    /// column can land on.
    #[test]
    fn a_column_moves_among_the_visible_ones() {
        let mut layout = ColumnLayout::default();
        layout.set_hidden("customer", true);
        let moved = layout
            .move_column(&declared(), &PageColumns::default(), "id", 1)
            .expect("moved");
        assert_eq!(
            moved,
            ["amount".to_owned(), "id".to_owned(), "qty".to_owned()]
        );

        // Unhiding puts the column back without scrambling the arrangement.
        layout.set_hidden("customer", false);
        assert_eq!(
            layout.effective(&declared(), &PageColumns::default()),
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
        assert!(
            layout
                .move_column(&declared(), &PageColumns::default(), "id", -1)
                .is_none()
        );
        assert!(
            layout
                .move_column(&declared(), &PageColumns::default(), "qty", 1)
                .is_none()
        );
        assert_eq!(
            layout.effective(&declared(), &PageColumns::default()),
            declared()
        );
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

    fn pins(pins: &[(&str, Pin)]) -> PageColumns {
        PageColumns {
            pins: pins
                .iter()
                .map(|(name, pin)| ((*name).to_owned(), *pin))
                .collect(),
            bands: Vec::new(),
        }
    }

    /// `id`, then the band `Address` over `street` and `city`, then `amount`.
    fn banded() -> (Vec<String>, PageColumns) {
        let page = PageColumns {
            pins: Vec::new(),
            bands: vec![
                ("street".to_owned(), "Address".to_owned()),
                ("city".to_owned(), "Address".to_owned()),
            ],
        };
        (names(&["id", "street", "city", "amount"]), page)
    }

    /// Pinned at the start comes first and pinned at the end last, each run in
    /// the order the columns had (issue #316).
    #[test]
    fn pinned_columns_stand_at_their_end_of_the_row() {
        let mut layout = ColumnLayout::default();
        let page = pins(&[("qty", Pin::Start)]);
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
        let page = pins(&[("id", Pin::Start)]);
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
        let page = pins(&[("id", Pin::Start), ("customer", Pin::Start)]);
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

    /// A band is in one piece (issue #319): its columns stand where the first
    /// of them stands, whatever the attribute or a view's order says, and the
    /// spans name each band once.
    #[test]
    fn a_band_is_always_in_one_piece() {
        let (_, page) = banded();
        let apart = names(&["street", "id", "city", "amount"]);
        let layout = ColumnLayout::default();
        let shown = layout.effective(&apart, &page);
        assert_eq!(shown, names(&["street", "city", "id", "amount"]));
        assert_eq!(page.band_spans(&shown), [("Address".to_owned(), 0, 2)]);

        let mut layout = ColumnLayout::default();
        layout.set_order(names(&["city", "amount", "street", "id"]));
        assert_eq!(
            layout.effective(&apart, &page),
            names(&["city", "street", "amount", "id"])
        );

        // The last shown column of a band takes the band along.
        layout.set_hidden("city", true);
        let shown = layout.effective(&apart, &page);
        assert_eq!(shown, names(&["amount", "street", "id"]));
        assert_eq!(page.band_spans(&shown), [("Address".to_owned(), 1, 1)]);
        layout.set_hidden("street", true);
        let shown = layout.effective(&apart, &page);
        assert_eq!(shown, names(&["amount", "id"]));
        assert!(page.band_spans(&shown).is_empty());
    }

    /// A column of a band moves inside the band and stops at its edge; a
    /// column without one steps over the band as over one column.
    #[test]
    fn a_column_moves_inside_its_band_and_others_step_over_it() {
        let (declared, page) = banded();
        let mut layout = ColumnLayout::default();
        assert!(layout.move_column(&declared, &page, "street", -1).is_none());
        assert!(layout.move_column(&declared, &page, "city", 1).is_none());
        assert_eq!(
            layout.move_column(&declared, &page, "street", 1),
            Some(names(&["id", "city", "street", "amount"]))
        );

        assert_eq!(
            layout.move_column(&declared, &page, "id", 1),
            Some(names(&["city", "street", "id", "amount"]))
        );
        assert_eq!(
            layout.move_column(&declared, &page, "amount", -1),
            Some(names(&["city", "street", "amount", "id"]))
        );
        assert_eq!(
            layout.move_column(&declared, &page, "amount", -1),
            Some(names(&["amount", "city", "street", "id"]))
        );
        assert!(layout.move_column(&declared, &page, "amount", -1).is_none());
        assert_eq!(layout.effective(&declared, &page), layout.order());
    }

    /// A band moves as a whole, past a column or past another band, and stops
    /// at the end of the row and at the edge of the pinned columns.
    #[test]
    fn a_band_moves_as_a_whole() {
        let (declared, mut page) = banded();
        let mut layout = ColumnLayout::default();
        assert_eq!(
            layout.move_band(&declared, &page, "Address", 1),
            Some(names(&["id", "amount", "street", "city"]))
        );
        assert!(layout.move_band(&declared, &page, "Address", 1).is_none());
        assert_eq!(
            layout.move_band(&declared, &page, "Address", -1),
            Some(names(&["id", "street", "city", "amount"]))
        );
        assert!(layout.move_band(&declared, &page, "Nope", 1).is_none());

        // Past another band: the two change places, each in its own order.
        page.bands.push(("id".to_owned(), "Key".to_owned()));
        page.bands.push(("amount".to_owned(), "Key".to_owned()));
        let mut layout = ColumnLayout::default();
        assert_eq!(
            layout.effective(&declared, &page),
            names(&["id", "amount", "street", "city"])
        );
        assert_eq!(
            layout.move_band(&declared, &page, "Address", -1),
            Some(names(&["street", "city", "id", "amount"]))
        );

        // Not past the edge of the pinned columns.
        let (declared, mut page) = banded();
        page.pins.push(("id".to_owned(), Pin::Start));
        let mut layout = ColumnLayout::default();
        assert!(layout.move_band(&declared, &page, "Address", -1).is_none());
        assert!(layout.move_column(&declared, &page, "id", 1).is_none());
    }

    /// A band is pinned as a whole: by the page's pin of one of its columns,
    /// and by the reader's — which is kept for each column of the band.
    #[test]
    fn a_pin_takes_the_whole_band() {
        let (declared, mut page) = banded();
        page.pins.push(("city".to_owned(), Pin::End));
        let mut layout = ColumnLayout::default();
        assert_eq!(layout.pin("street", &page), Some(Pin::End));
        assert_eq!(
            layout.effective(&declared, &page),
            names(&["id", "amount", "street", "city"])
        );

        assert!(layout.set_pin("street", Some(Pin::Start), &page));
        assert_eq!(
            layout.pins(),
            [
                ("city".to_owned(), Some(Pin::Start)),
                ("street".to_owned(), Some(Pin::Start))
            ]
        );
        assert_eq!(layout.pin("city", &page), Some(Pin::Start));
        assert_eq!(
            layout.effective(&declared, &page),
            names(&["street", "city", "id", "amount"])
        );
        assert!(!layout.set_pin("city", Some(Pin::Start), &page), "as it is");

        // Back to the page's pin: nothing of the reader's is left.
        assert!(layout.set_pin("city", Some(Pin::End), &page));
        assert!(layout.pins().is_empty());
        // A column outside the band is pinned alone.
        assert!(layout.set_pin("id", Some(Pin::Start), &page));
        assert_eq!(layout.pins(), [("id".to_owned(), Some(Pin::Start))]);
    }

    /// Half the width holds; a pixel more lets go.
    #[test]
    fn the_pins_hold_up_to_their_share() {
        assert!(pins_hold(160.0, 320.0));
        assert!(!pins_hold(161.0, 320.0));
    }
}
