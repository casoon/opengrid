//! Pinned columns (issue #316): which cells stay in place, where, and the
//! stylesheet that keeps them there.
//!
//! # One table, in the order it is read
//!
//! A pinned column is not drawn somewhere else: the columns pinned at the start
//! are the first of the row and the ones pinned at the end the last
//! ([`ColumnLayout::effective`](opengrid_grid::layout::ColumnLayout::effective)),
//! so the DOM, the arrow keys and a screen reader agree with the screen. What
//! keeps them in place is `position: sticky` on their cells, with the offsets
//! computed here from the widths the browser gave the header cells.
//!
//! # A sheet of its own
//!
//! The rules live in a sheet per grid, beside the one for the widths. A grid
//! without a pinned column has an empty one — nothing of its drawing changes —
//! and letting go (below) is writing the empty sheet again.
//!
//! # Letting go
//!
//! When the pinned columns would take more than
//! [`PINNED_SHARE`](opengrid_grid::layout::PINNED_SHARE) of the grid's width,
//! nothing is pinned: the columns that scroll would be left with a strip, and
//! at 320 px (WCAG 1.4.10) with nothing. The setting stays where it is.
//!
//! # The focus
//!
//! A cell scrolled under a pinned column and focused there would hold the
//! focus unseen (WCAG 2.4.11). The viewport's `scroll-padding` is the width of
//! the pinned columns at either end, so the browser's own scrolling — `focus()`
//! and `scrollIntoView` — stops clear of them.

use opengrid_grid::layout::{Pin, pins_hold};

use crate::grid::{ACCENT_PROPERTY, LINE_PROPERTY, LINE_STRONG_PROPERTY};

/// The selector of the selection column's cells, the header's and the rows'.
const SELECT: &str = "[data-select]";

/// Where the pinned cells of one grid stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinGeometry {
    /// The cells pinned at the start, each with its distance from the start
    /// edge in pixels: the selection column first (when it is there), then the
    /// columns by their `data-col`.
    pub start: Vec<(String, f64)>,
    /// The cells pinned at the end, each with its distance from the end edge.
    pub end: Vec<(String, f64)>,
    /// How wide the cells pinned at the start are together, and at the end.
    pub start_width: f64,
    pub end_width: f64,
    /// Whether the label column of a group or a total (`data-col="0"`) is
    /// pinned.
    pub label: bool,
    /// The header cells of the pinned bands (issue #319).
    pub bands: Vec<PinnedBand>,
}

/// The header cell of a pinned band (issue #319): it stays above its columns,
/// in one piece.
#[derive(Clone, Debug, PartialEq)]
pub struct PinnedBand {
    pub selector: String,
    /// Pinned at the end; at the start otherwise.
    pub end: bool,
    /// Its distance from that edge: the one of the column it has there.
    pub offset: f64,
    /// Whether the pinned columns end with this band, so it carries the line.
    pub edge: bool,
}

impl PinGeometry {
    /// Whether anything is pinned.
    pub fn is_empty(&self) -> bool {
        self.start.is_empty() && self.end.is_empty()
    }

    /// Adds the cells of the bands whose columns are pinned. `bands` are the
    /// bands of the shown columns as `(first column, how many)`; a band is
    /// pinned as a whole, so its first column says whether it is.
    pub fn with_bands(mut self, bands: &[(usize, usize)]) -> Self {
        let column = |col: usize| format!("[data-col=\"{col}\"]");
        for (first, count) in bands {
            let last = first + count - 1;
            // At the start the band stands where its first column does and
            // the pinned columns end with its last; at the end it is the
            // other way round, the cells being counted from the end.
            for (cells, end, near, far) in [
                (&self.start, false, *first, last),
                (&self.end, true, last, *first),
            ] {
                let Some((_, offset)) = cells.iter().find(|(cell, _)| *cell == column(near)) else {
                    continue;
                };
                self.bands.push(PinnedBand {
                    selector: format!("[data-band~=\"{first}\"]"),
                    end,
                    offset: *offset,
                    edge: cells.last().is_some_and(|(cell, _)| *cell == column(far)),
                });
            }
        }
        self
    }
}

/// What a grid's pins come to.
#[derive(Clone, Debug, PartialEq)]
pub enum Pins {
    /// No column is pinned.
    None,
    /// Columns are pinned, and there is no room to keep them in place.
    Released,
    /// The cells that stay in place.
    Held(PinGeometry),
}

/// Works out the pinned cells from what the browser laid out.
///
/// `columns` are the header cells that carry a `data-col`, in order: the
/// column's index, its width, and its pin — an action column (issue #189) has
/// no pin of its own and is `None`. `select` is the width of the selection
/// column when the grid shows one, `actions` how many of the trailing columns
/// are action columns, `viewport` the width the columns have to share.
///
/// Two columns are pinned without a setting of their own, because a pinned
/// column behind one that scrolls would stand in the middle of the row: the
/// **selection column** at the start whenever a column is pinned there, and
/// the **action columns** at the end whenever a column is pinned there.
pub fn pins(
    columns: &[(usize, f64, Option<Pin>)],
    select: Option<f64>,
    actions: usize,
    viewport: f64,
) -> Pins {
    let data = columns.len().saturating_sub(actions);
    let leading = columns[..data]
        .iter()
        .take_while(|(_, _, pin)| *pin == Some(Pin::Start))
        .count();
    let trailing = columns[leading..data]
        .iter()
        .rev()
        .take_while(|(_, _, pin)| *pin == Some(Pin::End))
        .count();
    if leading == 0 && trailing == 0 {
        return Pins::None;
    }

    let mut geometry = PinGeometry::default();
    if leading > 0 {
        if let Some(width) = select {
            geometry.start.push((SELECT.to_owned(), 0.0));
            geometry.start_width = width;
        }
        for (col, width, _) in &columns[..leading] {
            geometry
                .start
                .push((format!("[data-col=\"{col}\"]"), geometry.start_width));
            geometry.start_width += width;
            geometry.label |= *col == 0;
        }
    }
    if trailing > 0 {
        for (col, width, _) in columns[data - trailing..].iter().rev() {
            geometry
                .end
                .push((format!("[data-col=\"{col}\"]"), geometry.end_width));
            geometry.end_width += width;
        }
    }
    if pins_hold(geometry.start_width + geometry.end_width, viewport) {
        Pins::Held(geometry)
    } else {
        Pins::Released
    }
}

/// The sheet that keeps the pinned cells in place; empty when nothing is.
///
/// Everything a pinned grid draws differently is in here, so a grid without a
/// pin draws exactly as it did:
///
/// * the cells are `sticky` at their offset, by reading direction
///   (`inset-inline-*`, so right to left the start is the right edge);
/// * a data cell takes its row's background — a cell has none of its own, and
///   the columns that scroll would show through;
/// * the viewport's `scroll-padding` keeps a focused cell clear of them;
/// * a line marks where the pinned columns end, and the bar of a selected row
///   is drawn on the first cell, which now covers the row's own — both as an
///   inset `box-shadow`, mirrored right to left: a collapsed border belongs
///   to the table and stays where the cell would stand unpinned;
/// * under a forced palette, which drops shadows, the borders are the cells'
///   own instead (`border-collapse: separate`), so the border that marks a
///   selected row there travels with its pinned cell;
/// * a group's or total's label keeps to its column while the grid is
///   scrolled sideways (`data-scrolled` on the viewport) and runs across the
///   empty cells beside it otherwise, as it always did.
///
/// For the screen only: on paper nothing scrolls, so nothing is pinned.
pub fn pins_css(geometry: &PinGeometry) -> String {
    if geometry.is_empty() {
        return String::new();
    }
    let list = |cells: &[(String, f64)]| {
        cells
            .iter()
            .map(|(selector, _)| selector.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let bands = geometry
        .bands
        .iter()
        .map(|band| band.selector.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let all = [list(&geometry.start), list(&geometry.end), bands]
        .into_iter()
        .filter(|list| !list.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let mut css = format!(
        "@media screen {{\n\
         [part~=\"viewport\"] {{ scroll-padding-inline: {:.2}px {:.2}px; }}\n\
         :is(th, td):is({all}) {{ position: sticky; }}\n\
         td:is({all}) {{ background: inherit; }}\n\
         @media (forced-colors: active) {{ table, tbody tr {{ \
         border-collapse: separate; border-spacing: 0; }} }}\n",
        geometry.start_width, geometry.end_width
    );
    for (side, cells) in [("start", &geometry.start), ("end", &geometry.end)] {
        for (selector, offset) in cells {
            css.push_str(&format!(
                ":is(th, td){selector} {{ inset-inline-{side}: {offset:.2}px; }}\n"
            ));
        }
    }
    // The line where the pinned columns end, as drawn left to right at the
    // start (`-`) and at the end (``); right to left it is the other one.
    let line = |sign: &str| format!("inset {sign}1px 0 0 var({LINE_STRONG_PROPERTY})");
    let mirrored = |selector: &str, shadow: String, mirror: String| {
        format!(
            "{selector} {{ box-shadow: {shadow}; }}\n\
             {selector}:dir(rtl) {{ box-shadow: {mirror}; }}\n"
        )
    };
    if let Some((last, _)) = geometry.start.last() {
        css.push_str(&mirrored(
            &format!(":is(th, td){last}"),
            line("-"),
            line(""),
        ));
        // A single pinned cell carries the bar and the line.
        let (with, mirror) = if geometry.start.len() == 1 {
            (format!(", {}", line("-")), format!(", {}", line("")))
        } else {
            Default::default()
        };
        css.push_str(&mirrored(
            "tbody tr[data-selected=\"true\"] > td:first-child",
            format!("inset 3px 0 0 var({ACCENT_PROPERTY}){with}"),
            format!("inset -3px 0 0 var({ACCENT_PROPERTY}){mirror}"),
        ));
        css.push_str(&format!(
            "[part~=\"viewport\"]:not([data-scrolled]) tr[data-kind] td:is({}):empty {{ \
             background: none; }}\n",
            list(&geometry.start)
        ));
    }
    if let Some((first, _)) = geometry.end.last() {
        css.push_str(&mirrored(
            &format!(":is(th, td){first}"),
            line(""),
            line("-"),
        ));
    }
    // A pinned band's cell (issue #319): in place like its columns, and with
    // the line where the pinned columns end with it.
    for band in &geometry.bands {
        let (side, sign, mirror) = if band.end {
            ("end", "", "-")
        } else {
            ("start", "-", "")
        };
        let selector = format!("th{}", band.selector);
        css.push_str(&format!(
            "{selector} {{ inset-inline-{side}: {:.2}px; }}\n",
            band.offset
        ));
        // The line under the band is a shadow too, and is kept.
        if band.edge {
            let under = format!("inset 0 -1px 0 var({LINE_PROPERTY})");
            css.push_str(&mirrored(
                &selector,
                format!("{}, {under}", line(sign)),
                format!("{}, {under}", line(mirror)),
            ));
        }
    }
    if geometry.label {
        css.push_str(
            "[part~=\"viewport\"][data-scrolled] tr[data-kind] td[data-col=\"0\"] { \
             overflow: hidden; }\n",
        );
    }
    css.push_str("}\n");
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: Option<Pin> = Some(Pin::Start);
    const END: Option<Pin> = Some(Pin::End);

    fn held(pins: Pins) -> PinGeometry {
        match pins {
            Pins::Held(geometry) => geometry,
            other => panic!("not held: {other:?}"),
        }
    }

    /// Each pinned cell stands after the ones before it; at the end they are
    /// counted from the end.
    #[test]
    fn the_offsets_add_up_from_either_edge() {
        let columns = [
            (0, 80.0, START),
            (1, 120.0, START),
            (2, 300.0, None),
            (3, 300.0, None),
            (4, 90.0, END),
            (5, 60.0, END),
        ];
        let geometry = held(pins(&columns, None, 0, 1000.0));
        assert_eq!(
            geometry.start,
            [
                ("[data-col=\"0\"]".to_owned(), 0.0),
                ("[data-col=\"1\"]".to_owned(), 80.0)
            ]
        );
        assert_eq!(
            geometry.end,
            [
                ("[data-col=\"5\"]".to_owned(), 0.0),
                ("[data-col=\"4\"]".to_owned(), 60.0)
            ]
        );
        assert_eq!((geometry.start_width, geometry.end_width), (200.0, 150.0));
        assert!(geometry.label);
    }

    /// The selection column is pinned with the start, the action columns with
    /// the end — and neither without a column pinned there.
    #[test]
    fn the_selection_and_the_actions_go_with_their_end() {
        let columns = [
            (0, 80.0, START),
            (1, 300.0, None),
            (2, 70.0, None),
            (3, 70.0, None),
        ];
        let geometry = held(pins(&columns, Some(44.0), 2, 1000.0));
        assert_eq!(
            geometry.start,
            [
                ("[data-select]".to_owned(), 0.0),
                ("[data-col=\"0\"]".to_owned(), 44.0)
            ]
        );
        assert_eq!(geometry.start_width, 124.0);
        assert!(geometry.end.is_empty(), "no column is pinned at the end");

        let columns = [
            (0, 80.0, None),
            (1, 300.0, END),
            (2, 70.0, None),
            (3, 70.0, None),
        ];
        let geometry = held(pins(&columns, Some(44.0), 2, 1000.0));
        assert!(
            geometry.start.is_empty(),
            "no column is pinned at the start"
        );
        assert_eq!(
            geometry.end,
            [
                ("[data-col=\"3\"]".to_owned(), 0.0),
                ("[data-col=\"2\"]".to_owned(), 70.0),
                ("[data-col=\"1\"]".to_owned(), 140.0)
            ]
        );
        assert_eq!(geometry.end_width, 440.0);
        assert!(!geometry.label);
    }

    /// Up to half the width the pins hold, the selection column counted; past
    /// it nothing is pinned. Without a pin there is nothing to hold.
    #[test]
    fn past_their_share_the_pins_let_go() {
        let columns = [(0, 116.0, START), (1, 300.0, None)];
        assert!(matches!(
            pins(&columns, Some(44.0), 0, 320.0),
            Pins::Held(_)
        ));
        assert_eq!(pins(&columns, Some(44.0), 0, 319.0), Pins::Released);
        let both = [(0, 100.0, START), (1, 300.0, None), (2, 100.0, END)];
        assert_eq!(pins(&both, None, 0, 399.0), Pins::Released);
        assert!(matches!(pins(&both, None, 0, 400.0), Pins::Held(_)));

        let none = [(0, 116.0, None), (1, 300.0, None)];
        assert_eq!(pins(&none, Some(44.0), 1, 10.0), Pins::None);
    }

    /// The sheet: sticky at the offsets by reading direction, the room the
    /// focus needs, and nothing at all without a pin.
    #[test]
    fn the_sheet_keeps_the_cells_and_clears_the_focus() {
        assert_eq!(pins_css(&PinGeometry::default()), "");

        let columns = [(0, 80.0, START), (1, 300.0, None), (2, 90.0, END)];
        let css = pins_css(&held(pins(&columns, Some(44.0), 0, 1000.0)));
        assert!(css.starts_with("@media screen {"), "the screen only: {css}");
        assert!(css.contains("scroll-padding-inline: 124.00px 90.00px;"));
        assert!(css.contains(
            ":is(th, td):is([data-select], [data-col=\"0\"], [data-col=\"2\"]) { position: sticky; }"
        ));
        assert!(css.contains(":is(th, td)[data-select] { inset-inline-start: 0.00px; }"));
        assert!(css.contains(":is(th, td)[data-col=\"0\"] { inset-inline-start: 44.00px; }"));
        assert!(css.contains(":is(th, td)[data-col=\"2\"] { inset-inline-end: 0.00px; }"));
        // Logical sides only: right to left the same sheet pins at the right.
        for side in ["left:", "right:"] {
            assert!(!css.contains(side), "{side} in {css}");
        }
        // The label keeps to its column only where that column is pinned.
        assert!(css.contains("[data-scrolled] tr[data-kind] td[data-col=\"0\"]"));
        let columns = [(0, 80.0, None), (1, 90.0, END)];
        let css = pins_css(&held(pins(&columns, None, 0, 1000.0)));
        assert!(!css.contains("[data-scrolled]"), "{css}");
        assert!(!css.contains("data-selected"), "{css}");
    }

    /// A pinned band's cell stays above its columns (issue #319): at the start
    /// where its first column stands, at the end where its last one does, and
    /// with the line where the pinned columns end with it.
    #[test]
    fn a_pinned_band_stays_above_its_columns() {
        let columns = [
            (0, 80.0, START),
            (1, 100.0, START),
            (2, 120.0, START),
            (3, 300.0, None),
            (4, 300.0, None),
            (5, 90.0, END),
            (6, 60.0, END),
        ];
        // `1..=2` at the start, `3..=4` not pinned, `5..=6` at the end.
        let geometry = held(pins(&columns, None, 0, 2000.0)).with_bands(&[(1, 2), (3, 2), (5, 2)]);
        assert_eq!(
            geometry.bands,
            [
                PinnedBand {
                    selector: "[data-band~=\"1\"]".to_owned(),
                    end: false,
                    offset: 80.0,
                    edge: true,
                },
                PinnedBand {
                    selector: "[data-band~=\"5\"]".to_owned(),
                    end: true,
                    offset: 0.0,
                    edge: true,
                },
            ]
        );
        let css = pins_css(&geometry);
        for rule in [
            "[data-col=\"5\"], [data-band~=\"1\"], [data-band~=\"5\"]) { position: sticky; }",
            "th[data-band~=\"1\"] { inset-inline-start: 80.00px; }",
            "th[data-band~=\"5\"] { inset-inline-end: 0.00px; }",
            "th[data-band~=\"1\"] { box-shadow: inset -1px 0 0 var(--og-line-strong), \
             inset 0 -1px 0 var(--og-line); }",
            "th[data-band~=\"1\"]:dir(rtl) { box-shadow: inset 1px 0 0 var(--og-line-strong), \
             inset 0 -1px 0 var(--og-line); }",
            "th[data-band~=\"5\"] { box-shadow: inset 1px 0 0 var(--og-line-strong), \
             inset 0 -1px 0 var(--og-line); }",
        ] {
            assert!(css.contains(rule), "{rule} is missing from {css}");
        }

        // A band that is not the last of the pinned columns carries no line;
        // at the end the last is the one furthest from the edge.
        let geometry = held(pins(&columns, None, 0, 2000.0)).with_bands(&[(0, 2), (5, 1)]);
        assert!(!geometry.bands[0].edge && geometry.bands[1].edge);
        let css = pins_css(&geometry);
        assert!(css.contains("th[data-band~=\"0\"] { inset-inline-start: 0.00px; }"));
        assert!(css.contains("th[data-band~=\"5\"] { inset-inline-end: 60.00px; }"));
        assert!(!css.contains("th[data-band~=\"0\"] { box-shadow"), "{css}");
    }

    /// The line at the edge of the pinned columns and the bar of a selected
    /// row are shadows, so they are written for both directions.
    #[test]
    fn the_line_and_the_bar_are_mirrored() {
        let columns = [(0, 80.0, START), (1, 300.0, None), (2, 90.0, END)];
        let css = pins_css(&held(pins(&columns, None, 0, 1000.0)));
        for rule in [
            ":is(th, td)[data-col=\"0\"] { box-shadow: inset -1px 0 0 var(--og-line-strong); }",
            ":is(th, td)[data-col=\"0\"]:dir(rtl) { box-shadow: inset 1px 0 0 var(--og-line-strong); }",
            ":is(th, td)[data-col=\"2\"] { box-shadow: inset 1px 0 0 var(--og-line-strong); }",
            ":is(th, td)[data-col=\"2\"]:dir(rtl) { box-shadow: inset -1px 0 0 var(--og-line-strong); }",
            // One pinned cell at the start: it carries the bar and the line.
            "> td:first-child { box-shadow: inset 3px 0 0 var(--og-accent), \
             inset -1px 0 0 var(--og-line-strong); }",
            "> td:first-child:dir(rtl) { box-shadow: inset -3px 0 0 var(--og-accent), \
             inset 1px 0 0 var(--og-line-strong); }",
        ] {
            assert!(css.contains(rule), "{rule} is missing from {css}");
        }
        // With the selection column the first cell is not the last pinned one.
        let css = pins_css(&held(pins(&columns, Some(44.0), 0, 1000.0)));
        assert!(css.contains("> td:first-child { box-shadow: inset 3px 0 0 var(--og-accent); }"));
    }
}
