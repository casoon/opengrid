//! The `<opengrid-grid>` model: query building, result parsing, the virtualized
//! window/pool arithmetic, the keyboard navigation and the `<table role="grid">`
//! as pure patch data.
//!
//! Grid mode is the interactive sibling of table mode
//! (plan/spezifikation/09-accessibility.md §Zwei Rendering-Modi): a native
//! `<table role="grid">` whose cells are focusable and whose header cells toggle
//! the sort. Everything in this module is portable data — the query JSON is
//! built from the host attributes, the engine's result JSON is parsed into the
//! [`GridState`] of point 15, a key is mapped onto the next focused cell, the
//! virtual window is derived from a scroll offset, and the markup is computed as
//! [`Patch`]es. The same functions run on the host in unit tests and in the
//! browser through the renderer (plan/spezifikation/11-crates.md §Portabilität).
//!
//! # Display schema (Phase B)
//!
//! The provider result JSON carries only column names, not types: the typed
//! schema wire form is point 23. The grid therefore builds a **display schema**
//! whose fields are all [`DataType::Utf8`] from the result's column names, purely
//! for view bookkeeping (column order, count, header text). This is deliberate
//! and temporary — point 23 replaces it with the real types, at which point the
//! values can be formatted per type. Until then every value travels as its
//! display text (the wire notation of E13, decimals already strings).
//!
//! # Virtualization (point 17)
//!
//! At 100 000 logical rows the grid keeps a **fixed pool** of DOM rows and
//! recycles them while scrolling; no new nodes appear per scroll step
//! (plan/spezifikation/08-rendering.md §Change Detection, 13-risiken.md R2).
//!
//! * The **row pool** is [`DEFAULT_POOL_SIZE`] rows by default (the host
//!   `window-size` overrides it), each row a `<tr>` with one `<td>` per column.
//!   The pool is built once; scrolling only patches text, `data-row`,
//!   `aria-rowindex` and the row's `translateY`.
//! * **Row height** is the CSS custom property `--og-row-height` (default
//!   [`DEFAULT_ROW_HEIGHT`]px). The element resolves it once per host and passes
//!   the pixel value into the portable window math, so a logical row always
//!   occupies exactly that many pixels and the math stays stable.
//! * **Window math.** A logical row `r` sits at `r * row_height` inside the
//!   `<tbody>`, which acts as the sizer (`height = total_count * row_height`)
//!   and is `position: relative`; the rows are `position: absolute` and moved by
//!   `transform: translateY(...)`. The first visible row is therefore
//!   `scrollTop / row_height` ([`visible_start`]) and the fetched window starts
//!   [`OVERSCAN`] rows above it ([`window_offset`]), clamped so the last window
//!   ends at the last row. The `<thead>` is `position: sticky`.
//! * **Range fetching.** The provider is asked for exactly the window
//!   (`limit` = pool, `offset` = window start), never the whole result.
//! * **Focus pinning.** [`assign_pool`] keeps the slot that already holds the
//!   focused logical row, so scrolling never rewrites (and thus never blurs) the
//!   focused cell; the other slots are filled with the new window rows. A pinned
//!   row outside the window is left untouched and stays rendered (its content may
//!   be stale, which is invisible while it is scrolled out of view).
//!
//! # Counting rules (09)
//!
//! `aria-rowcount` is `total_count + 1` because the header row counts;
//! `aria-colcount` is the number of columns. Every row carries `aria-rowindex`,
//! 1-based including the header: the header row is 1, the first data row is 2,
//! so logical row `r` is `r + 2`.
//!
//! # Focus
//!
//! Exactly one cell carries `tabindex="0"`, every other cell `tabindex="-1"`
//! (roving tabindex). The active cell is an [`ActiveCell`] — a header cell or a
//! logical data cell. The element tracks it and mirrors a data cell into
//! [`GridState::set_focus`], so the state machine stays the owner of the logical
//! data focus; a header cell is element-level because the logical data model has
//! no header row. The DOM glue focuses the matching node and scrolls it into
//! view.
//!
//! # Status area (point 41)
//!
//! One line between the filter row and the viewport shows what the grid is
//! doing — loading, the result count, "no matches" or the reason a query
//! failed. It is a single `role="status"`/`aria-live="polite"` region, so every
//! state is announced without stealing the focus and never announced twice
//! (plan/spezifikation/09-accessibility.md §Statusmeldungen). The text comes
//! from the portable [`status_text`]; the [`GridStatus`] behind it is state, not
//! a renderer flag.
//!
//! # Theming (point 20)
//!
//! The shadow stylesheet is the grid's whole appearance, and a page reshapes it
//! from outside in two ways (plan/spezifikation/08-rendering.md §CSS-Architektur,
//! E8): the custom properties declared on `:host` — [`ROW_HEIGHT_PROPERTY`],
//! [`HEADER_HEIGHT_PROPERTY`], [`FILTER_HEIGHT_PROPERTY`],
//! [`STATUS_HEIGHT_PROPERTY`], [`BORDER_COLOR_PROPERTY`],
//! [`FOCUS_WIDTH_PROPERTY`] — and the exported parts `layout`, `filter`,
//! `filter-operator`, `filter-value`, `filter-clear`, `status`, `viewport`,
//! `header`, `sort-direction`, `sort-index`, `row` and `cell`. Neither requires
//! rebuilding the DOM structure.
//!
//! Three rules the theme cannot turn off, because they are accessibility, not
//! decoration:
//!
//! * **The focus ring** is an `outline` drawn *inside* the cell
//!   (`outline-offset: -width`). Drawn outside, the scroll container clips it
//!   exactly where a focused cell usually is — at the edge of the viewport.
//! * **`forced-colors`** needs no special case for text and background (they are
//!   `Canvas`/`CanvasText` and `Highlight`); only the rule colour is switched to
//!   a system colour, because a fixed grey would be forced to the same value as
//!   the background.
//! * **`prefers-reduced-motion`** neutralises animation and transition in the
//!   whole shadow tree with `!important`. That is deliberate: between shadow
//!   trees an important declaration from the **inner** tree beats the outer
//!   page, so the guarantee survives a theme that animates `::part(row)`.
//!
//! # Truncated values (point 47)
//!
//! A cell is one line with an ellipsis, because the virtualization needs a fixed
//! row height. The full value is nevertheless in the DOM — only CSS shortens it,
//! so a screen reader reads it whole — but a sighted user would lose it,
//! especially at 200%/400% zoom (WCAG 1.4.4, 1.4.10).
//!
//! The **focused cell therefore unfolds**: it wraps (`white-space: normal`, plus
//! `overflow-wrap: anywhere` for values without spaces) and, since a table
//! cell's `height` is a *minimum*, grows until the whole value fits. Its row is
//! absolutely positioned, so it grows **over** the rows below instead of moving
//! them: the sizer (`total_count * row_height`) and with it the whole window
//! arithmetic are untouched. The rows are opaque and the focused one is raised
//! by `z-index: 1` — above the other pool rows, below the sticky header
//! (`z-index: 2`), which must stay readable. Every cell is reachable through the
//! roving tabindex, so every value is.
//!
//! Two consequences worth knowing, both measured rather than assumed:
//!
//! * **At the end of the data the scroll area grows.** An absolutely positioned
//!   row still contributes to the scrollable overflow of the viewport, so a cell
//!   unfolding past the *last* row extends the scroll range by that overshoot
//!   (elsewhere it grows over rows that are inside the sizer anyway, and nothing
//!   changes). That is what keeps the end of an over-tall value reachable; it
//!   goes away when the cell folds back, and the sizer never moves, so the
//!   window arithmetic never sees it.
//! * **A cell taller than the viewport is read from the top.** `scrollIntoView`
//!   with `nearest` aligns its first line, and the cells' `scroll-margin-top`
//!   (the header height) keeps that line out from under the sticky header —
//!   which also improves plain vertical navigation, where a focused cell used to
//!   be able to land behind the header.
//!
//! Header cells deliberately do **not** unfold: the header row is in flow, so
//! growing it would shift the whole grid under it.

use opengrid_datasource::QueryResult;
use opengrid_grid::{CellRef, GridState, GridStatus, Window};
use opengrid_query::{CmpOp, FilterExpr};
use opengrid_types::{DataType, Field, FieldName, Schema};
use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

use crate::formats::CellFormat;
use crate::shared::{ASCENDING_GLYPH, DESCENDING_GLYPH, FILTER_OPERATORS, element, marker};
use crate::texts::GridTexts;

/// The custom element name (E1).
pub const GRID_TAG: &str = "opengrid-grid";

/// The host attribute naming the source in the query's `source` field.
pub const DATASOURCE_ATTRIBUTE: &str = "datasource";

/// The host attribute listing the selected fields, comma-separated.
pub const COLUMNS_ATTRIBUTE: &str = "columns";

/// The host attribute choosing where the query runs (plan point 28).
///
/// `local`, `remote`, `hybrid` or `auto`. The element does not interpret it —
/// it hands it to the provider, which is the only thing that knows whether
/// there is more than one place to run a query. Absent means the provider's own
/// default.
pub const MODE_ATTRIBUTE: &str = "mode";

/// The host attribute that switches the grid from scrolling to **paging**
/// (plan point 38).
///
/// With it the grid shows exactly one page and offers controls; without it it
/// virtualizes as before. The two are exclusive: a window inside a window would
/// mean two truths about `aria-rowcount` and two places that compute `offset`.
pub const PAGE_SIZE_ATTRIBUTE: &str = "page-size";

/// The host attribute for the recycled row pool (the query's `limit`).
///
/// Point 16 paged with this attribute; point 17 reinterprets it as the
/// **window/pool size** — how many rows are rendered and fetched at once while
/// scrolling. Review renamed the attribute from its old paging name to
/// `window-size` to match that meaning.
pub const WINDOW_SIZE_ATTRIBUTE: &str = "window-size";

/// The pool size used when the `window-size` attribute is absent or invalid.
pub const DEFAULT_POOL_SIZE: u64 = 40;

/// The default pixel height of one logical row (the virtualization contract).
///
/// The host can override it with the CSS custom property
/// [`ROW_HEIGHT_PROPERTY`]; the element resolves the computed value once per
/// host and feeds it into the portable window math.
pub const DEFAULT_ROW_HEIGHT: u64 = 42;

/// The CSS custom property a host can set to override the row height.
///
/// A shadow-root stylesheet seeds it with [`DEFAULT_ROW_HEIGHT`]; a document
/// rule on the host (or an inline style) overrides it and the value inherits
/// into the shadow tree.
pub const ROW_HEIGHT_PROPERTY: &str = "--og-row-height";

/// Rows kept above the first visible row so scrolling stays smooth.
pub const OVERSCAN: u64 = 6;

/// Fallback viewport height in rows for `PageUp`/`PageDown` when the browser
/// cannot report a laid-out height.
pub const DEFAULT_VIEWPORT_ROWS: u64 = 12;

/// The fixed pixel height of the filter row (`part="filter"`).
///
/// The row is a single horizontal line of controls, so its height is constant
/// regardless of the column count; the viewport below it takes the rest of the
/// host. A host can override it with [`FILTER_HEIGHT_PROPERTY`], but the default
/// keeps the virtualization math and the fixtures deterministic.
pub const FILTER_HEIGHT: u64 = 40;

/// The CSS custom property overriding [`FILTER_HEIGHT`].
pub const FILTER_HEIGHT_PROPERTY: &str = "--og-filter-height";

/// The pixel height of the status line (`part="status"`, point 41).
///
/// A **minimum**, not a fixed height: one line of status text is exactly this
/// tall, which keeps the viewport below it — and with it the `PageUp`/`PageDown`
/// step — deterministic, while a long error message is allowed to wrap instead
/// of being cut off. The window math reads the viewport's live height, so a
/// grown status line simply leaves fewer rows visible.
pub const STATUS_HEIGHT: u64 = 24;

/// The CSS custom property overriding [`STATUS_HEIGHT`].
pub const STATUS_HEIGHT_PROPERTY: &str = "--og-status-height";

/// The CSS custom property for the header row's height (point 20).
///
/// Defaults to [`ROW_HEIGHT_PROPERTY`] so a grid looks even out of the box. The
/// header is sticky inside the viewport and sits outside the `<tbody>` sizer, so
/// its height is independent of the virtualization math — unlike the row height,
/// which the element has to resolve in Rust.
pub const HEADER_HEIGHT_PROPERTY: &str = "--og-header-height";

/// The CSS custom property for the width of the focus ring (point 20).
pub const FOCUS_WIDTH_PROPERTY: &str = "--og-focus-width";

// ---------------------------------------------------------------------------
// The appearance tokens (point 57)
// ---------------------------------------------------------------------------
//
// Eleven properties a page sets, five the stylesheet computes from them. The
// split is the whole idea: a page picks an accent and the grid works out what a
// soft accent, a hover tint and a selected row look like against *its* surface,
// instead of asking for eleven more colours it would have to keep consistent.
//
// **The defaults are the system colours.** Two reasons, and both are
// accessibility rather than taste: a grid with no page CSS stays legible and in
// the right light/dark, and `forced-colors` keeps winning. `color-mix` with a
// system colour is valid CSS but resolves unpredictably once a forced palette is
// active, so the whole computed set is reset to system colours there — that is
// the one rule this layer has, and it is one line per token.
//
// Two tokens the prototype shows are deliberately **not** here, both for the
// same reason: a property the element declares and then ignores is a promise it
// does not keep, and the test below enforces exactly that.
//
// * `--og-canvas` is the page's own background. The grid is the card that sits
//   on it and uses [`SURFACE_PROPERTY`].
// * `--og-font-mono` has nothing to apply to until a column can ask for it
//   (point 60, `mono: true`). It arrives with the thing that uses it.
//
// Three more of the prototype's tokens wait for their feature for the same
// reason: `--og-on-accent` and `--og-accent-soft` until something is drawn *on*
// the accent (points 61 and 65), `--og-faint` until a cell has a placeholder to
// draw faintly (point 60).

/// The font family of everything the grid draws. Defaults to `inherit`.
pub const FONT_PROPERTY: &str = "--og-font";

/// The face of a column a page marked `mono` (point 60) — values that line up
/// character by character, such as ids and codes.
pub const FONT_MONO_PROPERTY: &str = "--og-font-mono";

/// The font size of everything the grid draws. Density drives it (point 58).
pub const FONT_SIZE_PROPERTY: &str = "--og-font-size";

/// The grid's own surface: rows, the body of the card.
pub const SURFACE_PROPERTY: &str = "--og-surface";

/// The surface one step away from the data: header, filter row, status line.
pub const SURFACE_2_PROPERTY: &str = "--og-surface-2";

/// The colour of the text.
pub const INK_PROPERTY: &str = "--og-ink";

/// The colour of text that is there but not the point.
pub const INK_MUTED_PROPERTY: &str = "--og-ink-muted";

/// Text drawn **on** the accent — the tick of a checked selection box
/// (point 61).
pub const ON_ACCENT_PROPERTY: &str = "--og-on-accent";

/// The rules between rows and cells.
pub const LINE_PROPERTY: &str = "--og-line";

/// The rules that separate regions — the filter row from the data, the status
/// line from the viewport.
pub const LINE_STRONG_PROPERTY: &str = "--og-line-strong";

/// The one colour a page picks. Everything accented is computed from it.
pub const ACCENT_PROPERTY: &str = "--og-accent";

/// The corner radius of the grid's boxes.
pub const RADIUS_PROPERTY: &str = "--og-radius";

/// The horizontal padding inside a cell. Density drives it (point 58).
pub const PAD_PROPERTY: &str = "--og-pad";

// ---------------------------------------------------------------------------
// Density (point 58)
// ---------------------------------------------------------------------------

/// The attribute that picks a density.
pub const DENSITY_ATTRIBUTE: &str = "density";

/// The columns a grid groups by, outermost first (point 62).
///
/// At most two. Setting it turns the `grid` into a `treegrid` for as long as it
/// is set (F2, 2026-09-23): an expandable hierarchy is what `treegrid` exists
/// for, and a `grid` with buttons in it would describe the structure worse.
pub const GROUP_BY_ATTRIBUTE: &str = "group-by";

/// The boolean attribute that puts a toolbar above the grid (point 65): the
/// active filters as chips, a switch for the filter row, the column list and
/// the density.
///
/// Opt-in, and not `filter-row`: the filter row has always been there, and a
/// boolean attribute is *off* by default in HTML — an attribute named for the
/// row would have taken it away from every grid that does not say it. Whether
/// the row shows is part of the view instead (`filterRow`), on by default.
pub const TOOLBAR_ATTRIBUTE: &str = "toolbar";

/// The boolean attribute that puts a search field above the grid (point 67):
/// free text, or a filter written out (`country = DE and amount ≥ 10`).
pub const SEARCH_ATTRIBUTE: &str = "search";

/// The boolean attribute that shows the facet sidebar (point 66).
///
/// The facets themselves are what a page configured with `set_columns`
/// (`facet: "list" | "pills" | "range" | "period"`) — there is no facet per
/// column by default, for the reason there is no aggregate by default: a range
/// over the ids would be a control nobody asked for.
pub const FACETS_ATTRIBUTE: &str = "facets";

/// The boolean attribute that gives every column header a menu (point 64).
///
/// Opt-in like the selection column: a grid that is read rather than worked
/// with has no use for a menu in every header.
pub const COLUMN_MENU_ATTRIBUTE: &str = "column-menu";

/// The boolean attribute that shows the selection column (point 61).
///
/// **Opt-in, like every other piece of chrome in this phase** (`filter-row`,
/// `facets`, `column-menu`). Selecting rows has worked from the keyboard since
/// point 35 whether or not this is set; what the attribute adds is the column
/// that *shows* it and the pointer path to it. A grid that is read rather than
/// worked with should not pay for a column in its `aria-colcount`, and a reader
/// should not be told there is a control where there is nothing to do.
pub const SELECTION_ATTRIBUTE: &str = "selection";

/// The three steps, as `(attribute value, row height px, padding px, font size)`.
///
/// `normal` is the default and is declared on `:host` itself, so a grid without
/// the attribute is a normal one — there is no fourth, nameless density.
///
/// **The row height is pixels and the font size is `rem`**, and that is not an
/// oversight. The row height is the virtualization contract: the element reads
/// the *specified* value of the custom property and parses `<number>px` from it,
/// because an unregistered custom property is not resolved for
/// `getComputedStyle`. The font size has no such reader, so it can be relative —
/// and it should be, or a reader who raised their browser's font size would be
/// overruled by ours (1.4.4). The consequence a page has to know: raising the
/// font size means raising [`ROW_HEIGHT_PROPERTY`] with it.
pub const DENSITIES: &[(&str, u64, u64, &str)] = &[
    ("compact", 34, 10, "0.8125rem"),
    ("normal", DEFAULT_ROW_HEIGHT, 12, "0.875rem"),
    ("comfortable", 50, 16, "0.875rem"),
];

/// The density a value names, or `normal` for anything else — including absent.
pub fn density_of(raw: Option<&str>) -> &'static (&'static str, u64, u64, &'static str) {
    let wanted = raw.unwrap_or("").trim();
    DENSITIES
        .iter()
        .find(|(name, ..)| *name == wanted)
        .unwrap_or(&DENSITIES[1])
}

/// Computed: the accent as a background behind accented text — a pressed
/// toolbar switch, a filter chip (point 65).
pub const ACCENT_SOFT_PROPERTY: &str = "--og-accent-soft";

/// Computed: the accent as readable text on the surface.
pub const ACCENT_INK_PROPERTY: &str = "--og-accent-ink";

/// Computed: the background of a selected row.
///
/// Never the only sign of selection — the row also carries an inset accent bar,
/// because colour alone is 1.4.1.
pub const SELECTED_PROPERTY: &str = "--og-selected";

/// Computed: the background of a hovered row.
pub const HOVER_PROPERTY: &str = "--og-hover";

/// Every token the page sets, in the order the documentation lists them.
#[cfg(test)]
pub const SET_TOKENS: &[&str] = &[
    FONT_PROPERTY,
    FONT_MONO_PROPERTY,
    FONT_SIZE_PROPERTY,
    SURFACE_PROPERTY,
    SURFACE_2_PROPERTY,
    INK_PROPERTY,
    INK_MUTED_PROPERTY,
    LINE_PROPERTY,
    LINE_STRONG_PROPERTY,
    ACCENT_PROPERTY,
    ON_ACCENT_PROPERTY,
    RADIUS_PROPERTY,
    PAD_PROPERTY,
    FOCUS_WIDTH_PROPERTY,
    ROW_HEIGHT_PROPERTY,
    HEADER_HEIGHT_PROPERTY,
    FILTER_HEIGHT_PROPERTY,
    STATUS_HEIGHT_PROPERTY,
];

/// Every token the stylesheet computes from [`SET_TOKENS`].
///
/// A page may override one, but it does not have to — and under a forced palette
/// every one of them is reset to a system colour.
#[cfg(test)]
pub const COMPUTED_TOKENS: &[&str] = &[
    ACCENT_SOFT_PROPERTY,
    ACCENT_INK_PROPERTY,
    SELECTED_PROPERTY,
    HOVER_PROPERTY,
];

/// The default focus ring width. Drawn **inside** the cell
/// (`outline-offset: -width`), so the scroll container cannot clip it at the
/// edges of the viewport — which is where a focused cell usually is.
pub const DEFAULT_FOCUS_WIDTH: &str = "2px";

/// The minimum size of a pointer target in the filter row (WCAG 2.2 §2.5.8).
///
/// The browser's default `<input>`/`<select>` is a little under this at the
/// inherited font size, so the grid raises it; the 40px filter row has the room.
pub const MIN_TARGET_SIZE: u64 = 24;

/// The operators that make sense for a column (plan point 51).
///
/// Until point 23 the grid had no types and offered all eight comparisons for
/// every column — against a number or a date that is a type error, which is why
/// filtering only ever worked on text. Now the column decides:
///
/// * `contains`/`starts_with` are substring tests and belong to `Utf8` alone.
/// * the ordered comparisons apply to every type but `Bool`, which has two
///   values and no order worth offering.
/// * `eq`/`ne` apply everywhere.
/// * `is_null`/`is_not_null` only where the column may actually be null — on a
///   required column they would be a question with a constant answer.
pub fn operators_for(data_type: DataType, nullable: bool) -> Vec<&'static str> {
    let mut operators = Vec::with_capacity(FILTER_OPERATORS.len());
    if data_type == DataType::Utf8 {
        operators.push("contains");
        operators.push("starts_with");
    }
    operators.push("eq");
    operators.push("ne");
    if data_type != DataType::Bool {
        operators.extend(["gt", "gte", "lt", "lte"]);
    }
    if nullable {
        operators.extend(["is_null", "is_not_null"]);
    }
    operators
}

/// Whether an operator needs a value at all.
///
/// `is_null`/`is_not_null` are the two that do not; their value input is
/// disabled rather than ignored, so the control says what it does.
pub fn takes_value(op: &str) -> bool {
    !matches!(op, "is_null" | "is_not_null")
}

/// The `type` of the value input for a column (plan point 51).
///
/// A date picker for a date, a number spinner for a number, a checkbox for a
/// boolean — the browser then does the parsing, the keyboard support and the
/// locale-correct presentation for free, and an impossible value is harder to
/// type in the first place.
pub fn input_type(data_type: DataType) -> &'static str {
    match data_type {
        DataType::Bool => "checkbox",
        DataType::Int64 | DataType::Float64 | DataType::Decimal { .. } => "number",
        DataType::Date => "date",
        // A timestamp needs a time zone-free local field; the wire form is UTC
        // ISO-8601, which `datetime-local` does not produce — text for now.
        DataType::Timestamp | DataType::Utf8 => "text",
    }
}

/// The step attribute that lets a number input accept the column's precision.
///
/// Without it a browser rounds a decimal input to whole numbers, and the value
/// the user typed is not the value that gets filtered.
pub fn input_step(data_type: DataType) -> Option<String> {
    match data_type {
        DataType::Int64 => Some("1".to_owned()),
        DataType::Float64 => Some("any".to_owned()),
        DataType::Decimal { scale, .. } if scale > 0 => {
            Some(format!("0.{}1", "0".repeat(scale as usize - 1)))
        }
        DataType::Decimal { .. } => Some("1".to_owned()),
        _ => None,
    }
}

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[
    LABEL_ATTRIBUTE,
    DATASOURCE_ATTRIBUTE,
    COLUMNS_ATTRIBUTE,
    WINDOW_SIZE_ATTRIBUTE,
    MODE_ATTRIBUTE,
    PAGE_SIZE_ATTRIBUTE,
    DENSITY_ATTRIBUTE,
    SELECTION_ATTRIBUTE,
    GROUP_BY_ATTRIBUTE,
    COLUMN_MENU_ATTRIBUTE,
    TOOLBAR_ATTRIBUTE,
    FACETS_ATTRIBUTE,
    SEARCH_ATTRIBUTE,
];

/// Reads `page-size`; absent, empty or unusable means "do not page".
pub fn parse_page_size(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|size| *size > 0)
}

/// What the rendered area covers.
///
/// While virtualizing it is the whole result (`base = 0`), so the sizer spans
/// every row and the window slides inside it. While paging it is **one page**:
/// the sizer is the page, `aria-rowcount` counts the page, and a row's position
/// is relative to it — a screen reader reads what is there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paging {
    /// The first logical row of the rendered area.
    pub base: u64,
    /// How many rows the rendered area holds.
    pub rows: u64,
    /// The page size while paging; `None` while virtualizing.
    pub size: Option<u64>,
}

impl Paging {
    /// The whole result, as the virtualizing grid renders it.
    pub const fn whole(total_count: u64) -> Self {
        Self {
            base: 0,
            rows: total_count,
            size: None,
        }
    }

    /// One page of `size` rows out of `total_count`, `page` counted from 0.
    pub fn page(page: u64, size: u64, total_count: u64) -> Self {
        let base = page.saturating_mul(size).min(total_count);
        Self {
            base,
            rows: (total_count - base).min(size),
            size: Some(size),
        }
    }

    /// The 0-based page currently rendered.
    pub fn index(&self) -> u64 {
        match self.size {
            Some(size) if size > 0 => self.base / size,
            _ => 0,
        }
    }

    /// How many pages `total_count` rows make at this size.
    pub fn count(total_count: u64, size: u64) -> u64 {
        if size == 0 {
            return 1;
        }
        total_count.div_ceil(size).max(1)
    }
}

/// Which cell owns the roving tabindex.
///
/// A header cell is addressed by its column alone; the logical data model has no
/// header row, so it cannot be a [`CellRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveCell {
    /// The `<th>` above the selection column (point 61).
    ///
    /// Its own variant rather than `Header { col: 0 }` with everything shifted:
    /// `col` means **schema column** everywhere else in this crate — formats,
    /// presentation, filters and every `data-col` in the DOM key on it. Making
    /// it mean "position in the row" instead would have renumbered all of them
    /// to save two variants.
    SelectAll,
    /// The `<td>` of a row's selection column.
    Select {
        /// Logical row in the whole result.
        row: u64,
    },
    /// A `<th>` of the header row.
    Header {
        /// Column index into the schema.
        col: usize,
    },
    /// A `<td>` of a data row.
    Data(CellRef),
}

impl ActiveCell {
    /// The logical data cell, or `None` for a header or selection cell.
    pub const fn data(self) -> Option<CellRef> {
        match self {
            Self::SelectAll | Self::Select { .. } | Self::Header { .. } => None,
            Self::Data(cell) => Some(cell),
        }
    }

    /// The logical row this cell belongs to, header rows excluded.
    pub const fn row(self) -> Option<u64> {
        match self {
            Self::SelectAll | Self::Header { .. } => None,
            Self::Select { row } => Some(row),
            Self::Data(cell) => Some(cell.row),
        }
    }
}

/// A key the grid handles (decoded from a `keydown` in the element).
///
/// The matrix is the one from plan/spezifikation/09-accessibility.md §Tastatur im
/// Grid Mode. `Tab`/`Shift+Tab` are deliberately absent: the roving tabindex lets
/// the browser move out of the grid on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridKey {
    /// One row up.
    ArrowUp,
    /// One row down.
    ArrowDown,
    /// One column left.
    ArrowLeft,
    /// One column right.
    ArrowRight,
    /// First column of the current row.
    Home,
    /// Last column of the current row.
    End,
    /// First cell of the grid.
    CtrlHome,
    /// Last cell of the grid.
    CtrlEnd,
    /// One viewport up.
    PageUp,
    /// One viewport down.
    PageDown,
}

/// The nodes [`build_grid`] creates and [`patch_grid`] updates.
///
/// The whole grid tree is built once; the ids stay valid because the element
/// keeps the [`Dom`](opengrid_web_core::renderer::Dom) that created them. Every
/// later frame patches these nodes instead of rebuilding the table, which is what
/// makes row recycling (and focus survival) possible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridNodes {
    /// The type-agnostic filter row above the table (`part="filter"`).
    pub filter: FilterNodes,
    /// The `role="status"` line below the filter row (`part="status"`).
    pub status: NodeId,
    /// The scrollable viewport (`overflow-y: auto`) inside the shadow root.
    pub viewport: NodeId,
    /// The table's `<tbody>`, used as the sizer (`height = total * row_height`).
    pub tbody: NodeId,
    /// The `<table role="grid">`.
    pub table: NodeId,
    /// The `<th>` above the selection column, when the column is shown.
    ///
    /// `Option`, not a sentinel node: [`NodeId::ROOT`] is a **legal** node, and
    /// a patch aimed at it replaces the whole shadow tree. That is not a
    /// theoretical objection — it is what happened.
    pub select_all: Option<NodeId>,
    /// The mark inside it, whose text is the checkbox glyph.
    pub select_all_mark: Option<NodeId>,
    /// The header cells, one per column.
    pub header_cells: Vec<GridHeaderNodes>,
    /// The recycled pool: one entry per slot.
    pub rows: Vec<GridRowNodes>,
    /// The paging controls below the table (`part="pager"`), hidden while the
    /// grid virtualizes (plan point 38).
    pub pager: PagerNodes,
    /// One checkbox per declared column (`part="columns"`, plan point 36).
    pub columns: ColumnsNodes,
    /// The empty state (point 68).
    pub empty: NodeId,
}

/// The column-visibility controls.
///
/// Ordinary checkboxes in a labelled group, **outside** `role="grid"`: hiding a
/// column has to be undoable, and a list of checkboxes is the way back that a
/// keyboard and a screen reader both already know. A menu would have to invent
/// one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnsNodes {
    /// The disclosure button that opens the list.
    pub toggle: NodeId,
    pub container: NodeId,
    /// `(checkbox, column name)` in the order the `columns` attribute declares.
    pub boxes: Vec<(NodeId, String)>,
}

/// The paging controls: four buttons and the label that says where you are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PagerNodes {
    /// The `role="group"` holding them, **outside** `role="grid"` — like the
    /// filter row, so the grid's roving tabindex is untouched.
    pub container: NodeId,
    pub first: NodeId,
    pub previous: NodeId,
    pub next: NodeId,
    pub last: NodeId,
    /// „Page 3 of 12" — where the reader is, in words.
    pub label: NodeId,
}

/// The nodes of one header cell: the `<th>` and its visible multi-sort index.
///
/// The column name lives in a static child `<span>`; the index `<span>` is
/// `aria-hidden`, so showing it never changes the header's accessible name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridHeaderNodes {
    /// The `<th scope="col">`.
    pub cell: NodeId,
    /// The `<span>` carrying the sort direction glyph (empty when unsorted).
    pub direction: NodeId,
    /// The `<span>` carrying the multi-sort order (empty for a single sort).
    pub index: NodeId,
}

/// The nodes of the filter row (plan point 18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterNodes {
    /// The row container (`part="filter"`).
    pub container: NodeId,
    /// The "Clear" button (`part="filter-clear"`).
    pub clear: NodeId,
    /// One operator `select` + value `input` per column.
    pub columns: Vec<FilterColumnNodes>,
}

/// The controls of one column's filter group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterColumnNodes {
    /// The operator `<select>`.
    pub select: NodeId,
    /// One `<option>` per entry of [`FILTER_OPERATORS`], in that order.
    ///
    /// All of them are built once — the patch language can set attributes but
    /// not remove children, and the column's **type** only arrives with the
    /// first result (point 23). Which of them apply is therefore an attribute on
    /// each option, updated per frame, not a different set of nodes.
    pub options: Vec<NodeId>,
    /// The value `<input>`.
    pub input: NodeId,
}

/// The nodes of one recycled pool row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridRowNodes {
    /// The `<tr>`.
    pub row: NodeId,
    /// The `<td>` of the selection column, when the column is shown.
    pub select: Option<NodeId>,
    /// The mark inside it.
    pub select_mark: Option<NodeId>,
    /// The `<td>`s, one per column.
    pub cells: Vec<NodeId>,
}

impl GridNodes {
    /// The pool size (number of recycled row slots).
    pub fn pool(&self) -> usize {
        self.rows.len()
    }
}

/// Splits the `columns` attribute into field names (same rule as table mode).
pub fn parse_columns(raw: Option<&str>) -> Vec<String> {
    crate::table::parse_columns(raw)
}

/// The pool size from the `window-size` attribute, or [`DEFAULT_POOL_SIZE`].
///
/// A missing, non-numeric or zero value falls back to the default, so a typo
/// never turns into a zero-row pool.
pub fn parse_window_size(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|size| *size > 0)
        .unwrap_or(DEFAULT_POOL_SIZE)
}

/// The row height from a computed CSS value, or [`DEFAULT_ROW_HEIGHT`].
///
/// Accepts a `<number>px` value (e.g. `48px`); anything else — absent, a
/// different unit, or zero — falls back to the default, so the window math
/// always has a positive, well-defined height.
pub fn parse_row_height(raw: &str) -> u64 {
    raw.trim()
        .strip_suffix("px")
        .and_then(|number| number.trim().parse::<u64>().ok())
        .filter(|height| *height > 0)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
}

/// The display schema for the initial render, before the first result arrives.
///
/// All fields are [`DataType::Utf8`] — the display schema decision documented on
/// the module. Names that are not valid identifiers are skipped; the result
/// schema replaces this one on the first [`GridState::apply_result`].
#[cfg(test)]
pub fn initial_schema(columns: &[String]) -> Schema {
    known_schema(columns, &std::collections::BTreeMap::new())
}

/// The schema of `columns` as far as it is known (plan point 88): a column a
/// result has already typed keeps that type across a rebuild, the rest are
/// display text until a result says otherwise.
///
/// Without this a rebuild — applying a view, showing a column — fell back to
/// all text, and a view's `qty ≥ 2` went out as the string `"2"`.
pub fn known_schema(
    columns: &[String],
    known: &std::collections::BTreeMap<String, Field>,
) -> Schema {
    let fields = columns
        .iter()
        .filter_map(|name| match known.get(name) {
            Some(field) => Some(field.clone()),
            None => FieldName::new(name.as_str())
                .ok()
                .map(|name| Field::new(name, DataType::Utf8)),
        })
        .collect();
    Schema::new(fields)
}

/// One row of the filter UI: a column, an operator and the typed value.
///
/// Values are always plain strings — the display schema is all `Utf8` in Phase B
/// and the result JSON carries no types (point 23). The literal is therefore sent
/// as a JSON string; comparing it works for text columns and is a documented
/// limitation for the others until point 23.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterEntry {
    /// The output column the comparison names.
    pub column: String,
    /// The chosen operator.
    pub op: FilterOp,
    /// The raw value text; an empty (or whitespace-only) value means "no filter",
    /// except for the operators that take none.
    pub value: String,
}

/// What the operator control can be set to (plan point 51).
///
/// `CmpOp` covers the comparisons; the two null tests are not comparisons in the
/// AST — they are their own filter nodes — so the control's choice needs a type
/// that can hold either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOp {
    Cmp(CmpOp),
    IsNull,
    IsNotNull,
}

impl FilterOp {
    /// Reads a wire token, including the two null tests.
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "is_null" => Some(FilterOp::IsNull),
            "is_not_null" => Some(FilterOp::IsNotNull),
            other => CmpOp::parse(other).map(FilterOp::Cmp),
        }
    }

    /// The wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            FilterOp::Cmp(op) => op.as_str(),
            FilterOp::IsNull => "is_null",
            FilterOp::IsNotNull => "is_not_null",
        }
    }
}

/// What a filter row entry can be wrong about (plan point 51).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterProblem {
    /// The column whose input does not fit.
    pub column: String,
    /// What the user typed.
    pub value: String,
}

/// The `filter` expression of the filter row, with literals typed per column.
///
/// Until point 51 every value went out as a JSON string. Against a `Decimal`,
/// `Int64`, `Date` or `Bool` column that is a **type error** by the rules of
/// §Literaltypen — which is why filtering only ever worked on text. Now each
/// literal is written in the notation its column expects (§Typsystem: decimal as
/// a string, date `YYYY-MM-DD`, timestamp ISO-8601 `Z`, numbers as JSON numbers,
/// booleans as JSON booleans).
///
/// An entry the column cannot accept is **not** sent: it comes back as a
/// [`FilterProblem`], so the grid can say so instead of letting the engine or the
/// server answer with a validation error the user did not cause.
///
/// Entries with a blank value are skipped — except for the two operators that
/// take no value at all. A schema without the column is skipped rather than
/// turned into a malformed query.
pub fn filter_expr(
    entries: &[FilterEntry],
    schema: &Schema,
) -> Result<Option<FilterExpr>, Vec<FilterProblem>> {
    let mut comparisons = Vec::new();
    let mut problems = Vec::new();

    for entry in entries {
        let Ok(field) = FieldName::new(entry.column.as_str()) else {
            continue;
        };
        let Some(column) = schema.field(entry.column.as_str()) else {
            continue;
        };

        match entry.op {
            FilterOp::IsNull => comparisons.push(FilterExpr::IsNull { field }),
            FilterOp::IsNotNull => comparisons.push(FilterExpr::IsNotNull { field }),
            FilterOp::Cmp(op) => {
                if entry.value.trim().is_empty() {
                    continue;
                }
                match literal(&entry.value, column.data_type) {
                    Some(value) => comparisons.push(FilterExpr::Cmp { field, op, value }),
                    None => problems.push(FilterProblem {
                        column: entry.column.clone(),
                        value: entry.value.clone(),
                    }),
                }
            }
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(if comparisons.is_empty() {
        None
    } else {
        Some(FilterExpr::And(comparisons))
    })
}

/// One typed literal in the notation its column expects, or `None` when the text
/// is not a value of that type.
///
/// The checking is deliberately shallow — it decides whether the JSON is of the
/// right *kind*, and validation against the schema does the rest. What it must
/// not do is pass something through that will fail later: the user typed it, so
/// the user should hear about it here.
pub fn literal(text: &str, data_type: DataType) -> Option<serde_json::Value> {
    let text = text.trim();
    match data_type {
        DataType::Utf8 => Some(serde_json::Value::String(text.to_owned())),
        DataType::Bool => match text {
            "true" | "on" | "1" => Some(serde_json::Value::Bool(true)),
            "false" | "off" | "0" | "" => Some(serde_json::Value::Bool(false)),
            _ => None,
        },
        DataType::Int64 => text.parse::<i64>().ok().map(Into::into),
        DataType::Float64 => {
            // E13: the three non-finite values travel as those exact words.
            if matches!(text, "NaN" | "Infinity" | "-Infinity") {
                return Some(serde_json::Value::String(text.to_owned()));
            }
            let number = text.parse::<f64>().ok()?;
            serde_json::Number::from_f64(number).map(serde_json::Value::Number)
        }
        // A decimal travels as a string so no precision is lost on the way
        // (§Typsystem). Checked here for shape only.
        DataType::Decimal { .. } => {
            let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
            let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
            let ok = !whole.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && fraction.bytes().all(|b| b.is_ascii_digit());
            ok.then(|| serde_json::Value::String(text.to_owned()))
        }
        DataType::Date => {
            // `YYYY-MM-DD`, which is what `<input type="date">` produces.
            let parts: Vec<&str> = text.split('-').collect();
            let ok = parts.len() == 3
                && parts[0].len() == 4
                && parts[1].len() == 2
                && parts[2].len() == 2
                && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_digit()));
            ok.then(|| serde_json::Value::String(text.to_owned()))
        }
        DataType::Timestamp => {
            let ok = text.len() >= 20 && text.ends_with('Z') && text.contains('T');
            ok.then(|| serde_json::Value::String(text.to_owned()))
        }
    }
}

/// The text of the status line for a status (plan point 41).
///
/// One sentence per state, from the component's [`GridTexts`] (point 48), so the
/// page decides the wording and the language. It is both what the user reads and
/// what the `aria-live` region announces, so it names a cause instead of a code:
/// the [`GridStatus::Error`] message is already the user-facing sentence
/// ([`GridTexts::error`] builds it).
pub fn status_text(texts: &GridTexts, status: &GridStatus, total_count: u64) -> String {
    match status {
        GridStatus::Loading => texts.loading.clone(),
        GridStatus::Ready => texts.matches(total_count),
        GridStatus::Empty => texts.empty.clone(),
        GridStatus::Error(message) => message.clone(),
    }
}

/// The status line, including the notice that a selection was dropped.
///
/// The notice rides along with the result that replaced the rows instead of
/// getting a live region of its own — point 41 left the grid exactly **one**,
/// and two would talk over each other.
pub fn status_line(texts: &GridTexts, state: &opengrid_grid::GridState) -> String {
    status_line_counting(texts, state, state.total_count())
}

/// [`status_line`] with the number of matches given rather than read — under
/// grouping (point 62) the state counts display positions, and those include
/// the group headers.
pub fn status_line_counting(
    texts: &GridTexts,
    state: &opengrid_grid::GridState,
    matches: u64,
) -> String {
    let mut text = status_text(texts, state.status(), matches);
    if state.announce_selection_cleared() {
        text = format!("{text} · {}", texts.selection_cleared);
    }
    // A column operation rides along with the result line, in the one live
    // region the grid has (points 36 and 41).
    if let Some(notice) = state.notice() {
        text = format!("{text} · {notice}");
    }
    text
}

/// The `data-state` token of the status line, for styling (`::part(status)`).
pub fn status_state(status: &GridStatus) -> &'static str {
    match status {
        GridStatus::Loading => "loading",
        GridStatus::Ready => "ready",
        GridStatus::Empty => "empty",
        GridStatus::Error(_) => "error",
    }
}

/// Builds the query JSON for a grid render.
///
/// The shape is `{ "source", "select", "filter"?, "sort"?, "limit", "offset" }`
/// (plan/spezifikation/02-query-modell.md §JSON-Vertrag). `sorts` is the whole
/// sort list in the user's order (point 18 multi-sort); each entry becomes a
/// `{ "field", "direction" }` object and the key is omitted when nothing is
/// sorted. `filter` is serialized from the [`FilterExpr`] built by
/// [`filter_expr`]; `direction` is the wire token `"asc"`/`"desc"`.
pub fn query_json(
    source: &str,
    columns: &[String],
    sorts: &[(String, &str)],
    filter: Option<&FilterExpr>,
    offset: u64,
    limit: u64,
) -> String {
    use serde_json::{Value as Json, json};
    let sort = sorts
        .iter()
        .map(|(field, direction)| json!({ "field": field, "direction": direction }))
        .collect();
    let mut query = query_object(source, columns, sort, filter);
    query.insert("limit".to_owned(), json!(limit));
    query.insert("offset".to_owned(), json!(offset));
    Json::Object(query).to_string()
}

/// The query of a whole view — the same as the grid asks, without a window
/// (plan point 82): what a page exports.
///
/// `groups` come first, ascending with **NULL last** stated explicitly — the
/// order [`crate::grouping::group_query_json`] gives the groups, written the
/// same way, so the rows of a grouped grid come in the order it shows them.
/// Then `sorts`, without the keys `groups` already has.
pub fn view_query_json(
    source: &str,
    columns: &[String],
    groups: &[String],
    sorts: &[(String, &str)],
    filter: Option<&FilterExpr>,
) -> String {
    use serde_json::json;
    let sort = groups
        .iter()
        .map(|field| json!({ "field": field, "direction": "asc", "nulls": "last" }))
        .chain(
            sorts
                .iter()
                .filter(|(field, _)| !groups.contains(field))
                .map(|(field, direction)| json!({ "field": field, "direction": direction })),
        )
        .collect();
    serde_json::Value::Object(query_object(source, columns, sort, filter)).to_string()
}

/// Source, projection, filter and sort — everything but the window.
fn query_object(
    source: &str,
    columns: &[String],
    sort: Vec<serde_json::Value>,
    filter: Option<&FilterExpr>,
) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::Value as Json;
    let mut query = serde_json::Map::new();
    query.insert("source".to_owned(), Json::String(source.to_owned()));
    query.insert(
        "select".to_owned(),
        Json::Array(columns.iter().cloned().map(Json::String).collect()),
    );
    if let Some(filter) = filter {
        let filter = serde_json::to_value(filter).expect("a filter expression serializes");
        query.insert("filter".to_owned(), filter);
    }
    if !sort.is_empty() {
        query.insert("sort".to_owned(), Json::Array(sort));
    }
    query
}

/// Parses a result in the wire form of point 23 into a [`QueryResult`].
///
/// The reading itself lives in [`opengrid_datasource::wire`], because server and
/// client must read the same bytes the same way. What this adds is the grid's
/// error type: a malformed result is a message the status line can show, not a
/// panic.
///
/// Since point 23 the values arrive **typed** — a decimal is a decimal, not its
/// display text — which is what makes typed filters (point 51) possible.
pub fn parse_result(result_json: &str) -> Result<QueryResult, String> {
    opengrid_datasource::wire::result_from_json(result_json)
        .map_err(|error| error.message().to_owned())
}

/// The first logical row visible at `scroll_top`.
///
/// With rows at `r * row_height` and a sticky header, the row whose top is at or
/// below the scroll offset is `scroll_top / row_height`.
pub const fn visible_start(scroll_top: u64, row_height: u64) -> u64 {
    scroll_top / row_height
}

/// The window start for a visible row, keeping [`OVERSCAN`] rows above it.
///
/// Clamped to `[0, total_count - pool]` so the last window ends at the last row
/// and never runs past the result.
pub fn window_offset(visible_start: u64, total_count: u64, pool: u64) -> u64 {
    window_offset_for_row(visible_start, total_count, pool)
}

/// The window start that contains `row`, keeping [`OVERSCAN`] rows above it and
/// clamped so the window stays inside `[0, total_count)`.
///
/// `lead` shrinks with a small pool so the row is always inside the window even
/// when the pool is shorter than the overscan.
pub fn window_offset_for_row(row: u64, total_count: u64, pool: u64) -> u64 {
    let max_offset = total_count.saturating_sub(pool);
    let lead = OVERSCAN.min(pool.saturating_sub(1));
    row.saturating_sub(lead).min(max_offset)
}

/// The logical rows of a window, hidded by `total_count`.
pub fn window_rows(offset: u64, total_count: u64, pool: u64) -> Vec<u64> {
    let end = offset.saturating_add(pool).min(total_count);
    (offset..end).collect()
}

/// Assigns logical rows to pool slots, **pinning the focused row**.
///
/// Rules (point 17):
///
/// * A slot whose row is still inside `rows` keeps it, so scrolling reuses the
///   same node for the same logical row where possible.
/// * The slot that already holds `focus` always keeps it — even when the row
///   left the window — so the focused DOM node is never reassigned and focus
///   survives scrolling.
/// * Remaining rows fill the free slots in order. If the pinned row leaves no
///   room for the whole window, one window row is dropped from the edge farthest
///   from the focus (still invisible thanks to the overscan).
pub fn assign_pool(
    old: &[Option<u64>],
    focus: Option<u64>,
    rows: &[u64],
    pool: usize,
) -> Vec<Option<u64>> {
    use std::collections::HashSet;

    let row_set: HashSet<u64> = rows.iter().copied().collect();
    let mut new = vec![None; pool];
    for (slot, current) in old.iter().enumerate().take(pool) {
        if let Some(row) = current
            && (row_set.contains(row) || Some(*row) == focus)
        {
            new[slot] = Some(*row);
        }
    }

    let pinned = new.iter().filter(|slot| slot.is_some()).count();
    let capacity = pool.saturating_sub(pinned);
    let mut to_place: Vec<u64> = rows
        .iter()
        .copied()
        .filter(|row| !new.contains(&Some(*row)))
        .collect();
    if to_place.len() > capacity {
        let drop = to_place.len() - capacity;
        // The focus is below the window when it scrolls down past it: drop the
        // rows at the top (smallest), otherwise drop the bottom (largest).
        let focus_below = focus
            .map(|focus| rows.last().is_none_or(|last| focus > *last))
            .unwrap_or(false);
        if focus_below {
            to_place.drain(0..drop);
        } else {
            let keep = to_place.len() - drop;
            to_place.truncate(keep);
        }
    }

    let mut free: Vec<usize> = (0..pool).filter(|slot| new[*slot].is_none()).collect();
    free.reverse();
    for row in to_place {
        if let Some(slot) = free.pop() {
            new[slot] = Some(row);
        }
    }
    new
}

/// The active cell after pressing `key`, before any reload.
///
/// Movement is clamped as plan/spezifikation/09-accessibility.md §Tastatur im
/// Grid Mode asks: arrows stay inside the columns and the whole result, `Home`/
/// `End` stay on the row, the page keys move by `viewport_rows` and
/// `Ctrl+Home`/`Ctrl+End` jump to the first/last cell of the whole result. The
/// caller derives from the answer whether the window has to be reloaded and, if
/// so, to which offset.
pub fn move_active(
    active: ActiveCell,
    key: GridKey,
    ncols: usize,
    total_count: u64,
    viewport_rows: u64,
    selection: bool,
) -> ActiveCell {
    if ncols == 0 {
        return active;
    }
    let last_row = total_count.saturating_sub(1);

    // One axis for the whole row, so the selection column is a place rather
    // than a special case in every arm: `-1` is the selection cell, `0..ncols`
    // are the schema columns (point 61). Without the column the axis simply
    // starts at 0, and nothing else in here changes.
    const SELECT: isize = -1;
    let first_col = if selection { SELECT } else { 0 };
    let last_col = ncols as isize - 1;
    let clamp = |col: isize| col.clamp(first_col, last_col);

    let column_of = |cell: ActiveCell| -> isize {
        match cell {
            ActiveCell::SelectAll | ActiveCell::Select { .. } => SELECT,
            ActiveCell::Header { col } => col as isize,
            ActiveCell::Data(reference) => reference.col as isize,
        }
    };
    let header = |col: isize| match clamp(col) {
        SELECT => ActiveCell::SelectAll,
        col => ActiveCell::Header { col: col as usize },
    };
    let data = |row: u64, col: isize| match clamp(col) {
        SELECT => ActiveCell::Select {
            row: row.min(last_row),
        },
        col => ActiveCell::Data(CellRef::new(row.min(last_row), col as usize)),
    };
    let is_header = matches!(active, ActiveCell::SelectAll | ActiveCell::Header { .. });
    let row = active.row().unwrap_or(0);
    let col = column_of(active);

    match key {
        GridKey::ArrowUp => {
            if is_header {
                active
            } else if row == 0 {
                header(col)
            } else {
                data(row - 1, col)
            }
        }
        GridKey::ArrowDown => {
            if is_header {
                if total_count == 0 {
                    active
                } else {
                    data(0, col)
                }
            } else {
                data(row + 1, col)
            }
        }
        GridKey::ArrowLeft => {
            if is_header {
                header(col - 1)
            } else {
                data(row, col - 1)
            }
        }
        GridKey::ArrowRight => {
            if is_header {
                header(col + 1)
            } else {
                data(row, col + 1)
            }
        }
        // `Home` is the start of the row, and since point 61 that is the
        // selection cell — the same place the eye starts.
        GridKey::Home => {
            if is_header {
                header(first_col)
            } else {
                data(row, first_col)
            }
        }
        GridKey::End => {
            if is_header {
                header(last_col)
            } else {
                data(row, last_col)
            }
        }
        GridKey::CtrlHome => header(first_col),
        GridKey::CtrlEnd => {
            if total_count == 0 {
                header(last_col)
            } else {
                data(last_row, last_col)
            }
        }
        GridKey::PageUp => {
            if is_header {
                active
            } else {
                data(row.saturating_sub(viewport_rows), col)
            }
        }
        GridKey::PageDown => {
            if is_header {
                if total_count == 0 {
                    active
                } else {
                    data(viewport_rows.saturating_sub(1), col)
                }
            } else {
                data(row.saturating_add(viewport_rows), col)
            }
        }
    }
}

/// The window offset a key asks for, given the cell it moved to.
///
/// A move that lands inside the current window needs no reload (`None`).
/// `Ctrl+Home`/`Ctrl+End` name the first/last window directly; any other move to
/// a data cell outside the window asks for the window that contains it.
pub fn requested_window(
    key: GridKey,
    next: ActiveCell,
    window: Window,
    total_count: u64,
    pool: u64,
) -> Option<u64> {
    let wanted = match key {
        GridKey::CtrlHome => 0,
        GridKey::CtrlEnd if total_count > 0 => {
            window_offset_for_row(total_count - 1, total_count, pool)
        }
        _ => match next {
            ActiveCell::Data(cell) if !window.contains(cell.row) => {
                window_offset_for_row(cell.row, total_count, pool)
            }
            _ => window.offset,
        },
    };
    (wanted != window.offset).then_some(wanted)
}

/// Builds the empty grid skeleton (point 17): the scrollable viewport, the
/// `<table role="grid">` with its sticky header and a fixed pool of empty data
/// rows, all hidden until the first result arrives.
///
/// This runs exactly once per data-attribute configuration. [`patch_grid`] then
/// recycles the returned nodes for every frame.
/// Everything the skeleton is built from, in one place.
///
/// A struct rather than eight parameters, and not only because clippy counts:
/// the skeleton is what every later feature of phase F adds to — a selection
/// column (61), group rows (62), a column menu (64) — and each of those would
/// otherwise be one more positional argument at every call site.
pub struct GridSkeleton<'a> {
    pub label: Option<&'a str>,
    pub schema: &'a Schema,
    /// Number of recycled DOM rows.
    pub pool: usize,
    pub texts: &'a GridTexts,
    /// Declared columns with their visibility, for the column list.
    pub declared: &'a [(String, bool)],
    pub presentation: &'a crate::presentation::ColumnStyles,
    /// Whether the selection column is shown (point 61).
    pub selection: bool,
    /// Whether every header gets a column menu (point 64).
    pub column_menu: bool,
    /// Whether the toolbar is built (point 65).
    pub toolbar: bool,
    /// Whether the facet sidebar is built (point 66).
    pub facets: bool,
    /// Whether the search field is built (point 67).
    pub search: bool,
}

pub fn build_grid(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    skeleton: &GridSkeleton<'_>,
) -> GridNodes {
    let GridSkeleton {
        label,
        schema,
        pool,
        texts,
        declared,
        presentation,
        selection,
        column_menu,
        toolbar,
        facets,
        search,
    } = *skeleton;
    let fields = schema.fields();
    let ncols = fields.len();

    // One shadow-root stylesheet: the `--og-row-height` default plus the
    // positioning rules the virtualized rows need. The property is declared on
    // `:host` with the default and can be overridden from the document (or an
    // inline style) on the host; the inner elements inherit the resolved value.
    // The three densities, unpacked so the stylesheet below reads as CSS.
    let (compact_name, compact_row, compact_pad, compact_font) = DENSITIES[0];
    let (_, _, normal_pad, normal_font) = DENSITIES[1];
    let (comfy_name, comfy_row, comfy_pad, comfy_font) = DENSITIES[2];
    let styles = format!(
        ":host {{ {FONT_PROPERTY}: inherit;
                   {FONT_MONO_PROPERTY}: ui-monospace, SFMono-Regular, Menlo, monospace;
                   {FONT_SIZE_PROPERTY}: {normal_font};
                   {SURFACE_PROPERTY}: Canvas;
                   {SURFACE_2_PROPERTY}: Canvas;
                   {INK_PROPERTY}: CanvasText;
                   {INK_MUTED_PROPERTY}: color-mix(in oklab, CanvasText 62%, Canvas);
                   {LINE_PROPERTY}: color-mix(in oklab, CanvasText 14%, Canvas);
                   {LINE_STRONG_PROPERTY}: color-mix(in oklab, CanvasText 26%, Canvas);
                   /* `LinkText`, not `Highlight`: `Highlight` is the background of
                      a text selection — a pale colour meant to have dark text
                      on it — and the accent is also drawn *as* text
                      (`--og-accent-ink`). Found by axe in point 65 at 2.6:1. */
                   {ACCENT_PROPERTY}: LinkText;
                   {ON_ACCENT_PROPERTY}: Canvas;
                   {RADIUS_PROPERTY}: 0;
                   {PAD_PROPERTY}: {normal_pad}px;
                   {FOCUS_WIDTH_PROPERTY}: {DEFAULT_FOCUS_WIDTH};
                   {ROW_HEIGHT_PROPERTY}: {DEFAULT_ROW_HEIGHT}px;
                   {HEADER_HEIGHT_PROPERTY}: var({ROW_HEIGHT_PROPERTY});
                   {FILTER_HEIGHT_PROPERTY}: {FILTER_HEIGHT}px;
                   {STATUS_HEIGHT_PROPERTY}: {STATUS_HEIGHT}px;
                   /* Computed from the eleven above; reset under forced colors. */
                   {ACCENT_SOFT_PROPERTY}: color-mix(in oklab, var({ACCENT_PROPERTY}) 13%, var({SURFACE_PROPERTY}));
                   {ACCENT_INK_PROPERTY}: color-mix(in oklab, var({ACCENT_PROPERTY}) 80%, var({INK_PROPERTY}));
                   {SELECTED_PROPERTY}: color-mix(in oklab, var({ACCENT_PROPERTY}) 9%, var({SURFACE_PROPERTY}));
                   {HOVER_PROPERTY}: color-mix(in oklab, var({INK_PROPERTY}) 4%, var({SURFACE_PROPERTY}));
                   background: var({SURFACE_PROPERTY}); color: var({INK_PROPERTY});
                   font-family: var({FONT_PROPERTY}); font-size: var({FONT_SIZE_PROPERTY}); }}
         /* Density (point 58). `normal` is `:host` itself, so a grid without the
            attribute is a normal one rather than a fourth, nameless density. */
         :host([{DENSITY_ATTRIBUTE}=\"{compact_name}\"]) {{ {ROW_HEIGHT_PROPERTY}: {compact_row}px;
                   {PAD_PROPERTY}: {compact_pad}px; {FONT_SIZE_PROPERTY}: {compact_font}; }}
         :host([{DENSITY_ATTRIBUTE}=\"{comfy_name}\"]) {{ {ROW_HEIGHT_PROPERTY}: {comfy_row}px;
                   {PAD_PROPERTY}: {comfy_pad}px; {FONT_SIZE_PROPERTY}: {comfy_font}; }}
         [part=\"layout\"] {{ display: flex; flex-direction: column; height: 100%; min-height: 0; }}
         [part=\"filter\"] {{ display: flex; align-items: center; gap: 0.5rem; box-sizing: border-box;
                             flex: 0 0 auto;
                             height: var({FILTER_HEIGHT_PROPERTY}); padding: 0 var({PAD_PROPERTY});
                             background: var({SURFACE_2_PROPERTY});
                             border-bottom: 1px solid var({LINE_STRONG_PROPERTY});
                             overflow-x: auto; overflow-y: hidden; white-space: nowrap; }}
         [part=\"filter\"] select, [part=\"filter\"] input, [part=\"filter\"] button {{
                             font: inherit; min-height: {MIN_TARGET_SIZE}px;
                             color: var({INK_PROPERTY}); background: var({SURFACE_PROPERTY});
                             border-radius: min(var({RADIUS_PROPERTY}), 8px); }}
         td[data-changed] {{ font-style: italic; }}
         td[data-changed]::after {{ content: \" *\"; }}
         [part=\"editor\"] {{ font: inherit; width: 100%; box-sizing: border-box;
                             min-height: {MIN_TARGET_SIZE}px;
                             color: var({INK_PROPERTY});
                             border-radius: min(var({RADIUS_PROPERTY}), 8px); }}
         /* WebKit draws a native `select` at a height of its own — 20px in the
            compact density — and with a corner of its own, whatever `min-height`
            and the radius token say: under the 24px target and off the theme, in
            Safari alone. Only `appearance: none` hands the box to this sheet, and
            then the arrow is drawn here, as two gradient halves in the ink. The
            query is true in WebKit only; the other engines size their native
            select as asked, so they keep it. */
         @supports (font: -apple-system-body) {{
           [part=\"filter\"] select, select[part=\"editor\"] {{ appearance: none;
                             padding: 0 1.5em 0 0.3em; background-repeat: no-repeat;
                             background-image: linear-gradient(45deg, transparent 50%, currentColor 50%),
                                               linear-gradient(135deg, currentColor 50%, transparent 50%);
                             background-position: calc(100% - 0.9em) 55%, calc(100% - 0.55em) 55%;
                             background-size: 0.35em 0.35em; }}
         }}
         [part=\"columns\"] {{ display: flex; gap: 0.5rem; align-items: center; }}
         [part=\"columns\"][hidden] {{ display: none; }}
         [part=\"column-toggle\"] {{ display: inline-flex; gap: 0.25rem; align-items: center;
                             min-height: {MIN_TARGET_SIZE}px; }}
         [part=\"column-toggle\"] input {{ min-width: {MIN_TARGET_SIZE}px;
                             min-height: {MIN_TARGET_SIZE}px;
                             accent-color: var({ACCENT_PROPERTY}); }}
         [part=\"pager\"] {{ flex: 0 0 auto; display: flex; gap: 0.5rem; align-items: center;
                             padding: 0.25rem var({PAD_PROPERTY});
                             background: var({SURFACE_2_PROPERTY});
                             border-top: 1px solid var({LINE_STRONG_PROPERTY}); }}
         /* `display` beats the user agent's `[hidden] {{ display: none }}`, so
            the hidden pager has to be told again — otherwise it takes height
            and shrinks the viewport that PageUp/PageDown step by. */
         [part=\"pager\"][hidden] {{ display: none; }}
         [part=\"pager\"] button {{ font: inherit; min-height: {MIN_TARGET_SIZE}px;
                             min-width: {MIN_TARGET_SIZE}px;
                             color: var({INK_PROPERTY}); background: var({SURFACE_PROPERTY});
                             border-radius: min(var({RADIUS_PROPERTY}), 8px); }}
         /* No border of its own: the status line sits directly under the
            filter row, whose bottom border already separates the two. A top
            border here draws the same line twice. */
         [part=\"status\"] {{ flex: 0 0 auto; margin: 0; padding: 0 var({PAD_PROPERTY});
                             box-sizing: border-box;
                             min-height: var({STATUS_HEIGHT_PROPERTY});
                             background: var({SURFACE_2_PROPERTY});
                             color: var({INK_MUTED_PROPERTY}); }}
         [part=\"status\"][data-state=\"error\"] {{ font-weight: bold; color: var({INK_PROPERTY}); }}
         [part=\"viewport\"] {{ flex: 1 1 0; min-height: 0; overflow-y: auto; position: relative; display: block; }}
         [part=\"sort-direction\"], [part=\"sort-index\"] {{ margin-left: 0.25rem; font-size: 0.75em;
                             color: var({ACCENT_INK_PROPERTY}); }}
         /* An empty mark must not reserve space: an unsorted header would
            otherwise truncate its name earlier than the cells below it. */
         [part=\"sort-direction\"]:empty, [part=\"sort-index\"]:empty {{ margin-left: 0; }}
         /* The marks are last in the header, so a name wider than its column
            would push them out of the clipped box — and the direction would be
            invisible exactly where it is needed (point 49). Letting the *name*
            ellipsize instead keeps them: measured, a flex header would drop out
            of the table layout and break the column alignment. */
         [part=\"header\"] > span:first-child {{ display: inline-block; max-width: 100%;
                   overflow: hidden; text-overflow: ellipsis; vertical-align: bottom; }}
         [part=\"header\"][aria-sort=\"ascending\"] > span:first-child,
         [part=\"header\"][aria-sort=\"descending\"] > span:first-child {{ max-width: calc(100% - 2.75em); }}
         table {{ width: 100%; table-layout: fixed; border-collapse: collapse; }}
         thead {{ position: sticky; top: 0; z-index: 2;
                  background: var({SURFACE_2_PROPERTY}); color: var({INK_PROPERTY}); }}
         tbody tr {{ position: absolute; left: 0; width: 100%; display: table; table-layout: fixed;
                     background: var({SURFACE_PROPERTY}); }}
         tbody tr:hover {{ background: var({HOVER_PROPERTY}); }}
         /* Until now a selected row was announced and invisible. The tint alone
            would be 1.4.1, so the bar carries the information and the tint only
            helps it. Selected beats hovered, hence the order. */
         tbody tr[data-selected=\"true\"] {{ background: var({SELECTED_PROPERTY});
                     box-shadow: inset 3px 0 0 var({ACCENT_PROPERTY}); }}
         tbody tr:has(:focus) {{ z-index: 1; }}
         tbody td:focus {{ white-space: normal; overflow-wrap: anywhere; }}
         tbody td {{ scroll-margin-top: var({HEADER_HEIGHT_PROPERTY}); }}
         th, td {{ box-sizing: border-box; padding: 0 var({PAD_PROPERTY});
                   border-bottom: 1px solid var({LINE_PROPERTY});
                   white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
         td {{ height: var({ROW_HEIGHT_PROPERTY}); }}
         /* The selection column (point 61). A fixed, narrow column: its
            content is one glyph, and letting it share the table's flexible
            width would make it move as the data changes. */
         [part=\"select\"], [part=\"select-all\"] {{ width: 44px; text-align: center;
                   padding: 0;
                   cursor: pointer; }}
         [part=\"select-mark\"] {{ position: relative; display: inline-flex;
                   align-items: center; justify-content: center;
                   width: 16px; height: 16px; box-sizing: border-box;
                   border: 1.5px solid var({LINE_STRONG_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 4px);
                   font-size: 0.75em; line-height: 1; }}
         /* The drawn box stays 16px — that is the design. The **target** is the
            whole cell, which is 44px wide and a row tall: WCAG 2.2 §2.5.8
            measures what a pointer can hit, not what it can see. */
         [part=\"select-mark\"][aria-checked=\"true\"],
         [part=\"select-mark\"][aria-checked=\"mixed\"],
         tr[data-selected=\"true\"] [part=\"select-mark\"] {{
                   border-color: var({ACCENT_PROPERTY});
                   background: var({ACCENT_PROPERTY}); color: var({ON_ACCENT_PROPERTY}); }}
         /* Group rows (point 62). The label runs across the empty cells beside
            it — they have no background, so the text stays visible. The
            chevron is drawn from `aria-expanded` with an empty alternative
            text: it is seen, and not read a second time after the state. */
         tr[data-kind=\"group\"] {{ background: var({SURFACE_2_PROPERTY}); font-weight: 600; }}
         /* Left-aligned whatever the column is: the label is a sentence, and in
            a right-aligned number column it would overflow to the left, out of
            the row. */
         tr[data-kind=\"group\"] td[data-col=\"0\"] {{ overflow: visible; text-align: left; }}
         tr[data-kind=\"group\"] td[data-col=\"0\"]::before {{ content: \"\\25B8\" / \"\";
                   display: inline-block; width: 1.25em; color: var({INK_MUTED_PROPERTY}); }}
         tr[data-kind=\"group\"][aria-expanded=\"true\"] td[data-col=\"0\"]::before {{
                   content: \"\\25BE\" / \"\"; }}
         tr[data-kind=\"group\"][data-level=\"2\"] td[data-col=\"0\"] {{
                   padding-left: calc(var({PAD_PROPERTY}) + 1.25em); }}
         /* Aggregates (point 63): the glyph is drawn, with an empty
            alternative — the cell's `aria-label` says the word. */
         tr[data-kind=\"total\"] {{ background: var({SURFACE_2_PROPERTY}); font-weight: 600;
                   border-top: 1px solid var({LINE_STRONG_PROPERTY}); }}
         tr[data-kind=\"total\"] td[data-col=\"0\"] {{ overflow: visible; text-align: left; }}
         td[data-aggregate]::before, th[data-aggregate]::after {{
                   color: var({ACCENT_INK_PROPERTY}); font-weight: 500; }}
         td[data-aggregate]::before {{ margin-right: 0.35em; }}
         th[data-aggregate]::after {{ margin-left: 0.35em; }}
         [data-aggregate=\"sum\"]::before, th[data-aggregate=\"sum\"]::after {{ content: \"\\03A3\" / \"\"; }}
         [data-aggregate=\"avg\"]::before, th[data-aggregate=\"avg\"]::after {{ content: \"\\2300\" / \"\"; }}
         [data-aggregate=\"count\"]::before, th[data-aggregate=\"count\"]::after {{ content: \"#\" / \"\"; }}
         [data-aggregate=\"min\"]::before, th[data-aggregate=\"min\"]::after {{ content: \"min\" / \"\"; }}
         [data-aggregate=\"max\"]::before, th[data-aggregate=\"max\"]::after {{ content: \"max\" / \"\"; }}
         /* A range needs no glyph in its cell — the dash between its ends says
            it — but the header says which summary the column shows (F7). */
         th[data-aggregate=\"range\"]::after {{ content: \"\\2194\" / \"\"; }}
         th[data-aggregate]::before {{ content: none; }}
         /* The column menu (point 64). The trigger is 24px square (2.5.8) and
            sits at the end of the header; the name gives way to it. */
         [part=\"column-menu-button\"] {{ display: inline-flex; align-items: center;
                   justify-content: center; width: {MIN_TARGET_SIZE}px; height: {MIN_TARGET_SIZE}px;
                   margin-left: 0.25rem; vertical-align: middle; cursor: pointer;
                   border-radius: min(var({RADIUS_PROPERTY}), 5px); color: var({INK_MUTED_PROPERTY}); }}
         [part=\"column-menu-button\"]:hover {{ background: var({HOVER_PROPERTY}); }}
         [part=\"header\"]:has([part=\"column-menu-button\"]) > span:first-child {{
                   max-width: calc(100% - {MIN_TARGET_SIZE}px - 0.5rem); }}
         [part=\"header\"][aria-sort=\"ascending\"]:has([part=\"column-menu-button\"]) > span:first-child,
         [part=\"header\"][aria-sort=\"descending\"]:has([part=\"column-menu-button\"]) > span:first-child {{
                   max-width: calc(100% - 2.75em - {MIN_TARGET_SIZE}px - 0.5rem); }}
         [part=\"column-menu\"] {{ position: fixed; inset: auto; margin: 0; padding: 6px;
                   min-width: 14rem; box-sizing: border-box;
                   background: var({SURFACE_PROPERTY}); color: var({INK_PROPERTY});
                   border: 1px solid var({LINE_STRONG_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 10px);
                   box-shadow: 0 12px 32px rgb(0 0 0 / 0.18);
                   font-family: var({FONT_PROPERTY}); font-size: var({FONT_SIZE_PROPERTY}); }}
         [part=\"column-menu\"] [role^=\"menuitem\"] {{ display: flex; align-items: center;
                   gap: 0.5rem; min-height: 32px; padding: 0 8px; cursor: pointer;
                   border-radius: min(var({RADIUS_PROPERTY}), 6px); }}
         [part=\"column-menu\"] [role^=\"menuitem\"]:hover {{ background: var({HOVER_PROPERTY}); }}
         [part=\"column-menu\"] [role^=\"menuitem\"]:focus {{
                   outline: var({FOCUS_WIDTH_PROPERTY}) solid Highlight;
                   outline-offset: calc(-1 * var({FOCUS_WIDTH_PROPERTY})); }}
         [part=\"column-menu\"] [role=\"menuitemradio\"]::before {{ content: \"\" / \"\";
                   display: inline-block; width: 1em; }}
         [part=\"column-menu\"] [role=\"menuitemradio\"][aria-checked=\"true\"]::before {{
                   content: \"\\2713\" / \"\"; color: var({ACCENT_INK_PROPERTY}); }}
         [part=\"column-menu\"] [role=\"separator\"] {{ height: 1px; margin: 5px 0;
                   background: var({LINE_PROPERTY}); }}
         [part=\"column-menu\"] [role=\"group\"] > [part=\"menu-label\"] {{
                   padding: 4px 8px; font-size: 0.75em; color: var({INK_MUTED_PROPERTY});
                   text-transform: uppercase; letter-spacing: 0.05em; }}
         /* The toolbar and the chips (point 65). Each hidden group says
            `display: none` again: `display` beats `[hidden]` (phase E (h)), and
            a hidden row that kept its height would shrink the viewport. */
         [part=\"toolbar\"] {{ display: flex; flex-wrap: wrap; align-items: center;
                   gap: 0.5rem; padding: 0.375rem var({PAD_PROPERTY});
                   border-bottom: 1px solid var({LINE_PROPERTY}); }}
         [part=\"toolbar\"] button, [part=\"chips\"] button {{ font: inherit;
                   min-height: {MIN_TARGET_SIZE}px; min-width: {MIN_TARGET_SIZE}px;
                   color: var({INK_PROPERTY}); background: var({SURFACE_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 8px); }}
         [part=\"toolbar\"] button[aria-pressed=\"true\"] {{ background: var({ACCENT_SOFT_PROPERTY});
                   color: var({ACCENT_INK_PROPERTY}); }}
         [part=\"density\"] {{ display: inline-flex; gap: 2px; margin-left: auto; }}
         [part=\"chips\"] {{ display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem;
                   padding: 0.375rem var({PAD_PROPERTY}); }}
         [part=\"chips\"][hidden], [part=\"filter\"][hidden] {{ display: none; }}
         [part=\"chip\"] {{ display: inline-flex; align-items: center; gap: 0.25rem;
                   padding: 0 0 0 0.75rem; border-radius: 999px;
                   background: var({ACCENT_SOFT_PROPERTY}); color: var({ACCENT_INK_PROPERTY}); }}
         [part=\"chips\"] [part=\"chip-remove\"] {{ border: 0; background: transparent; border-radius: 999px;
                   cursor: pointer; }}
         /* The facet sidebar (point 66). */
         [part=\"body\"] {{ flex: 1 1 0; min-height: 0; display: flex; }}
         [part=\"body\"] > [part=\"viewport\"] {{ flex: 1 1 0; min-width: 0; }}
         [part=\"facets\"] {{ flex: 0 0 15.5rem; overflow-y: auto; box-sizing: border-box;
                   padding: 0.75rem var({PAD_PROPERTY}); display: flex; flex-direction: column;
                   gap: 1rem; background: var({SURFACE_2_PROPERTY});
                   border-right: 1px solid var({LINE_PROPERTY}); }}
         [part=\"facets-head\"] {{ display: flex; justify-content: space-between;
                   align-items: center; gap: 0.5rem; }}
         [part=\"facets-head\"] button, [part=\"facet\"] button {{ font: inherit;
                   min-height: {MIN_TARGET_SIZE}px; color: var({INK_PROPERTY});
                   background: var({SURFACE_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 8px); }}
         [part=\"facet-cost\"] {{ color: var({INK_MUTED_PROPERTY}); font-size: 0.85em; }}
         [part=\"facet\"] {{ border: 0; margin: 0; padding: 0; display: flex;
                   flex-direction: column; gap: 0.25rem; min-width: 0; }}
         [part=\"facet\"] legend {{ padding: 0 0 0.25rem; font-size: 0.85em;
                   color: var({INK_MUTED_PROPERTY}); }}
         [part=\"facet-value\"] {{ display: flex; align-items: center; gap: 0.5rem;
                   min-height: {MIN_TARGET_SIZE}px; }}
         [part=\"facet-value\"] input {{ min-width: {MIN_TARGET_SIZE}px;
                   min-height: {MIN_TARGET_SIZE}px; margin: 0; accent-color: var({ACCENT_PROPERTY}); }}
         [part=\"facet-value\"] [part=\"facet-count\"], [part=\"facet-pill\"] [part=\"facet-count\"] {{
                   margin-left: auto; font-family: var({FONT_MONO_PROPERTY}); font-size: 0.85em;
                   color: var({INK_MUTED_PROPERTY}); }}
         [part=\"facet-pills\"] {{ display: flex; flex-wrap: wrap; gap: 0.375rem; }}
         [part=\"facet-pill\"] {{ display: inline-flex; gap: 0.375rem; align-items: center;
                   border: 1px solid var({LINE_STRONG_PROPERTY}); background: var({SURFACE_PROPERTY}); }}
         [part=\"facet-pill\"][aria-pressed=\"true\"] {{ border-color: var({ACCENT_PROPERTY});
                   background: var({ACCENT_SOFT_PROPERTY}); color: var({ACCENT_INK_PROPERTY}); }}
         [part=\"facet-bounds\"] {{ display: grid; grid-template-columns: 1fr 1fr; gap: 0.375rem; }}
         [part=\"facet-bounds\"] label {{ display: flex; flex-direction: column; gap: 0.125rem;
                   font-size: 0.85em; color: var({INK_MUTED_PROPERTY}); min-width: 0; }}
         [part=\"facet-bounds\"] input {{ font: inherit; min-height: {MIN_TARGET_SIZE}px;
                   min-width: 0; color: var({INK_PROPERTY}); background: var({SURFACE_PROPERTY}); }}
         /* The search field (point 67). */
         [part=\"search\"] {{ position: relative; display: flex; align-items: center; gap: 0.5rem;
                   padding: 0.375rem var({PAD_PROPERTY}); }}
         [part=\"search-input\"] {{ flex: 1 1 18rem; max-width: 32rem; font: inherit;
                   min-height: 32px; box-sizing: border-box; padding: 0 0.625rem;
                   color: var({INK_PROPERTY}); background: var({SURFACE_2_PROPERTY});
                   border: 1px solid var({LINE_STRONG_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 9px); }}
         [part=\"search-input\"][data-query] {{ font-family: var({FONT_MONO_PROPERTY}); }}
         [part=\"search-hint\"] {{ font-size: 0.75em; padding: 0.2em 0.5em; border-radius: 5px;
                   background: var({ACCENT_SOFT_PROPERTY}); color: var({ACCENT_INK_PROPERTY}); }}
         [part=\"search-hint\"][hidden], [part=\"search-list\"][hidden] {{ display: none; }}
         [part=\"search-list\"] {{ position: absolute; top: 100%; left: var({PAD_PROPERTY}); z-index: 5;
                   margin: 0; padding: 6px; list-style: none; min-width: 16rem;
                   background: var({SURFACE_PROPERTY}); border: 1px solid var({LINE_STRONG_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 10px);
                   box-shadow: 0 12px 32px rgb(0 0 0 / 0.16); }}
         [part=\"search-list\"] [role=\"option\"] {{ display: flex; justify-content: space-between;
                   gap: 1rem; min-height: 32px; align-items: center; padding: 0 8px;
                   border-radius: 6px; cursor: pointer; font-family: var({FONT_MONO_PROPERTY}); }}
         [part=\"search-list\"] [role=\"option\"][aria-selected=\"true\"] {{
                   background: var({ACCENT_SOFT_PROPERTY}); color: var({ACCENT_INK_PROPERTY}); }}
         /* The empty state (point 68). */
         [part=\"empty\"] {{ display: flex; flex-direction: column; align-items: center;
                   gap: 0.75rem; padding: 3rem 1.5rem; text-align: center; }}
         [part=\"empty\"][hidden], [part=\"empty-reset\"][hidden] {{ display: none; }}
         [part=\"empty-text\"] {{ margin: 0; color: var({INK_MUTED_PROPERTY}); }}
         [part=\"empty-reset\"] {{ font: inherit; min-height: 32px; padding: 0 0.875rem;
                   color: var({INK_PROPERTY}); background: var({SURFACE_PROPERTY});
                   border: 1px solid var({LINE_STRONG_PROPERTY});
                   border-radius: min(var({RADIUS_PROPERTY}), 8px); }}
         /* Column presentation (point 60). Markers rather than inline styles,
            so the sheet decides what \"muted\" looks like and a page can still
            reach the cell through `::part(cell)`. */
         [data-align=\"end\"] {{ text-align: right; }}
         [data-align=\"center\"] {{ text-align: center; }}
         [data-align=\"start\"] {{ text-align: left; }}
         /* Tabular figures on every number, not only on the mono columns:
            digits of the same magnitude have to stand under each other or the
            column cannot be read down. */
         td[data-align=\"end\"] {{ font-variant-numeric: tabular-nums; }}
         /* Values only: a group or total row reuses the pooled cells, and its
            label in the first column is not an `id` because the column is. */
         tr:not([data-kind]) td[data-mono] {{ font-family: var({FONT_MONO_PROPERTY}); }}
         tr:not([data-kind]) td[data-emphasis] {{ font-weight: 600; }}
         tr:not([data-kind]) td[data-muted] {{ color: var({INK_MUTED_PROPERTY}); }}
         th {{ height: var({HEADER_HEIGHT_PROPERTY}); background: var({SURFACE_2_PROPERTY});
               color: var({INK_MUTED_PROPERTY}); text-align: left; }}
         /* The focus ring never uses the accent: a pale accent would make it
            invisible, and the ring is not decoration. */
         th:focus, td:focus {{ outline: var({FOCUS_WIDTH_PROPERTY}) solid Highlight;
                               outline-offset: calc(-1 * var({FOCUS_WIDTH_PROPERTY})); }}
         /* Under a forced palette every computed colour is a system colour: a
            `color-mix` of two system colours resolves unpredictably, and the
            user's palette is the one that has to win. */
         @media (forced-colors: active) {{
           :host {{ {SURFACE_PROPERTY}: Canvas; {SURFACE_2_PROPERTY}: Canvas;
                    {INK_PROPERTY}: CanvasText; {INK_MUTED_PROPERTY}: CanvasText;
                    {LINE_PROPERTY}: CanvasText; {LINE_STRONG_PROPERTY}: CanvasText;
                    {ACCENT_PROPERTY}: Highlight; {ON_ACCENT_PROPERTY}: HighlightText;
                    {ACCENT_SOFT_PROPERTY}: Canvas; {ACCENT_INK_PROPERTY}: CanvasText;
                    {SELECTED_PROPERTY}: Canvas; {HOVER_PROPERTY}: Canvas; }}
           /* A forced palette drops background images, and with them WebKit's
              drawn select arrow (above): the native select comes back, so it
              keeps its sign. Elsewhere the select is native already. */
           [part=\"filter\"] select, select[part=\"editor\"] {{ appearance: auto;
                             background-image: none; }}
         }}
         @media (prefers-reduced-motion: reduce) {{
           * {{ animation-duration: 0.01ms !important; animation-iteration-count: 1 !important;
                transition-duration: 0.01ms !important; scroll-behavior: auto !important; }}
         }}"
    );
    let style = element(buffer, nodes, Some(NodeId::ROOT), "style");
    buffer.push(Patch::SetText {
        node: style,
        text: styles,
    });

    let layout = element(buffer, nodes, Some(NodeId::ROOT), "div");
    buffer.push(Patch::SetAttribute {
        node: layout,
        name: "part".to_owned(),
        value: "layout".to_owned(),
    });
    // The search field (point 67), first: it is where a reader starts.
    if search {
        build_search(buffer, nodes, layout, texts);
    }
    // The toolbar and the chips (point 65), above the filter row.
    let tools = toolbar.then(|| {
        build_toolbar(
            buffer,
            nodes,
            layout,
            texts,
            !presentation.facets().is_empty(),
            facets,
        )
    });
    let filter = build_filter(buffer, nodes, layout, fields, texts);
    let status = build_status(buffer, nodes, layout, texts);

    // Inside the filter row, not above it: a new row of its own would shrink
    // the viewport, and the viewport is what `PageUp`/`PageDown` step by. With a
    // toolbar it moves there, where the prototype has it — and where it stays
    // reachable when the reader hides the filter row.
    let columns = build_columns(
        buffer,
        nodes,
        tools.unwrap_or(filter.container),
        declared,
        texts,
    );
    let pager = build_pager(buffer, nodes, layout, texts);

    // With facets, the viewport shares a row with the sidebar (point 66).
    // Without, the skeleton is exactly what it was: the wrapper would be one
    // more box around the viewport, and the viewport's height is what the
    // window math and `PageDown` are made of.
    let viewport_parent = if facets {
        let body = element(buffer, nodes, Some(layout), "div");
        buffer.push(Patch::SetAttribute {
            node: body,
            name: "part".to_owned(),
            value: "body".to_owned(),
        });
        let sidebar = element(buffer, nodes, Some(body), "div");
        // A scroller: out of the tab sequence, like the viewport below.
        for (name, value) in [("part", "facets"), ("role", "group"), ("tabindex", "-1")] {
            buffer.push(Patch::SetAttribute {
                node: sidebar,
                name: name.to_owned(),
                value: value.to_owned(),
            });
        }
        label_by(
            buffer,
            nodes,
            body,
            sidebar,
            "og-label-facets",
            &texts.facets_group,
            texts,
        );
        body
    } else {
        layout
    };
    let viewport = element(buffer, nodes, Some(viewport_parent), "div");
    buffer.push(Patch::SetAttribute {
        node: viewport,
        name: "part".to_owned(),
        value: "viewport".to_owned(),
    });
    // Firefox makes every scroll container a tab stop of its own, focusable
    // children or not: Shift+Tab from the header landed on this unnamed div
    // before reaching the filter row. The grid is one tab stop, and its keys
    // do the scrolling; `-1` takes the div out of the sequence and nothing else.
    // The filter row and the facets scroll too, and get the same.
    buffer.push(Patch::SetAttribute {
        node: viewport,
        name: "tabindex".to_owned(),
        value: "-1".to_owned(),
    });

    let table = element(buffer, nodes, Some(viewport), "table");
    buffer.push(Patch::SetAttribute {
        node: table,
        name: "role".to_owned(),
        value: "grid".to_owned(),
    });
    if let Some((name, value)) = mirror_label(label) {
        buffer.push(Patch::SetAttribute {
            node: table,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    buffer.push(Patch::SetAttribute {
        node: table,
        name: "aria-rowcount".to_owned(),
        value: "1".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: table,
        name: "aria-colcount".to_owned(),
        // The selection column counts when it is there: a reader told there are
        // five would look for a sixth.
        value: (ncols + usize::from(selection)).to_string(),
    });

    let thead = element(buffer, nodes, Some(table), "thead");
    let header_row = element(buffer, nodes, Some(thead), "tr");
    buffer.push(Patch::SetAttribute {
        node: header_row,
        name: "aria-rowindex".to_owned(),
        value: "1".to_owned(),
    });
    let (select_all, select_all_mark) = if selection {
        // The selection column's header (point 61). A `<th>` like any other, so
        // the arrow keys reach it and a screen reader counts it; `Enter`/`Space` on
        // it selects every matching row — the same promise `Ctrl`+`A` already made,
        // now visible.
        let select_all = element(buffer, nodes, Some(header_row), "th");
        for (name, value) in [
            ("part", "select-all"),
            ("scope", "col"),
            // On the **cell**: this is what a pointer hits (44px wide, a row tall),
            // and WCAG 2.2 §2.5.8 measures the target, not the drawn box. The
            // widget below carries the role and the state.
            ("data-select", "all"),
        ] {
            buffer.push(Patch::SetAttribute {
                node: select_all,
                name: name.to_owned(),
                value: value.to_owned(),
            });
        }
        // The checkbox is the **span**, not the cell. `aria-checked` on a
        // `columnheader` is not allowed (axe says so, and it is right): a cell is
        // not a widget. The grid pattern answers this directly — when a cell holds
        // a single widget, the widget is the focusable element — so the roving
        // tabindex lands here and the cell stays a plain header.
        let select_all_mark = element(buffer, nodes, Some(select_all), "span");
        for (name, value) in [
            ("role", "checkbox"),
            ("part", "select-mark"),
            ("tabindex", "-1"),
            ("aria-checked", "false"),
        ] {
            buffer.push(Patch::SetAttribute {
                node: select_all_mark,
                name: name.to_owned(),
                value: value.to_owned(),
            });
        }
        // A word, not the glyph: "✓" read aloud is not a promise anybody can act
        // on, and what this selects is **every matching row**, not the page.
        buffer.push(Patch::SetAttribute {
            node: select_all_mark,
            name: "aria-label".to_owned(),
            value: texts.select_all.clone(),
        });
        // The label is our sentence, so the language sits here — and not on the
        // `<th>`, whose siblings hold the page's column names (point 39, and the
        // lesson of phase E (k)).
        set_lang(buffer, select_all_mark, texts);
        (Some(select_all), Some(select_all_mark))
    } else {
        (None, None)
    };

    let mut header_cells = Vec::with_capacity(ncols);
    for (col, field) in fields.iter().enumerate() {
        let th = element(buffer, nodes, Some(header_row), "th");
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "part".to_owned(),
            value: "header".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "scope".to_owned(),
            value: "col".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "data-col".to_owned(),
            value: col.to_string(),
        });
        // The header lines up with the values under it, or the column reads
        // as two columns.
        for (marker, value) in presentation.markers(field.name.as_str(), field.data_type) {
            buffer.push(Patch::SetAttribute {
                node: th,
                name: marker.to_owned(),
                value,
            });
        }
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "aria-sort".to_owned(),
            value: "none".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: th,
            name: "tabindex".to_owned(),
            value: "-1".to_owned(),
        });
        // The column name lives in its own span so the sort marks can be added
        // as further, `aria-hidden` spans without touching the name: the header's
        // accessible name stays the column, and the direction is announced once,
        // through `aria-sort`. Reading order is name, direction, order index —
        // "customer ▲ 2".
        let name = element(buffer, nodes, Some(th), "span");
        buffer.push(Patch::SetText {
            node: name,
            text: field.name.as_str().to_owned(),
        });
        let direction = marker(buffer, nodes, th, "sort-direction");
        let index = marker(buffer, nodes, th, "sort-index");
        // The column menu (point 64): a pointer target only. The keyboard path
        // is `Alt`+`↓` on the cell itself, said by `aria-keyshortcuts` — a
        // focusable button in here would be a second tab stop inside the grid,
        // next to the cell that already holds the roving tabindex.
        if column_menu {
            buffer.push(Patch::SetAttribute {
                node: th,
                name: "aria-keyshortcuts".to_owned(),
                value: "Alt+ArrowDown".to_owned(),
            });
            let button = element(buffer, nodes, Some(th), "span");
            for (name, value) in [("part", "column-menu-button"), ("aria-hidden", "true")] {
                buffer.push(Patch::SetAttribute {
                    node: button,
                    name: name.to_owned(),
                    value: value.to_owned(),
                });
            }
            buffer.push(Patch::SetText {
                node: button,
                text: "\u{22EF}".to_owned(),
            });
        }
        header_cells.push(GridHeaderNodes {
            cell: th,
            direction,
            index,
        });
    }

    let tbody = element(buffer, nodes, Some(table), "tbody");
    set_style(buffer, tbody, "position: relative; height: 0px;");

    let mut rows = Vec::with_capacity(pool);
    for _ in 0..pool {
        let tr = element(buffer, nodes, Some(tbody), "tr");
        buffer.push(Patch::SetAttribute {
            node: tr,
            name: "part".to_owned(),
            value: "row".to_owned(),
        });
        set_style(buffer, tr, ROW_HIDDEN_STYLE);
        let mut cells = Vec::with_capacity(ncols);
        let (select, select_mark) = if selection {
            let select = element(buffer, nodes, Some(tr), "td");
            for (name, value) in [
                ("part", "select"),
                ("data-select", "row"),
                ("tabindex", "-1"),
            ] {
                buffer.push(Patch::SetAttribute {
                    node: select,
                    name: name.to_owned(),
                    value: value.to_owned(),
                });
            }
            let select_mark = element(buffer, nodes, Some(select), "span");
            // Decoration: the row already says whether it is selected
            // (`aria-selected`), and a second voice per row would double every
            // announcement.
            buffer.push(Patch::SetAttribute {
                node: select_mark,
                name: "aria-hidden".to_owned(),
                value: "true".to_owned(),
            });
            buffer.push(Patch::SetAttribute {
                node: select_mark,
                name: "part".to_owned(),
                value: "select-mark".to_owned(),
            });
            (Some(select), Some(select_mark))
        } else {
            (None, None)
        };

        for (col, field) in fields.iter().enumerate() {
            let td = element(buffer, nodes, Some(tr), "td");
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "part".to_owned(),
                value: "cell".to_owned(),
            });
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "data-col".to_owned(),
                value: col.to_string(),
            });
            // The presentation of point 60. It belongs in the skeleton, not in
            // the per-frame patch: a pool cell always shows the same column, so
            // writing these once is the whole cost of them.
            for (marker, value) in presentation.markers(field.name.as_str(), field.data_type) {
                buffer.push(Patch::SetAttribute {
                    node: td,
                    name: marker.to_owned(),
                    value,
                });
            }
            buffer.push(Patch::SetAttribute {
                node: td,
                name: "tabindex".to_owned(),
                value: "-1".to_owned(),
            });
            cells.push(td);
        }
        rows.push(GridRowNodes {
            row: tr,
            select,
            select_mark,
            cells,
        });
    }

    // The empty state (point 68): drawn by the element when a result has no
    // rows. It sits in the viewport, under the header, where the rows would
    // be — and says nothing aloud: the status line already said "No matches",
    // and a second voice would be the doubling phase E (o) made visible.
    let empty = element(buffer, nodes, Some(viewport), "div");
    for (name, value) in [("part", "empty"), ("hidden", "")] {
        buffer.push(Patch::SetAttribute {
            node: empty,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    set_lang(buffer, empty, texts);
    let sentence = element(buffer, nodes, Some(empty), "p");
    buffer.push(Patch::SetAttribute {
        node: sentence,
        name: "part".to_owned(),
        value: "empty-text".to_owned(),
    });
    let reset = element(buffer, nodes, Some(empty), "button");
    for (name, value) in [
        ("type", "button"),
        ("part", "empty-reset"),
        ("data-empty-reset", ""),
    ] {
        buffer.push(Patch::SetAttribute {
            node: reset,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    buffer.push(Patch::SetText {
        node: reset,
        text: texts.empty_reset.clone(),
    });

    GridNodes {
        filter,
        status,
        viewport,
        tbody,
        table,
        select_all,
        select_all_mark,
        header_cells,
        rows,
        pager,
        columns,
        empty,
    }
}

/// Builds the search field (point 67): an ARIA combobox — the text field, a
/// listbox of column suggestions it controls, and the hint that says an input
/// reads as a filter. The listbox is filled by the element as the reader
/// types; `aria-activedescendant` moves through it while the focus stays in the
/// field, as the combobox pattern has it.
fn build_search(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    texts: &GridTexts,
) {
    let attribute = |buffer: &mut PatchBuffer, node: NodeId, name: &str, value: &str| {
        buffer.push(Patch::SetAttribute {
            node,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    };
    let row = element(buffer, nodes, Some(parent), "div");
    attribute(buffer, row, "part", "search");

    let input = element(buffer, nodes, Some(row), "input");
    for (name, value) in [
        ("type", "text"),
        ("part", "search-input"),
        ("role", "combobox"),
        ("aria-autocomplete", "list"),
        ("aria-expanded", "false"),
        ("aria-controls", "og-search-list"),
        ("autocomplete", "off"),
        ("spellcheck", "false"),
    ] {
        attribute(buffer, input, name, value);
    }
    label_by(
        buffer,
        nodes,
        row,
        input,
        "og-label-search",
        &texts.search_label,
        texts,
    );
    attribute(buffer, input, "placeholder", &texts.search_placeholder);

    let hint = element(buffer, nodes, Some(row), "span");
    attribute(buffer, hint, "part", "search-hint");
    attribute(buffer, hint, "hidden", "");
    set_lang(buffer, hint, texts);
    buffer.push(Patch::SetText {
        node: hint,
        text: texts.search_hint.clone(),
    });

    let list = element(buffer, nodes, Some(row), "ul");
    for (name, value) in [
        ("part", "search-list"),
        ("role", "listbox"),
        ("id", "og-search-list"),
        ("hidden", ""),
    ] {
        attribute(buffer, list, name, value);
    }
    label_by(
        buffer,
        nodes,
        row,
        list,
        "og-label-suggestions",
        &texts.search_suggestions,
        texts,
    );
}

/// Builds the toolbar and the (empty) chip group (point 65); answers the
/// toolbar's node, into which the column list is then put.
///
/// A labelled `group` of ordinary buttons, **outside** `role="grid"`, like the
/// filter row: `role="toolbar"` would promise arrow-key movement between the
/// controls, and a promise not kept is worse than the plainer role. The chips
/// are filled by the element from the view, because the filters live in the
/// filter row's fields rather than in the grid state.
fn build_toolbar(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    texts: &GridTexts,
    has_facets: bool,
    facets_shown: bool,
) -> NodeId {
    let attribute = |buffer: &mut PatchBuffer, node: NodeId, name: &str, value: &str| {
        buffer.push(Patch::SetAttribute {
            node,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    };
    let bar = element(buffer, nodes, Some(parent), "div");
    attribute(buffer, bar, "part", "toolbar");
    attribute(buffer, bar, "role", "group");
    label_by(
        buffer,
        nodes,
        parent,
        bar,
        "og-label-toolbar",
        &texts.toolbar_group,
        texts,
    );

    let toggle = element(buffer, nodes, Some(bar), "button");
    attribute(buffer, toggle, "type", "button");
    attribute(buffer, toggle, "part", "filter-row-toggle");
    attribute(buffer, toggle, "data-toolbar", "filter-row");
    attribute(buffer, toggle, "aria-pressed", "true");
    buffer.push(Patch::SetText {
        node: toggle,
        text: texts.filter_row_toggle.clone(),
    });
    set_lang(buffer, toggle, texts);

    // The facet switch, only where there are facets to show (point 66).
    if has_facets {
        let switch = element(buffer, nodes, Some(bar), "button");
        attribute(buffer, switch, "type", "button");
        attribute(buffer, switch, "part", "facets-toggle");
        attribute(buffer, switch, "data-toolbar", "facets");
        attribute(buffer, switch, "aria-pressed", &facets_shown.to_string());
        buffer.push(Patch::SetText {
            node: switch,
            text: texts.facets_toggle.clone(),
        });
        set_lang(buffer, switch, texts);
    }

    let density = element(buffer, nodes, Some(bar), "div");
    attribute(buffer, density, "part", "density");
    attribute(buffer, density, "role", "group");
    attribute(buffer, density, "aria-label", &texts.density_group);
    set_lang(buffer, density, texts);
    for (name, ..) in DENSITIES {
        let button = element(buffer, nodes, Some(density), "button");
        attribute(buffer, button, "type", "button");
        attribute(buffer, button, "data-density", name);
        attribute(
            buffer,
            button,
            "aria-pressed",
            if *name == "normal" { "true" } else { "false" },
        );
        buffer.push(Patch::SetText {
            node: button,
            text: texts.density(name).to_owned(),
        });
    }

    let chips = element(buffer, nodes, Some(parent), "div");
    attribute(buffer, chips, "part", "chips");
    attribute(buffer, chips, "role", "group");
    label_by(
        buffer,
        nodes,
        parent,
        chips,
        "og-label-chips",
        &texts.chips_group,
        texts,
    );
    attribute(buffer, chips, "hidden", "");
    bar
}

/// Builds the column-visibility group (plan point 36).
///
/// One `<input type="checkbox">` with a `<label>` per **declared** column — the
/// hidden ones included, or there would be no way back. The group is empty
/// (and hidden) when the grid has no columns yet.
fn build_columns(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    declared: &[(String, bool)],
    texts: &GridTexts,
) -> ColumnsNodes {
    // A disclosure, not a permanent list: a grid with twenty columns would
    // otherwise carry twenty checkboxes above its data forever.
    let toggle = element(buffer, nodes, Some(parent), "button");
    for (name, value) in [
        ("type", "button"),
        ("part", "columns-toggle"),
        ("aria-expanded", "false"),
    ] {
        buffer.push(Patch::SetAttribute {
            node: toggle,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    buffer.push(Patch::SetText {
        node: toggle,
        text: texts.columns_group.clone(),
    });
    // The toggle carries the word, so the toggle carries its language — not the
    // container below, whose children are column names (see `set_lang`).
    set_lang(buffer, toggle, texts);

    let container = element(buffer, nodes, Some(parent), "div");
    for (name, value) in [("part", "columns"), ("role", "group"), ("hidden", "")] {
        buffer.push(Patch::SetAttribute {
            node: container,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    label_by(
        buffer,
        nodes,
        parent,
        container,
        "og-label-columns",
        &texts.columns_group,
        texts,
    );

    let mut boxes = Vec::with_capacity(declared.len());
    for (name, visible) in declared {
        let label = element(buffer, nodes, Some(container), "label");
        buffer.push(Patch::SetAttribute {
            node: label,
            name: "part".to_owned(),
            value: "column-toggle".to_owned(),
        });
        let input = element(buffer, nodes, Some(label), "input");
        for (attribute, value) in [("type", "checkbox"), ("data-column", name.as_str())] {
            buffer.push(Patch::SetAttribute {
                node: input,
                name: attribute.to_owned(),
                value: value.to_owned(),
            });
        }
        if *visible {
            buffer.push(Patch::SetAttribute {
                node: input,
                name: "checked".to_owned(),
                value: String::new(),
            });
        }
        // The column name is the label's text, so it is the accessible name —
        // and it is the page's word, in the page's language, not ours.
        let text = element(buffer, nodes, Some(label), "span");
        buffer.push(Patch::SetText {
            node: text,
            text: name.clone(),
        });
        boxes.push((input, name.clone()));
    }

    ColumnsNodes {
        toggle,
        container,
        boxes,
    }
}

/// Builds the status line (point 41): the one live region of the grid.
///
/// It is created with the skeleton and never removed, so an announcement is a
/// text change in a region assistive technology already observes — a region that
/// only appears when something goes wrong is announced unreliably. `role`
/// already implies `aria-live="polite"`; the attribute is written out because
/// the status is the contract of this point, not an implementation detail.
/// `data-state` carries the state for `::part(status)` styling.
fn build_status(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    texts: &GridTexts,
) -> NodeId {
    let status = element(buffer, nodes, Some(parent), "p");
    for (name, value) in [
        ("part", "status"),
        ("role", "status"),
        ("aria-live", "polite"),
        ("data-state", status_state(&GridStatus::Loading)),
    ] {
        buffer.push(Patch::SetAttribute {
            node: status,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    set_lang(buffer, status, texts);
    buffer.push(Patch::SetText {
        node: status,
        text: status_text(texts, &GridStatus::Loading, 0),
    });
    status
}

/// Builds the paging controls (plan point 38).
///
/// Built once and hidden while the grid virtualizes, so switching `page-size`
/// on and off does not rebuild the skeleton. Ordinary `<button>`s in a labelled
/// group **outside** the grid table: the keyboard reaches them with `Tab`, and
/// the grid's own matrix stays exactly as it was.
fn build_pager(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    texts: &GridTexts,
) -> PagerNodes {
    let container = element(buffer, nodes, Some(parent), "div");
    for (name, value) in [("part", "pager"), ("role", "group"), ("hidden", "")] {
        buffer.push(Patch::SetAttribute {
            node: container,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    set_lang(buffer, container, texts);

    let button =
        |buffer: &mut PatchBuffer, nodes: &mut NodeAllocator, part: &str, label: &str| -> NodeId {
            let node = element(buffer, nodes, Some(container), "button");
            for (name, value) in [("type", "button"), ("part", part)] {
                buffer.push(Patch::SetAttribute {
                    node,
                    name: name.to_owned(),
                    value: value.to_owned(),
                });
            }
            buffer.push(Patch::SetText {
                node,
                text: label.to_owned(),
            });
            node
        };

    let first = button(buffer, nodes, "page-first", &texts.page_first);
    let previous = button(buffer, nodes, "page-previous", &texts.page_previous);
    let label = element(buffer, nodes, Some(container), "span");
    buffer.push(Patch::SetAttribute {
        node: label,
        name: "part".to_owned(),
        value: "page-label".to_owned(),
    });
    let next = button(buffer, nodes, "page-next", &texts.page_next);
    let last = button(buffer, nodes, "page-last", &texts.page_last);

    PagerNodes {
        container,
        first,
        previous,
        next,
        last,
        label,
    }
}

/// Builds the type-agnostic filter row: one operator `select` and one value
/// `input` per column, plus a clear button (point 18).
///
/// It sits **outside** the `role="grid"` table (`part="filter"`), so the grid's
/// roving tabindex and keyboard matrix are untouched; the controls are ordinary
/// focusable form elements. Each control is labelled with its column name via
/// `aria-label` (all references stay inside the shadow root, E8/R6).
fn build_filter(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    fields: &[Field],
    texts: &GridTexts,
) -> FilterNodes {
    let container = element(buffer, nodes, Some(parent), "div");
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "part".to_owned(),
        value: "filter".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "data-filter".to_owned(),
        value: String::new(),
    });
    // It scrolls sideways when narrow, so Firefox would make it a tab stop of
    // its own — out of the sequence, like the viewport (`build_grid`).
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "tabindex".to_owned(),
        value: "-1".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: container,
        name: "role".to_owned(),
        value: "group".to_owned(),
    });
    label_by(
        buffer,
        nodes,
        parent,
        container,
        "og-label-filter",
        &texts.filter_group,
        texts,
    );

    let mut columns = Vec::with_capacity(fields.len());
    for (col, field) in fields.iter().enumerate() {
        let group = element(buffer, nodes, Some(container), "span");
        set_style(
            buffer,
            group,
            "display: inline-flex; align-items: center; gap: 0.25rem;",
        );

        let select = element(buffer, nodes, Some(group), "select");
        buffer.push(Patch::SetAttribute {
            node: select,
            name: "part".to_owned(),
            value: "filter-operator".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: select,
            name: "data-col".to_owned(),
            value: col.to_string(),
        });
        buffer.push(Patch::SetAttribute {
            node: select,
            name: "aria-label".to_owned(),
            value: texts.operator_label(field.name.as_str()),
        });
        // The options below are our words, so the language sits here and not on
        // the filter row, which also holds the column disclosure (see
        // `set_lang`). The `aria-label` above mixes our word with a column name
        // and is the one place no `lang` can be right for both.
        set_lang(buffer, select, texts);
        let mut options = Vec::with_capacity(FILTER_OPERATORS.len());
        for (option_index, op) in FILTER_OPERATORS.iter().enumerate() {
            let option = element(buffer, nodes, Some(select), "option");
            buffer.push(Patch::SetAttribute {
                node: option,
                name: "value".to_owned(),
                value: (*op).to_owned(),
            });
            if option_index == 0 {
                buffer.push(Patch::SetAttribute {
                    node: option,
                    name: "selected".to_owned(),
                    value: String::new(),
                });
            }
            // The `value` above is the wire token the query needs; what the
            // user reads is a word (point 48).
            buffer.push(Patch::SetText {
                node: option,
                text: texts.operator(option_index, op),
            });
            options.push(option);
        }

        let input = element(buffer, nodes, Some(group), "input");
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "type".to_owned(),
            value: "text".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "part".to_owned(),
            value: "filter-value".to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "data-col".to_owned(),
            value: col.to_string(),
        });
        buffer.push(Patch::SetAttribute {
            node: input,
            name: "aria-label".to_owned(),
            value: texts.value_label(field.name.as_str()),
        });
        set_style(buffer, input, "width: 6rem;");
        columns.push(FilterColumnNodes {
            select,
            options,
            input,
        });
    }

    let clear = element(buffer, nodes, Some(container), "button");
    buffer.push(Patch::SetAttribute {
        node: clear,
        name: "type".to_owned(),
        value: "button".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: clear,
        name: "part".to_owned(),
        value: "filter-clear".to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: clear,
        name: "data-filter-clear".to_owned(),
        value: String::new(),
    });
    buffer.push(Patch::SetText {
        node: clear,
        text: texts.clear.clone(),
    });
    set_lang(buffer, clear, texts);

    FilterNodes {
        container,
        clear,
        columns,
    }
}

/// Updates the skeleton in one frame: sizes the sizer, refreshes the header and
/// the result count and recycles the pool rows for the current window.
///
/// `slots` is the slot → logical row assignment (computed by [`assign_pool`]).
/// `pinned_slot` is the slot holding the focused cell: it is left completely
/// untouched, so the focused DOM node keeps its `data-row` and the browser keeps
/// the focus. Every other slot gets its new `aria-rowindex`, `data-row`, text
/// and `translateY` — no node is created or removed.
///
/// `sorts` is the whole sort list in order (point 18): a column's `aria-sort`
/// reflects its key and, when more than one key is active, its 1-based position
/// is shown in the header's `aria-hidden` index span.
#[allow(clippy::too_many_arguments)]
pub fn patch_grid(
    buffer: &mut PatchBuffer,
    nodes: &GridNodes,
    state: &GridState,
    slots: &[Option<u64>],
    active: ActiveCell,
    sorts: &[(String, &str)],
    pinned_slot: Option<usize>,
    row_height: u64,
    texts: &GridTexts,
    format: &dyn CellFormat,
    paging: Paging,
    grouping: Option<&crate::grouping::Grouping>,
) {
    let fields = state.schema().fields();
    let total_count = state.total_count();

    // **The one place the role is decided** (F2): a grouped grid is a
    // `treegrid`, an ungrouped one a `grid`. Kept here, and nowhere else, so
    // that the screen-reader run of point 71 can turn it back with one line if
    // the switch at run time turns out to be louder than the lie it avoids.
    buffer.push(Patch::SetAttribute {
        node: nodes.table,
        name: "role".to_owned(),
        value: if grouping.is_some() {
            "treegrid"
        } else {
            "grid"
        }
        .to_owned(),
    });

    buffer.push(Patch::SetAttribute {
        node: nodes.tbody,
        name: "style".to_owned(),
        value: format!(
            "position: relative; height: {}px;",
            paging.rows * row_height
        ),
    });
    buffer.push(Patch::SetAttribute {
        node: nodes.table,
        name: "aria-rowcount".to_owned(),
        value: (paging.rows + 1).to_string(),
    });
    patch_pager(buffer, &nodes.pager, paging, total_count, texts);

    // The select-all header, in its three states (point 61).
    if let Some(mark) = nodes.select_all_mark {
        let all = select_all_state(state.selection().len(), total_count);
        buffer.push(Patch::SetAttribute {
            node: mark,
            name: "tabindex".to_owned(),
            value: tabindex_for(active == ActiveCell::SelectAll).to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: mark,
            name: "aria-checked".to_owned(),
            value: all.as_str().to_owned(),
        });
        buffer.push(Patch::SetText {
            node: mark,
            text: check_glyph(all).to_owned(),
        });
    }

    buffer.push(Patch::SetAttribute {
        node: nodes.status,
        name: "data-state".to_owned(),
        value: status_state(state.status()).to_owned(),
    });
    buffer.push(Patch::SetText {
        node: nodes.status,
        text: match grouping {
            // "57 matches" for 52 rows and five group headers would be false:
            // under grouping the display list is longer than the result.
            Some(grouping) => status_line_counting(texts, state, grouping.row_count()),
            None => status_line(texts, state),
        },
    });

    // The filter row follows the schema: which operators a column offers and what
    // kind of input it takes are decided by its type, which only exists once a
    // result has arrived (point 51).
    for (col, column) in nodes.filter.columns.iter().enumerate() {
        let Some(field) = fields.get(col) else {
            continue;
        };
        let allowed = operators_for(field.data_type, field.nullable);
        for (index, option) in column.options.iter().enumerate() {
            let fits = FILTER_OPERATORS
                .get(index)
                .is_some_and(|op| allowed.contains(op));
            // Hidden *and* disabled: hidden keeps it out of the list, disabled
            // keeps it out of reach for anything that ignores `hidden`.
            for name in ["hidden", "disabled"] {
                if fits {
                    buffer.push(Patch::RemoveAttribute {
                        node: *option,
                        name: name.to_owned(),
                    });
                } else {
                    buffer.push(Patch::SetAttribute {
                        node: *option,
                        name: name.to_owned(),
                        value: String::new(),
                    });
                }
            }
        }
        buffer.push(Patch::SetAttribute {
            node: column.input,
            name: "type".to_owned(),
            value: input_type(field.data_type).to_owned(),
        });
        match input_step(field.data_type) {
            Some(step) => buffer.push(Patch::SetAttribute {
                node: column.input,
                name: "step".to_owned(),
                value: step,
            }),
            None => buffer.push(Patch::RemoveAttribute {
                node: column.input,
                name: "step".to_owned(),
            }),
        }
    }

    for (col, header) in nodes.header_cells.iter().enumerate() {
        let key = fields
            .get(col)
            .and_then(|field| sort_position(sorts, field.name.as_str()));
        buffer.push(Patch::SetAttribute {
            node: header.cell,
            name: "aria-sort".to_owned(),
            value: key
                .map_or("none", |position| aria_sort(sorts[position].1))
                .to_owned(),
        });
        buffer.push(Patch::SetAttribute {
            node: header.cell,
            name: "tabindex".to_owned(),
            value: tabindex_for(active == ActiveCell::Header { col }).to_owned(),
        });
        // The aggregate a column shows in groups (point 63), drawn as a glyph
        // next to the name — the cells themselves say the word.
        let aggregate = grouping.and_then(|grouping| {
            fields.get(col).and_then(|field| {
                grouping
                    .aggregates()
                    .iter()
                    .find(|(column, _)| column == field.name.as_str())
                    .map(|(_, function)| function.as_str())
            })
        });
        match aggregate {
            Some(function) => buffer.push(Patch::SetAttribute {
                node: header.cell,
                name: "data-aggregate".to_owned(),
                value: function.to_owned(),
            }),
            None => buffer.push(Patch::RemoveAttribute {
                node: header.cell,
                name: "data-aggregate".to_owned(),
            }),
        }
        // Both marks come from the same key as `aria-sort`, so what is seen and
        // what is announced cannot drift apart.
        buffer.push(Patch::SetText {
            node: header.direction,
            text: key
                .map(|position| direction_glyph(sorts[position].1))
                .unwrap_or_default()
                .to_owned(),
        });
        let index = if sorts.len() > 1 {
            key.map(|position| (position + 1).to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };
        buffer.push(Patch::SetText {
            node: header.index,
            text: index,
        });
    }

    for (slot, row_nodes) in nodes.rows.iter().enumerate() {
        if Some(slot) == pinned_slot {
            // The pinned slot keeps its `data-row`, its text and its position —
            // rewriting them would move the row out from under the focus. Its
            // **selection** still has to follow, or selecting the row you are
            // standing on would show nothing: `aria-selected` touches neither
            // identity nor focus.
            if let Some(row) = slots.get(slot).copied().flatten() {
                // A group header is exactly what the focus stands on when it is
                // toggled — the fourth time the pin of point 17 meets a feature
                // (phase E (i)). Its own label and `aria-expanded` are rewritten;
                // its position and identity are not, so the focus stays.
                if let Some(grouping) = grouping
                    && let Some(
                        item @ (crate::grouping::Item::Group { .. }
                        | crate::grouping::Item::Total { .. }),
                    ) = grouping.item_at(row)
                {
                    patch_group_row(buffer, row_nodes, &item, grouping, fields, texts, format);
                    continue;
                }
                let selected = state.is_selected(row);
                selection_attributes(buffer, row_nodes.row, selected);
                if let (Some(cell), Some(mark)) = (row_nodes.select, row_nodes.select_mark) {
                    buffer.push(Patch::SetAttribute {
                        node: cell,
                        name: "tabindex".to_owned(),
                        value: tabindex_for(active == ActiveCell::Select { row }).to_owned(),
                    });
                    // The mark is decoration: the row already carries
                    // `aria-selected`, and a second voice per row would double
                    // every announcement (point 61).
                    buffer.push(Patch::SetText {
                        node: mark,
                        text: check_glyph(if selected {
                            CheckState::On
                        } else {
                            CheckState::Off
                        })
                        .to_owned(),
                    });
                }
            }
            continue;
        }
        match slots.get(slot).copied().flatten() {
            Some(row) => {
                let local = row.saturating_sub(paging.base);
                set_style(buffer, row_nodes.row, &row_style(local, row_height));
                buffer.push(Patch::SetAttribute {
                    node: row_nodes.row,
                    name: "aria-rowindex".to_owned(),
                    value: (local + 2).to_string(),
                });
                // Under grouping a position is a group header or a data row, and
                // the same pooled `<tr>` draws either — so scrolling still adds
                // no node, grouped or not (point 17's promise, kept).
                let item = grouping.and_then(|grouping| grouping.item_at(row));
                if let (
                    Some(grouping),
                    Some(
                        item @ (crate::grouping::Item::Group { .. }
                        | crate::grouping::Item::Total { .. }),
                    ),
                ) = (grouping, &item)
                {
                    for (col, cell) in row_nodes.cells.iter().enumerate() {
                        buffer.push(Patch::SetAttribute {
                            node: *cell,
                            name: "data-row".to_owned(),
                            value: row.to_string(),
                        });
                        let is_active = active == ActiveCell::Data(CellRef::new(row, col));
                        buffer.push(Patch::SetAttribute {
                            node: *cell,
                            name: "tabindex".to_owned(),
                            value: tabindex_for(is_active).to_owned(),
                        });
                        buffer.push(Patch::RemoveAttribute {
                            node: *cell,
                            name: "data-changed".to_owned(),
                        });
                    }
                    if let (Some(cell), Some(mark)) = (row_nodes.select, row_nodes.select_mark) {
                        buffer.push(Patch::SetAttribute {
                            node: cell,
                            name: "tabindex".to_owned(),
                            value: tabindex_for(active == ActiveCell::Select { row }).to_owned(),
                        });
                        buffer.push(Patch::SetText {
                            node: mark,
                            text: String::new(),
                        });
                    }
                    patch_group_row(buffer, row_nodes, item, grouping, fields, texts, format);
                    continue;
                }
                // A data row: one level deeper than the innermost group, or no
                // level at all when nothing is grouped.
                match grouping {
                    Some(grouping) => buffer.push(Patch::SetAttribute {
                        node: row_nodes.row,
                        name: "aria-level".to_owned(),
                        value: (grouping.levels() + 1).to_string(),
                    }),
                    None => buffer.push(Patch::RemoveAttribute {
                        node: row_nodes.row,
                        name: "aria-level".to_owned(),
                    }),
                }
                for name in ["aria-expanded", "data-kind", "data-level"] {
                    buffer.push(Patch::RemoveAttribute {
                        node: row_nodes.row,
                        name: name.to_owned(),
                    });
                }
                // A slot that showed the total carried our `lang` on its label;
                // a value in it is the page's word again.
                if let Some(first) = row_nodes.cells.first() {
                    buffer.push(Patch::RemoveAttribute {
                        node: *first,
                        name: "lang".to_owned(),
                    });
                }
                // Selection is per **logical** row, so a recycled slot picks up
                // the state of whatever row it now shows — that is what makes a
                // selection survive scrolling (point 35). `data-selected` is the
                // styling hook; `aria-selected` is the announcement.
                let selected = state.is_selected(row);
                selection_attributes(buffer, row_nodes.row, selected);
                if let (Some(cell), Some(mark)) = (row_nodes.select, row_nodes.select_mark) {
                    buffer.push(Patch::SetAttribute {
                        node: cell,
                        name: "tabindex".to_owned(),
                        value: tabindex_for(active == ActiveCell::Select { row }).to_owned(),
                    });
                    // The mark is decoration: the row already carries
                    // `aria-selected`, and a second voice per row would double
                    // every announcement (point 61).
                    buffer.push(Patch::SetText {
                        node: mark,
                        text: check_glyph(if selected {
                            CheckState::On
                        } else {
                            CheckState::Off
                        })
                        .to_owned(),
                    });
                }
                for (col, cell) in row_nodes.cells.iter().enumerate() {
                    buffer.push(Patch::SetAttribute {
                        node: *cell,
                        name: "data-row".to_owned(),
                        value: row.to_string(),
                    });
                    // A pooled cell that drew a group row a frame ago still
                    // carries its aggregate name; a data cell must not.
                    if grouping.is_some() {
                        for name in ["data-aggregate", "aria-label"] {
                            buffer.push(Patch::RemoveAttribute {
                                node: *cell,
                                name: name.to_owned(),
                            });
                        }
                    }
                    let reference = CellRef::new(row, col);
                    let is_active = active == ActiveCell::Data(reference);
                    buffer.push(Patch::SetAttribute {
                        node: *cell,
                        name: "tabindex".to_owned(),
                        value: tabindex_for(is_active).to_owned(),
                    });
                    // A cell the reader changed but nobody saved is marked —
                    // the grid shows what was typed and does not pretend it is
                    // stored (point 37).
                    if state.is_changed(reference) {
                        buffer.push(Patch::SetAttribute {
                            node: *cell,
                            name: "data-changed".to_owned(),
                            value: "true".to_owned(),
                        });
                    } else {
                        buffer.push(Patch::RemoveAttribute {
                            node: *cell,
                            name: "data-changed".to_owned(),
                        });
                    }

                    // The cell being edited holds an `<input>`, not text: the
                    // element owns those nodes, and writing text here would
                    // throw the editor away mid-keystroke.
                    if state.editing() == Some(reference) {
                        continue;
                    }
                    // Display only (point 42): the value behind it is what the
                    // filter and the sort keep working on.
                    let text = state
                        .cell(reference)
                        .map(|value| format.text(col, value))
                        .unwrap_or_default();
                    buffer.push(Patch::SetText { node: *cell, text });
                }
            }
            None => {
                set_style(buffer, row_nodes.row, ROW_HIDDEN_STYLE);
                buffer.push(Patch::RemoveAttribute {
                    node: row_nodes.row,
                    name: "aria-rowindex".to_owned(),
                });
                buffer.push(Patch::RemoveAttribute {
                    node: row_nodes.row,
                    name: "aria-selected".to_owned(),
                });
                buffer.push(Patch::RemoveAttribute {
                    node: row_nodes.row,
                    name: "data-selected".to_owned(),
                });
            }
        }
    }
}

/// Writes a row's selection: the announcement and the styling hook.
///
/// `aria-selected` is what assistive technology reads; `data-selected` is what a
/// theme can shade. Both, because shading alone is not information
/// (WCAG 1.4.1) and an ARIA state alone is invisible.
/// Writes a group header — or the grand total — into a pooled row (points 62
/// and 63).
///
/// The label goes into the first data cell, as one sentence ("country: DE
/// (52 rows)", "Total (200 rows)"). Every column with a chosen aggregate gets
/// its value; the rest are emptied. The chevron is not text: it is drawn from
/// `aria-expanded` in the stylesheet, with an empty alternative, so it is seen
/// and not read twice — the state is already said by `aria-expanded` itself.
fn patch_group_row(
    buffer: &mut PatchBuffer,
    row_nodes: &GridRowNodes,
    item: &crate::grouping::Item,
    grouping: &crate::grouping::Grouping,
    fields: &[opengrid_types::Field],
    texts: &GridTexts,
    format: &dyn CellFormat,
) {
    use crate::grouping::Item;
    let (label, level, expanded, aggregates, kind) = match item {
        Item::Group {
            level,
            value,
            count,
            expanded,
            aggregates,
            ..
        } => {
            let column = grouping
                .by()
                .get(level - 1)
                .map(String::as_str)
                .unwrap_or_default();
            let value = group_value_text(value, column, fields, texts, format);
            (
                texts.group_row(column, &value, *count),
                *level,
                Some(*expanded),
                aggregates,
                "group",
            )
        }
        Item::Total { count, aggregates } => {
            (texts.total_row(*count), 1, None, aggregates, "total")
        }
        Item::Row { .. } => return,
    };

    for (name, value) in [
        ("data-kind", kind.to_owned()),
        ("data-level", level.to_string()),
        ("aria-level", level.to_string()),
    ] {
        buffer.push(Patch::SetAttribute {
            node: row_nodes.row,
            name: name.to_owned(),
            value,
        });
    }
    match expanded {
        Some(expanded) => buffer.push(Patch::SetAttribute {
            node: row_nodes.row,
            name: "aria-expanded".to_owned(),
            value: expanded.to_string(),
        }),
        // The total opens nothing, and `aria-expanded` on it would promise that
        // it does.
        None => buffer.push(Patch::RemoveAttribute {
            node: row_nodes.row,
            name: "aria-expanded".to_owned(),
        }),
    }
    // A group is not a record: it cannot be selected, and saying
    // `aria-selected="false"` about it would invite trying.
    for name in ["aria-selected", "data-selected"] {
        buffer.push(Patch::RemoveAttribute {
            node: row_nodes.row,
            name: name.to_owned(),
        });
    }

    // One slice of values per chosen summary: a range answers two.
    let values = crate::grouping::split(grouping.aggregates(), aggregates);
    for (col, cell) in row_nodes.cells.iter().enumerate() {
        let chosen = fields.get(col).and_then(|field| {
            grouping
                .aggregates()
                .iter()
                .position(|(column, _)| column == field.name.as_str())
                .map(|index| (index, grouping.aggregates()[index].1))
        });
        // The first cell is the label, even when its column has an aggregate:
        // a row nobody can name is worse than one number fewer.
        let (text, aggregate) = if col == 0 {
            // The total's label is all ours ("Total (6 rows)"); a group's names
            // a column and a value, the page's words, so it claims no language.
            if kind == "total" && !texts.lang.trim().is_empty() {
                buffer.push(Patch::SetAttribute {
                    node: *cell,
                    name: "lang".to_owned(),
                    value: texts.lang.clone(),
                });
            } else {
                buffer.push(Patch::RemoveAttribute {
                    node: *cell,
                    name: "lang".to_owned(),
                });
            }
            (label.clone(), None)
        } else if let Some((index, function)) = chosen {
            let value = values.get(index).copied().unwrap_or(&[]);
            (aggregate_text(function, col, value, format), Some(function))
        } else {
            (String::new(), None)
        };
        match aggregate {
            // An aggregate over nothing is NULL (S11): the cell stays empty and
            // draws no glyph — a "Σ" with no number beside it reads as a zero —
            // but it still *says* what it is, with the word every other missing
            // value gets. "Average: " would be half a sentence.
            Some(function) if text.is_empty() => {
                buffer.push(Patch::RemoveAttribute {
                    node: *cell,
                    name: "data-aggregate".to_owned(),
                });
                buffer.push(Patch::SetAttribute {
                    node: *cell,
                    name: "aria-label".to_owned(),
                    value: texts.aggregate_cell(function, &texts.no_value),
                });
            }
            Some(function) => {
                buffer.push(Patch::SetAttribute {
                    node: *cell,
                    name: "data-aggregate".to_owned(),
                    value: function.as_str().to_owned(),
                });
                buffer.push(Patch::SetAttribute {
                    node: *cell,
                    name: "aria-label".to_owned(),
                    value: texts.aggregate_cell(function, &text),
                });
            }
            None => {
                for name in ["data-aggregate", "aria-label"] {
                    buffer.push(Patch::RemoveAttribute {
                        node: *cell,
                        name: name.to_owned(),
                    });
                }
            }
        }
        buffer.push(Patch::SetText { node: *cell, text });
    }
}

/// An aggregate as text.
///
/// `count` is a number of rows, whatever the column holds — a count of a
/// currency column printed as currency would be "€52.00" rows. Everything else
/// goes through the column's format, so a sum reads like the values it sums.
/// NULL (S11: a sum over nothing) is empty, not zero: zero would be a claim.
///
/// A range (F7) is its two ends, "from – to", each in the column's format; a
/// group whose rows all hold the same value shows it once, since "2.3.2026 –
/// 2.3.2026" says less than "2.3.2026".
fn aggregate_text(
    summary: crate::presentation::Summary,
    col: usize,
    values: &[opengrid_types::Value],
    format: &dyn CellFormat,
) -> String {
    use crate::presentation::Summary;
    use opengrid_types::Value;
    match (summary, values) {
        (Summary::Range, [Value::Null, Value::Null]) => String::new(),
        (Summary::Range, [low, high]) if low == high => format.text(col, low),
        (Summary::Range, [low, high]) => {
            format!(
                "{} \u{2013} {}",
                format.text(col, low),
                format.text(col, high)
            )
        }
        (_, [Value::Null] | []) => String::new(),
        (Summary::Fn(opengrid_query::AggregateFn::Count), [value]) => {
            crate::formats::plain_text(value)
        }
        (_, [value, ..]) => format.text(col, value),
    }
}

/// The key of a group as a reader sees it.
///
/// NULL and the empty string are different groups (S10, S14) and get different
/// words — an empty label is silence to a screen reader, the reason
/// `noValue`/`emptyValue` exist since the pivot of point 32. Anything else goes
/// through the column's format, so a date key reads like the dates below it.
pub fn group_value_text(
    value: &opengrid_types::Value,
    column: &str,
    fields: &[opengrid_types::Field],
    texts: &GridTexts,
    format: &dyn CellFormat,
) -> String {
    match value {
        opengrid_types::Value::Null => texts.no_value.clone(),
        opengrid_types::Value::Utf8(text) if text.is_empty() => texts.empty_value.clone(),
        value => match fields
            .iter()
            .position(|field| field.name.as_str() == column)
        {
            Some(col) => format.text(col, value),
            None => crate::formats::plain_text(value),
        },
    }
}

/// The glyph of a checkbox in one of its three states.
///
/// Text, not an image or a pseudo-element: it survives forced colours, it
/// scales with the font, and it costs nothing. It is `aria-hidden` wherever it
/// appears — the state is on the cell, in words.
pub const fn check_glyph(state: CheckState) -> &'static str {
    match state {
        CheckState::On => "\u{2713}",
        CheckState::Mixed => "\u{2013}",
        CheckState::Off => "",
    }
}

/// What a checkbox says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Off,
    Mixed,
    On,
}

impl CheckState {
    /// The `aria-checked` value.
    pub const fn as_str(&self) -> &'static str {
        match self {
            CheckState::Off => "false",
            CheckState::Mixed => "mixed",
            CheckState::On => "true",
        }
    }
}

/// The state of the select-all header for a selection over `total_count` rows.
///
/// **"All" means all matching rows, not the loaded page.** The grid holds one
/// window at a time; a header that said "all" about what happens to be on
/// screen would be a different promise from `Ctrl`+`A`, which has selected
/// every matching row since point 35.
pub fn select_all_state(selected: usize, total_count: u64) -> CheckState {
    if total_count == 0 || selected == 0 {
        CheckState::Off
    } else if selected as u64 >= total_count {
        CheckState::On
    } else {
        CheckState::Mixed
    }
}

fn selection_attributes(buffer: &mut PatchBuffer, row: NodeId, selected: bool) {
    buffer.push(Patch::SetAttribute {
        node: row,
        name: "aria-selected".to_owned(),
        value: selected.to_string(),
    });
    if selected {
        buffer.push(Patch::SetAttribute {
            node: row,
            name: "data-selected".to_owned(),
            value: "true".to_owned(),
        });
    } else {
        buffer.push(Patch::RemoveAttribute {
            node: row,
            name: "data-selected".to_owned(),
        });
    }
}

/// Updates the paging controls, or hides them while the grid virtualizes.
///
/// A button that cannot do anything is **disabled**, not missing: the row of
/// controls keeps its shape, and a screen reader is told why the first page has
/// no "previous" instead of finding one less button than last time.
fn patch_pager(
    buffer: &mut PatchBuffer,
    pager: &PagerNodes,
    paging: Paging,
    total_count: u64,
    texts: &GridTexts,
) {
    let Some(size) = paging.size else {
        buffer.push(Patch::SetAttribute {
            node: pager.container,
            name: "hidden".to_owned(),
            value: String::new(),
        });
        return;
    };
    buffer.push(Patch::RemoveAttribute {
        node: pager.container,
        name: "hidden".to_owned(),
    });

    let pages = Paging::count(total_count, size);
    let page = paging.index();
    buffer.push(Patch::SetAttribute {
        node: pager.container,
        name: "aria-label".to_owned(),
        value: texts.page_of(page + 1, pages),
    });
    buffer.push(Patch::SetText {
        node: pager.label,
        text: texts.page_of(page + 1, pages),
    });

    for (node, disabled) in [
        (pager.first, page == 0),
        (pager.previous, page == 0),
        (pager.next, page + 1 >= pages),
        (pager.last, page + 1 >= pages),
    ] {
        if disabled {
            buffer.push(Patch::SetAttribute {
                node,
                name: "disabled".to_owned(),
                value: String::new(),
            });
        } else {
            buffer.push(Patch::RemoveAttribute {
                node,
                name: "disabled".to_owned(),
            });
        }
    }
}

/// The inline style placing a visible pool row at its logical position.
///
/// The static `position: absolute` box comes from the shadow stylesheet; only
/// the logical offset is per-row.
fn row_style(row: u64, row_height: u64) -> String {
    format!("transform: translateY({}px);", row * row_height)
}

/// Hides a pool slot that holds no logical row in the current window.
const ROW_HIDDEN_STYLE: &str = "display: none;";

/// Sets an element's `style` attribute.
fn set_style(buffer: &mut PatchBuffer, node: NodeId, style: &str) {
    buffer.push(Patch::SetAttribute {
        node,
        name: "style".to_owned(),
        value: style.to_owned(),
    });
}

/// The position of a column in the sort list, if it is a key.
fn sort_position(sorts: &[(String, &str)], name: &str) -> Option<usize> {
    sorts.iter().position(|(field, _)| field == name)
}

/// The visible glyph for a sort direction (point 49), empty for an unknown one.
fn direction_glyph(direction: &str) -> &'static str {
    match direction {
        "asc" => ASCENDING_GLYPH,
        "desc" => DESCENDING_GLYPH,
        _ => "",
    }
}

/// The `aria-sort` token for a wire direction.
fn aria_sort(direction: &str) -> &'static str {
    match direction {
        "asc" => "ascending",
        "desc" => "descending",
        _ => "none",
    }
}

/// The `tabindex` value of a cell: `0` for the active one, `-1` otherwise.
fn tabindex_for(is_active: bool) -> &'static str {
    if is_active { "0" } else { "-1" }
}

/// Marks `node` as being written in the texts' language (point 48).
///
/// Only the elements that carry the component's **own** texts get it — the
/// filter row, the status line, the pager and the column disclosure's toggle.
/// The cells and the column headers are the page's data, in the page's
/// language; declaring them English because the built-in texts are English
/// would be the very WCAG 3.1.2 failure this is meant to remove. An empty
/// `lang` leaves the document's language everywhere.
///
/// The rule is about the node's **subtree**, not just its own text, because
/// `lang` is inherited. Two containers used to break it:
///
/// * `part="columns"`, the disclosure, holds one checkbox per column and every
///   label there is a **column name**.
/// * `part="filter"`, the filter row, *contains* that disclosure (point 37 moved
///   it there so the list would stop taking height), so its `lang` reached the
///   column names too.
///
/// So neither container is tagged. The language sits on the nodes whose whole
/// subtree is ours: the disclosure's toggle button, each operator `select` (its
/// `option`s are our words) and the clear button. Both containers keep their
/// `aria-label` untagged — two group names in the page's language are a far
/// smaller claim than every column name in the wrong one.
///
/// The one case no `lang` can settle is the `aria-label` of the operator and
/// value controls: `operatorLabel`/`valueLabel` splice a column name into one of
/// our words (`"{column} Operator"`), so the attribute is mixed by construction.
/// Whether that is audible is a question for a real screen reader — it is in the
/// point 55 protocol (S11), not decided here.
fn set_lang(buffer: &mut PatchBuffer, node: NodeId, texts: &GridTexts) {
    if texts.lang.trim().is_empty() {
        return;
    }
    buffer.push(Patch::SetAttribute {
        node,
        name: "lang".to_owned(),
        value: texts.lang.clone(),
    });
}

/// Names `target` with our words when its subtree holds the page's (F9,
/// decided 2026-09-24).
///
/// A toolbar, the facets, the search field and the column list are named in
/// our language but contain column names or typed values — a `lang` on the
/// container would claim those, and an `aria-label` cannot carry one of its
/// own. So the name lives in a hidden `<span>` beside the target, with our
/// `lang`, and the target points at it with `aria-labelledby`. A hidden element
/// still names what refers to it, and the reference stays inside the shadow
/// root (E8).
fn label_by(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    target: NodeId,
    id: &str,
    text: &str,
    texts: &GridTexts,
) {
    let label = element(buffer, nodes, Some(parent), "span");
    for (name, value) in [("id", id), ("hidden", "")] {
        buffer.push(Patch::SetAttribute {
            node: label,
            name: name.to_owned(),
            value: value.to_owned(),
        });
    }
    set_lang(buffer, label, texts);
    buffer.push(Patch::SetText {
        node: label,
        text: text.to_owned(),
    });
    buffer.push(Patch::SetAttribute {
        node: target,
        name: "aria-labelledby".to_owned(),
        value: id.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use opengrid_grid::Window;
    use opengrid_types::Value;

    fn state_with(rows: &[&str], total: u64, offset: u64, pool: u64) -> GridState {
        let schema = Schema::new(vec![
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("qty").unwrap(), DataType::Utf8),
        ]);
        let mut state = GridState::new(schema);
        state.set_window(Window::new(offset, pool));
        let customers = rows
            .iter()
            .map(|row| Value::Utf8((*row).to_owned()))
            .collect();
        let qtys = (0..rows.len())
            .map(|index| Value::Utf8(index.to_string()))
            .collect();
        state.apply_result(QueryResult::new(
            Schema::new(vec![
                Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
                Field::new(FieldName::new("qty").unwrap(), DataType::Utf8),
            ]),
            vec![customers, qtys],
            total,
        ));
        state
    }

    /// The columns attribute is the table's trimmed list.
    #[test]
    fn columns_attribute_is_a_trimmed_list() {
        assert_eq!(
            parse_columns(Some(" a , b ,,")),
            ["a".to_owned(), "b".to_owned()]
        );
    }

    /// The pool size falls back to the default on a missing, broken or zero value.
    #[test]
    fn pool_size_falls_back_to_the_default() {
        assert_eq!(parse_window_size(None), 40);
        assert_eq!(parse_window_size(Some(" 25 ")), 25);
        assert_eq!(parse_window_size(Some("0")), 40);
        assert_eq!(parse_window_size(Some("nope")), 40);
    }

    /// The row height accepts `<number>px` and falls back on anything else.
    #[test]
    fn row_height_parses_pixels_and_falls_back() {
        assert_eq!(parse_row_height("48px"), 48);
        assert_eq!(parse_row_height(" 20px "), 20);
        assert_eq!(parse_row_height("48"), DEFAULT_ROW_HEIGHT);
        assert_eq!(parse_row_height("48em"), DEFAULT_ROW_HEIGHT);
        assert_eq!(parse_row_height("0px"), DEFAULT_ROW_HEIGHT);
        assert_eq!(parse_row_height(""), DEFAULT_ROW_HEIGHT);
    }

    /// The query carries select, limit and offset, and omits sort when unsorted.
    #[test]
    fn an_unsorted_query_has_limit_and_offset() {
        use serde_json::{Value as Json, json};
        let query = query_json("orders", &["a".to_owned()], &[], None, 100, 40);
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(value["source"], "orders");
        assert_eq!(value["select"], json!(["a"]));
        assert_eq!(value["limit"], json!(40));
        assert_eq!(value["offset"], json!(100));
        assert!(value.get("sort").is_none());
        assert!(value.get("filter").is_none());
    }

    /// A sorted query carries one sort object per key, in order.
    #[test]
    fn a_sorted_query_names_one_direction() {
        use serde_json::{Value as Json, json};
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[("customer".to_owned(), "desc")],
            None,
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([{ "field": "customer", "direction": "desc" }])
        );
    }

    /// A view's query: the group keys first, NULL last said explicitly as the
    /// group query says it, then the sort without the keys already there —
    /// and no window.
    #[test]
    fn a_view_query_leads_with_the_group_keys_null_last() {
        use serde_json::{Value as Json, json};
        let query = view_query_json(
            "orders",
            &["country".to_owned(), "amount".to_owned()],
            &["country".to_owned(), "customer".to_owned()],
            &[
                ("amount".to_owned(), "desc"),
                ("country".to_owned(), "desc"),
            ],
            None,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([
                { "field": "country", "direction": "asc", "nulls": "last" },
                { "field": "customer", "direction": "asc", "nulls": "last" },
                { "field": "amount", "direction": "desc" }
            ])
        );
        assert!(value.get("limit").is_none());
        assert!(value.get("offset").is_none());
        // Ungrouped, the sort is the sort as it is.
        let plain = view_query_json(
            "orders",
            &["amount".to_owned()],
            &[],
            &[("amount".to_owned(), "desc")],
            None,
        );
        let value: Json = serde_json::from_str(&plain).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([{ "field": "amount", "direction": "desc" }])
        );
    }

    /// A multi-sort query keeps the keys in the user's order.
    #[test]
    fn a_multi_sort_query_keeps_the_key_order() {
        use serde_json::{Value as Json, json};
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[
                ("customer".to_owned(), "asc"),
                ("amount".to_owned(), "desc"),
            ],
            None,
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["sort"],
            json!([
                { "field": "customer", "direction": "asc" },
                { "field": "amount", "direction": "desc" }
            ])
        );
    }

    /// A schema with one column of each interesting type.
    fn typed_schema() -> Schema {
        Schema::new(vec![
            Field::new(FieldName::new("customer").unwrap(), DataType::Utf8),
            Field::new(FieldName::new("qty").unwrap(), DataType::Int64),
            Field::new(
                FieldName::new("amount").unwrap(),
                DataType::decimal(12, 2).unwrap(),
            ),
            Field::new(FieldName::new("ordered_on").unwrap(), DataType::Date),
            Field::new(FieldName::new("flag").unwrap(), DataType::Bool),
            Field::required(FieldName::new("id").unwrap(), DataType::Int64),
        ])
    }

    fn entry(column: &str, op: &str, value: &str) -> FilterEntry {
        FilterEntry {
            column: column.to_owned(),
            op: FilterOp::parse(op).expect("a known operator"),
            value: value.to_owned(),
        }
    }

    /// The filter expression is the `and` of the non-empty entries, each literal
    /// written the way its column expects (point 51).
    #[test]
    fn the_filter_query_types_every_literal() {
        use serde_json::{Value as Json, json};
        let entries = [
            entry("customer", "contains", "Al"),
            entry("qty", "gt", "2"),
            entry("amount", "lte", "10.50"),
            entry("ordered_on", "eq", "2026-01-01"),
            entry("flag", "eq", "true"),
        ];
        let filter = filter_expr(&entries, &typed_schema())
            .expect("no problems")
            .expect("a filter");
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[],
            Some(&filter),
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");

        assert_eq!(
            value["filter"],
            json!({ "and": [
                { "field": "customer", "op": "contains", "value": "Al" },
                // A number, not the string "2" — which is what made numeric
                // filters a type error before this point.
                { "field": "qty", "op": "gt", "value": 2 },
                // A decimal stays a string so no precision is lost.
                { "field": "amount", "op": "lte", "value": "10.50" },
                { "field": "ordered_on", "op": "eq", "value": "2026-01-01" },
                { "field": "flag", "op": "eq", "value": true }
            ]})
        );
    }

    /// The two operators that take no value become their own filter nodes.
    #[test]
    fn the_null_operators_need_no_value() {
        use serde_json::{Value as Json, json};
        let entries = [
            entry("customer", "is_null", ""),
            entry("qty", "is_not_null", ""),
        ];
        let filter = filter_expr(&entries, &typed_schema())
            .expect("no problems")
            .expect("a filter");
        let query = query_json(
            "orders",
            &["customer".to_owned()],
            &[],
            Some(&filter),
            0,
            40,
        );
        let value: Json = serde_json::from_str(&query).expect("valid JSON");
        assert_eq!(
            value["filter"],
            json!({ "and": [
                { "field": "customer", "op": "is_null" },
                { "field": "qty", "op": "is_not_null" }
            ]})
        );
    }

    /// An input the column cannot hold never becomes a query — the user hears
    /// about it instead of the engine.
    #[test]
    fn an_impossible_value_is_a_problem_not_a_query() {
        let schema = typed_schema();
        for (column, text) in [
            ("qty", "zwei"),
            ("qty", "2.5"),
            ("amount", "10,50"),
            ("ordered_on", "01.01.2026"),
            ("flag", "vielleicht"),
        ] {
            let problems = filter_expr(&[entry(column, "eq", text)], &schema)
                .expect_err(&format!("{column} = {text:?} must be refused"));
            assert_eq!(problems.len(), 1);
            assert_eq!(problems[0].column, column);
            assert_eq!(problems[0].value, text);
        }
    }

    /// Blank entries are skipped; an all-blank input clears the filter.
    #[test]
    fn blank_filter_entries_do_not_build_a_filter() {
        let schema = typed_schema();
        assert_eq!(filter_expr(&[], &schema), Ok(None));
        assert_eq!(
            filter_expr(&[entry("customer", "eq", "  ")], &schema),
            Ok(None)
        );
        assert!(
            filter_expr(&[entry("customer", "eq", "DE")], &schema)
                .unwrap()
                .is_some()
        );
    }

    /// Which operators a column offers, and which value input it gets.
    #[test]
    fn a_column_offers_the_operators_that_fit_it() {
        // Substring tests belong to text alone.
        assert!(operators_for(DataType::Utf8, true).contains(&"contains"));
        assert!(!operators_for(DataType::Int64, true).contains(&"contains"));

        // A boolean has two values and no useful order.
        let flags = operators_for(DataType::Bool, true);
        assert!(flags.contains(&"eq") && !flags.contains(&"gt"));

        // The null tests need a column that can be null.
        assert!(operators_for(DataType::Int64, true).contains(&"is_null"));
        assert!(!operators_for(DataType::Int64, false).contains(&"is_null"));

        // Every operator offered is one the filter row knows.
        for data_type in [
            DataType::Utf8,
            DataType::Int64,
            DataType::Bool,
            DataType::Date,
        ] {
            for op in operators_for(data_type, true) {
                assert!(FILTER_OPERATORS.contains(&op), "{op}");
                assert!(FilterOp::parse(op).is_some(), "{op}");
            }
        }

        assert_eq!(input_type(DataType::Date), "date");
        assert_eq!(input_type(DataType::Bool), "checkbox");
        assert_eq!(input_type(DataType::Int64), "number");
        assert_eq!(input_type(DataType::Utf8), "text");
        // A decimal input must accept its own scale, or the browser rounds it
        // away before anyone sees it.
        assert_eq!(
            input_step(DataType::decimal(12, 2).unwrap()).as_deref(),
            Some("0.01")
        );
        assert_eq!(input_step(DataType::Int64).as_deref(), Some("1"));
        assert_eq!(input_step(DataType::Utf8), None);
    }

    /// Every status has its own sentence; only `Ready` shows the count. The
    /// wording comes from the texts, so the same state reads differently once a
    /// page overrides them (point 48).
    #[test]
    fn the_status_line_has_one_sentence_per_state() {
        let texts = GridTexts::default();
        assert_eq!(status_text(&texts, &GridStatus::Ready, 1), "1 match");
        assert_eq!(
            status_text(&texts, &GridStatus::Ready, 1_234),
            "1234 matches"
        );
        assert_eq!(status_text(&texts, &GridStatus::Loading, 7), "Loading …");
        assert_eq!(status_text(&texts, &GridStatus::Empty, 0), "No matches");

        let failed = GridStatus::Error(texts.error("unknown source \"orders\""));
        assert_eq!(
            status_text(&texts, &failed, 5),
            "The data could not be loaded: unknown source \"orders\""
        );
        assert_eq!(status_state(&failed), "error");

        let german = GridTexts {
            empty: "Keine Treffer".to_owned(),
            ..GridTexts::default()
        };
        assert_eq!(status_text(&german, &GridStatus::Empty, 0), "Keine Treffer");
    }

    /// The result arrives **typed** (point 23): a decimal is a decimal, and a
    /// null is a null — not both of them text.
    #[test]
    fn a_result_keeps_the_types_of_its_columns() {
        let result = r#"{
            "total_count": 7,
            "row_count": 2,
            "columns": [
                { "name": "customer", "type": "utf8", "nullable": true, "values": ["Alpha", null] },
                { "name": "amount", "type": { "decimal": { "precision": 12, "scale": 2 } },
                  "nullable": true, "values": ["10.00", "20.00"] }
            ]
        }"#;
        let parsed = parse_result(result).expect("parses");
        assert_eq!(parsed.total_count, 7);
        assert_eq!(parsed.row_count(), 2);
        assert_eq!(
            parsed
                .schema
                .fields()
                .iter()
                .map(|field| field.data_type)
                .collect::<Vec<_>>(),
            [
                DataType::Utf8,
                DataType::Decimal {
                    precision: 12,
                    scale: 2
                }
            ]
        );
        assert_eq!(
            parsed.columns[0],
            [Value::Utf8("Alpha".to_owned()), Value::Null]
        );
        // And the renderer still turns them into the same text as before.
        assert_eq!(crate::formats::plain_text(&parsed.columns[1][0]), "10.00");
        assert_eq!(crate::formats::plain_text(&parsed.columns[0][1]), "");
    }

    /// A malformed result is an error, not a panic.
    #[test]
    fn a_result_without_columns_is_an_error() {
        assert!(parse_result("{}").is_err());
        assert!(parse_result("not json").is_err());
    }

    /// The skeleton builds role, counts, header and exactly `pool` hidden rows.
    #[test]
    fn the_skeleton_counts_rows_and_columns() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: Some("Bestellungen"),
                schema: &schema,
                pool: 3,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );

        assert_eq!(view.pool(), 3);
        assert_eq!(view.header_cells.len(), 2);
        assert_eq!(view.rows[0].cells.len(), 2);
        assert_eq!(view.filter.columns.len(), 2);

        let attributes = |name: &str| -> Vec<String> {
            buffer
                .patches()
                .iter()
                .filter_map(|patch| match patch {
                    Patch::SetAttribute {
                        name: attr, value, ..
                    } if attr == name => Some(value.clone()),
                    _ => None,
                })
                .collect()
        };

        // The filter group, the status line, the pager and the grid itself
        // carry roles; the status line stays the single polite live region
        // (point 41), and the three groups all sit outside `role="grid"` so the
        // roving tabindex and the keyboard matrix are untouched.
        assert_eq!(
            attributes("role"),
            ["group", "status", "group", "group", "grid"]
        );
        assert_eq!(attributes("aria-live"), ["polite"]);
        // Three accessible names, and each names a different thing: the filter
        // group, the selection column's header (point 61) and the grid itself.
        // Asserted as a set, not by position — the order is where they happen
        // to be built, which is not a promise to anybody.
        // (The filter row adds an operator and a value name per column, so the
        // list is longer than these two.) Asserted as a set, not by position —
        // the order is where they happen to be built, which is not a promise.
        let labels = attributes("aria-label");
        assert!(
            labels.iter().any(|label| label == "Bestellungen"),
            "the grid is not named"
        );
        // The filter group and the column list are named by reference (F9):
        // the name sits in a hidden element that can carry our language.
        assert_eq!(
            attributes("aria-labelledby"),
            ["og-label-filter", "og-label-columns"]
        );
        // Two columns, and no selection column: it is opt-in (point 61).
        assert_eq!(attributes("aria-colcount"), ["2"]);
        // Only the header row carries an index in the skeleton; the pool rows
        // get theirs from `patch_grid`.
        assert_eq!(attributes("aria-rowindex"), ["1"]);
        let parts = attributes("part");
        assert_eq!(&parts[..2], ["layout", "filter"]);
        assert!(parts.iter().any(|part| part == "viewport"));
        assert_eq!(
            parts
                .iter()
                .filter(|part| part.as_str() == "filter-operator")
                .count(),
            2
        );
        // Every header carries both sort marks, not just one somewhere.
        for mark in ["sort-direction", "sort-index"] {
            assert_eq!(
                parts.iter().filter(|part| part.as_str() == mark).count(),
                2,
                "every column needs its own {mark}"
            );
        }
    }

    /// Patching the pool recycles the existing rows: counts, rowindexes and
    /// exactly one `tabindex=0`.
    #[test]
    fn patching_recycles_the_pool() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: Some("Bestellungen"),
                schema: &schema,
                pool: 4,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 4);
        let slots = assign_pool(&[None; 4], None, &window_rows(0, 5, 4), 4);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[],
            None,
            DEFAULT_ROW_HEIGHT,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(state.total_count()),
            None,
        );

        let attributes = |name: &str| -> Vec<String> {
            buffer
                .patches()
                .iter()
                .filter_map(|patch| match patch {
                    Patch::SetAttribute {
                        name: attr, value, ..
                    } if attr == name => Some(value.clone()),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(attributes("aria-rowcount"), ["6"]);
        // The header row (1) is set in the skeleton; this frame patches the four
        // data rows 0..4 → 2..5.
        assert_eq!(attributes("aria-rowindex"), ["2", "3", "4", "5"]);
        // Exactly one tabindex 0: the active header cell.
        let tabindexes = attributes("tabindex");
        assert_eq!(
            tabindexes
                .iter()
                .filter(|value| value.as_str() == "0")
                .count(),
            1
        );
    }

    /// The sorted column is the only one with a non-`none` `aria-sort`.
    #[test]
    fn only_the_sorted_header_carries_aria_sort() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma"], 1, 0, 1);
        let slots = assign_pool(&[None], None, &window_rows(0, 1, 1), 1);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Data(CellRef::new(0, 1)),
            &[("qty".to_owned(), "asc")],
            None,
            DEFAULT_ROW_HEIGHT,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(state.total_count()),
            None,
        );
        let sorts: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { name, value, .. } if name == "aria-sort" => {
                    Some(value.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(sorts, ["none", "ascending"]);
    }

    /// Multi-sort marks each key with its direction and shows its order index.
    #[test]
    fn a_multi_sort_shows_each_key_with_its_order() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma"], 1, 0, 1);
        let slots = assign_pool(&[None], None, &window_rows(0, 1, 1), 1);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[("customer".to_owned(), "asc"), ("qty".to_owned(), "desc")],
            None,
            DEFAULT_ROW_HEIGHT,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(state.total_count()),
            None,
        );

        let aria: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { name, value, .. } if name == "aria-sort" => {
                    Some(value.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(aria, ["ascending", "descending"]);

        let text_of = |node: NodeId| -> String {
            buffer
                .patches()
                .iter()
                .find_map(|patch| match patch {
                    Patch::SetText {
                        node: current,
                        text,
                    } if *current == node => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        assert_eq!(text_of(view.header_cells[0].index), "1");
        assert_eq!(text_of(view.header_cells[1].index), "2");
        assert_eq!(text_of(view.status), "1 match");
    }

    /// Only the component's own texts claim a language — the data must keep the
    /// page's (point 48).
    #[test]
    fn only_the_texts_carry_a_language() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 2,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );

        let tagged: Vec<NodeId> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { node, name, .. } if name == "lang" => Some(*node),
                _ => None,
            })
            .collect();
        // Tagged: every node whose whole subtree is our own wording — the
        // status line, the pager (it writes words too, point 38), the
        // disclosure's toggle, each operator `select` (its options are our
        // words) and the clear button.
        // And the empty state of point 68: its sentence and its button are
        // ours, and it holds no data of the page's.
        let mut expected = vec![
            view.status,
            view.columns.toggle,
            view.pager.container,
            view.empty,
        ];
        expected.push(view.filter.clear);
        for column in &view.filter.columns {
            expected.push(column.select);
        }
        // And the hidden names of the two containers below (F9): the name is
        // ours even where the container's contents are not.
        for id in ["og-label-filter", "og-label-columns"] {
            let label = buffer
                .patches()
                .iter()
                .find_map(|patch| match patch {
                    Patch::SetAttribute { node, name, value } if name == "id" && value == id => {
                        Some(*node)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{id} names its container"));
            expected.push(label);
        }
        for node in &expected {
            assert!(tagged.contains(node), "{node:?} should carry the language");
        }

        // Never tagged: the two containers that hold column names. `lang` is
        // inherited, so tagging either would declare the page's own words
        // English — `part="filter"` counts because the disclosure sits inside
        // it (point 37).
        for node in [
            view.filter.container,
            view.columns.container,
            view.table,
            view.tbody,
        ] {
            assert!(
                !tagged.contains(&node),
                "{node:?} holds the page's data and must not claim a language"
            );
        }
        assert_eq!(tagged.len(), expected.len());

        // No language at all when the texts do not claim one.
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let silent = GridTexts {
            lang: String::new(),
            ..GridTexts::default()
        };
        build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 2,
                texts: &silent,
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        assert!(
            !buffer
                .patches()
                .iter()
                .any(|patch| matches!(patch, Patch::SetAttribute { name, .. } if name == "lang"))
        );
    }

    /// The direction is visible, not only in `aria-sort` (point 49) — and both
    /// come from the same key, so they cannot disagree.
    #[test]
    fn the_header_shows_its_sort_direction() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma"], 1, 0, 1);

        let glyphs = |sorts: &[(String, &str)]| -> Vec<String> {
            let mut buffer = PatchBuffer::new();
            patch_grid(
                &mut buffer,
                &view,
                &state,
                &[Some(0)],
                ActiveCell::Header { col: 0 },
                sorts,
                None,
                DEFAULT_ROW_HEIGHT,
                &GridTexts::default(),
                &crate::formats::Plain,
                Paging::whole(state.total_count()),
                None,
            );
            view.header_cells
                .iter()
                .map(|header| {
                    buffer
                        .patches()
                        .iter()
                        .find_map(|patch| match patch {
                            Patch::SetText { node, text } if *node == header.direction => {
                                Some(text.clone())
                            }
                            _ => None,
                        })
                        .expect("every header's direction is written on every frame")
                })
                .collect()
        };

        assert_eq!(glyphs(&[]), ["", ""]);
        assert_eq!(
            glyphs(&[("customer".to_owned(), "asc")]),
            [ASCENDING_GLYPH, ""]
        );
        assert_eq!(
            glyphs(&[("qty".to_owned(), "desc")]),
            ["", DESCENDING_GLYPH]
        );
        // Multi-sort: every key shows its own direction.
        assert_eq!(
            glyphs(&[("customer".to_owned(), "desc"), ("qty".to_owned(), "asc")]),
            [DESCENDING_GLYPH, ASCENDING_GLYPH]
        );
    }

    /// The exported parts are the theming contract (point 20): a page styles the
    /// grid through them, so the skeleton must name every one of them.
    #[test]
    fn the_skeleton_exports_the_documented_parts() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );

        let mut parts: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { name, value, .. } if name == "part" => Some(value.as_str()),
                _ => None,
            })
            .collect();
        parts.sort_unstable();
        parts.dedup();
        assert_eq!(
            parts,
            [
                "cell",
                "columns",
                "columns-toggle",
                "empty",
                "empty-reset",
                "empty-text",
                "filter",
                "filter-clear",
                "filter-operator",
                "filter-value",
                "header",
                "layout",
                "page-first",
                "page-label",
                "page-last",
                "page-next",
                "page-previous",
                "pager",
                "row",
                "sort-direction",
                "sort-index",
                "status",
                "viewport",
            ]
        );
    }

    /// A range reads "from – to" in the column's format, once when both ends
    /// are the same, and not at all over nothing (F7, S11).
    #[test]
    fn a_range_reads_from_to() {
        use crate::presentation::Summary;
        use opengrid_types::Value;
        let range =
            |values: &[Value]| aggregate_text(Summary::Range, 0, values, &crate::formats::Plain);
        assert_eq!(range(&[Value::Int64(1), Value::Int64(9)]), "1 \u{2013} 9");
        assert_eq!(range(&[Value::Int64(4), Value::Int64(4)]), "4");
        assert_eq!(range(&[Value::Null, Value::Null]), "");
        assert_eq!(
            aggregate_text(
                Summary::Fn(opengrid_query::AggregateFn::Count),
                0,
                &[Value::Int64(3)],
                &crate::formats::Plain
            ),
            "3"
        );
    }

    /// Every scroller is out of the tab sequence. Firefox makes a scroll
    /// container a tab stop of its own even with focusable children, and CI
    /// runs Chromium, which does not — so this is where it has to hold.
    #[test]
    fn no_scroller_is_a_tab_stop() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: true,
                search: false,
            },
        );
        let attribute = |node: NodeId, wanted: &str| {
            buffer.patches().iter().find_map(|patch| match patch {
                Patch::SetAttribute {
                    node: at,
                    name,
                    value,
                } if *at == node && name == wanted => Some(value.clone()),
                _ => None,
            })
        };
        for part in ["filter", "facets", "viewport"] {
            let node = buffer
                .patches()
                .iter()
                .find_map(|patch| match patch {
                    Patch::SetAttribute { node, name, value }
                        if name == "part" && value == part =>
                    {
                        Some(*node)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the skeleton has part={part}"));
            assert_eq!(attribute(node, "tabindex").as_deref(), Some("-1"), "{part}");
        }
    }

    /// A control that takes its text colour from the theme takes its background
    /// from it too. Otherwise the background is the system's `ButtonFace` or
    /// `Field`, which follows the page's `color-scheme` and not the theme: the
    /// light ink of a dark theme on a mid-grey button read at 4.34:1 (point 69).
    #[test]
    fn a_themed_control_colour_comes_with_its_background() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: true,
                column_menu: true,
                toolbar: true,
                facets: true,
                search: true,
            },
        );
        let styles = buffer
            .patches()
            .iter()
            .find_map(|patch| match patch {
                Patch::SetText { text, .. } if text.contains(":host") => Some(text.clone()),
                _ => None,
            })
            .expect("the skeleton carries a stylesheet");

        let mut checked = 0;
        for rule in styles.split('}') {
            let Some((selector, body)) = rule.split_once('{') else {
                continue;
            };
            let control = ["button", "input", "select"]
                .iter()
                .any(|tag| selector.split([' ', ',', '>']).any(|part| part == *tag));
            if control && body.contains("color:") && !body.contains("accent-color") {
                checked += 1;
                assert!(
                    body.contains("background"),
                    "{} sets a colour but leaves the background to the system",
                    selector.trim()
                );
            }
        }
        assert!(checked >= 4, "the check found the control rules");
    }

    /// Every themeable property is declared on `:host` with a default, so a page
    /// can override one without knowing the others.
    #[test]
    fn the_stylesheet_declares_every_custom_property() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );

        let styles = buffer
            .patches()
            .iter()
            .find_map(|patch| match patch {
                Patch::SetText { text, .. } if text.contains(":host") => Some(text.clone()),
                _ => None,
            })
            .expect("the skeleton carries a stylesheet");

        for property in SET_TOKENS.iter().chain(COMPUTED_TOKENS) {
            assert!(
                styles.contains(&format!("{property}:")),
                "{property} has no default"
            );
            assert!(
                styles.contains(&format!("var({property})")),
                "{property} is declared but never used"
            );
        }

        // Every computed colour is reset under a forced palette. A `color-mix`
        // of two system colours resolves unpredictably, and the user's palette
        // is the one that has to win.
        let forced = styles
            .split("forced-colors: active")
            .nth(1)
            .expect("the stylesheet has a forced-colors block");
        let forced = &forced[..forced
            .find("prefers-reduced-motion")
            .unwrap_or(forced.len())];
        for property in COMPUTED_TOKENS {
            assert!(
                forced.contains(&format!("{property}:")),
                "{property} survives a forced palette"
            );
        }
        assert!(
            !forced.contains("color-mix"),
            "a forced palette must resolve to system colours, not to a mix"
        );

        // The focus ring does not hang off the accent: a pale accent would make
        // it invisible, and the ring is not decoration.
        let ring = styles
            .split("th:focus, td:focus")
            .nth(1)
            .expect("the stylesheet draws a focus ring");
        let ring = &ring[..ring.find('}').unwrap_or(ring.len())];
        assert!(
            ring.contains("Highlight") && !ring.contains(ACCENT_PROPERTY),
            "the focus ring must not be themable away"
        );

        // The two guarantees a theme must not be able to switch off.
        assert!(styles.contains("forced-colors: active"));
        assert!(styles.contains("prefers-reduced-motion: reduce"));
    }

    /// The unfolding of a truncated value (point 47) is three rules that only
    /// work together: wrapping, breaking a word without spaces, and a raised,
    /// opaque row to grow over the ones below.
    #[test]
    fn the_stylesheet_unfolds_the_focused_cell() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );

        let styles = buffer
            .patches()
            .iter()
            .find_map(|patch| match patch {
                Patch::SetText { text, .. } if text.contains(":host") => Some(text.clone()),
                _ => None,
            })
            .expect("the skeleton carries a stylesheet");

        for rule in [
            "tbody td:focus",
            "white-space: normal",
            "overflow-wrap: anywhere",
            "tbody tr:has(:focus)",
            "z-index: 1",
            "scroll-margin-top",
        ] {
            assert!(styles.contains(rule), "the unfold rules lost {rule:?}");
        }
    }

    /// A single sort leaves the header's order index empty.
    #[test]
    fn a_single_sort_hides_the_order_index() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 1,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma"], 1, 0, 1);
        let slots = assign_pool(&[None], None, &window_rows(0, 1, 1), 1);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[("customer".to_owned(), "asc")],
            None,
            DEFAULT_ROW_HEIGHT,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(state.total_count()),
            None,
        );
        let indexes: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetText { node, text } if *node == view.header_cells[0].index => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(indexes, [""]);
    }

    /// The sizer height is `total_count * row_height`.
    #[test]
    fn the_sizer_reflects_the_total_count() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 2,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &Default::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 2);
        let slots = assign_pool(&[None, None], None, &window_rows(0, 5, 2), 2);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[],
            None,
            DEFAULT_ROW_HEIGHT,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(state.total_count()),
            None,
        );
        assert!(buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::SetAttribute { node, name, value }
                if *node == view.tbody && name == "style"
                    && *value == format!("position: relative; height: {}px;", 5 * DEFAULT_ROW_HEIGHT)
        )));
    }

    /// A non-default row height drives both the sizer and the row offsets.
    #[test]
    fn a_custom_row_height_rescales_the_sizer_and_the_rows() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 3,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        let state = state_with(&["Gamma", "Alpha"], 5, 0, 3);
        let slots = assign_pool(&[None, None, None], None, &window_rows(0, 5, 3), 3);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state,
            &slots,
            ActiveCell::Header { col: 0 },
            &[],
            None,
            48,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(state.total_count()),
            None,
        );

        // The sizer is `total * 48`, not `total * 32`.
        assert!(buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::SetAttribute { node, name, value }
                if *node == view.tbody && name == "style" && value == "position: relative; height: 240px;"
        )));
        // Row 2 sits at `2 * 48`.
        assert!(buffer.patches().iter().any(|patch| matches!(
            patch,
            Patch::SetAttribute { node, name, value }
                if *node == view.rows[2].row && name == "style" && value == "transform: translateY(96px);"
        )));
    }

    /// The pinned slot is not patched, so the focused node keeps its `data-row`.
    #[test]
    fn the_pinned_slot_is_left_untouched() {
        let schema = initial_schema(&["customer".to_owned()]);
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        let view = build_grid(
            &mut buffer,
            &mut nodes,
            &GridSkeleton {
                label: None,
                schema: &schema,
                pool: 4,
                texts: &GridTexts::default(),
                declared: &[],
                presentation: &crate::presentation::ColumnStyles::default(),
                selection: false,
                column_menu: false,
                toolbar: false,
                facets: false,
                search: false,
            },
        );
        // Slot 3 holds the focused row 6; the window scrolled to rows 20..24.
        let old = [Some(20), Some(21), Some(22), Some(6)];
        let focus = Some(6);
        let rows = window_rows(20, 100, 4);
        let slots = assign_pool(&old, focus, &rows, 4);
        let pinned = old.iter().position(|slot| *slot == focus);

        let mut buffer = PatchBuffer::new();
        patch_grid(
            &mut buffer,
            &view,
            &state_with(&[], 100, 20, 4),
            &slots,
            ActiveCell::Data(CellRef::new(6, 0)),
            &[],
            pinned,
            DEFAULT_ROW_HEIGHT,
            &GridTexts::default(),
            &crate::formats::Plain,
            Paging::whole(100),
            None,
        );

        let pinned_row = view.rows[3].row;
        // Its identity and position must not move — that is what keeps the
        // browser's focus where it is. Since point 35 its **selection** does
        // follow: `aria-selected` touches neither, and without it, selecting
        // the row you are standing on would show nothing.
        let touched: Vec<&str> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetAttribute { node, name, .. } if *node == pinned_row => {
                    Some(name.as_str())
                }
                Patch::RemoveAttribute { node, name } if *node == pinned_row => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            touched,
            ["aria-selected", "data-selected"],
            "only the selection may be written to the pinned row"
        );
        assert!(
            !buffer.patches().iter().any(|patch| matches!(
                patch,
                Patch::SetText { node, .. } if *node == pinned_row
            )),
            "the pinned row keeps its text"
        );
        assert!(slots.contains(&focus));
    }

    /// Arrow keys move by one and clamp to the whole result, not the window.
    #[test]
    fn arrows_move_and_clamp() {
        let active = ActiveCell::Data(CellRef::new(0, 0));
        let down = move_active(active, GridKey::ArrowDown, 2, 5, 3, true);
        assert_eq!(down, ActiveCell::Data(CellRef::new(1, 0)));
        // At the last row the move is clamped.
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(4, 0)),
                GridKey::ArrowDown,
                2,
                5,
                3,
                true,
            ),
            ActiveCell::Data(CellRef::new(4, 0))
        );
        // Up from the first row enters the header.
        assert_eq!(
            move_active(active, GridKey::ArrowUp, 2, 5, 3, true),
            ActiveCell::Header { col: 0 }
        );
        // Down from the header enters the first row.
        assert_eq!(
            move_active(
                ActiveCell::Header { col: 0 },
                GridKey::ArrowDown,
                2,
                5,
                3,
                true
            ),
            ActiveCell::Data(CellRef::new(0, 0))
        );
        // Right/left clamp at the last/first column.
        assert_eq!(
            move_active(active, GridKey::ArrowRight, 2, 5, 3, true),
            ActiveCell::Data(CellRef::new(0, 1))
        );
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(0, 1)),
                GridKey::ArrowRight,
                2,
                5,
                3,
                true,
            ),
            ActiveCell::Data(CellRef::new(0, 1))
        );
        // Left from the first schema column is the selection cell — since
        // point 61 that is the start of the row, not column 0.
        assert_eq!(
            move_active(active, GridKey::ArrowLeft, 2, 5, 3, true),
            ActiveCell::Select { row: 0 }
        );
        assert_eq!(
            move_active(
                ActiveCell::Select { row: 0 },
                GridKey::ArrowLeft,
                2,
                5,
                3,
                true
            ),
            ActiveCell::Select { row: 0 }
        );
        assert_eq!(
            move_active(
                ActiveCell::Select { row: 0 },
                GridKey::ArrowRight,
                2,
                5,
                3,
                true
            ),
            ActiveCell::Data(CellRef::new(0, 0))
        );
        // The selection column has a header of its own, and the vertical moves
        // treat it like any other column.
        assert_eq!(
            move_active(
                ActiveCell::Select { row: 0 },
                GridKey::ArrowUp,
                2,
                5,
                3,
                true
            ),
            ActiveCell::SelectAll
        );
        assert_eq!(
            move_active(ActiveCell::SelectAll, GridKey::ArrowDown, 2, 5, 3, true),
            ActiveCell::Select { row: 0 }
        );
        assert_eq!(
            move_active(ActiveCell::SelectAll, GridKey::ArrowRight, 2, 5, 3, true),
            ActiveCell::Header { col: 0 }
        );
        assert_eq!(
            move_active(
                ActiveCell::Header { col: 0 },
                GridKey::ArrowLeft,
                2,
                5,
                3,
                true
            ),
            ActiveCell::SelectAll
        );
    }

    /// Home/End move within the row; Ctrl+Home/End jump to the grid's corners.
    #[test]
    fn home_end_and_ctrl_jump() {
        let active = ActiveCell::Data(CellRef::new(1, 1));
        // `Home` is the start of the row, and since point 61 that is the
        // selection cell — the same place the eye starts.
        assert_eq!(
            move_active(active, GridKey::Home, 2, 5, 3, true),
            ActiveCell::Select { row: 1 }
        );
        assert_eq!(
            move_active(active, GridKey::End, 2, 5, 3, true),
            ActiveCell::Data(CellRef::new(1, 1))
        );
        assert_eq!(
            move_active(active, GridKey::CtrlHome, 2, 5, 3, true),
            ActiveCell::SelectAll
        );
        assert_eq!(
            move_active(active, GridKey::CtrlEnd, 2, 5, 3, true),
            ActiveCell::Data(CellRef::new(4, 1))
        );
    }

    /// Page keys move by a viewport and clamp to the whole result.
    #[test]
    fn page_keys_move_by_a_viewport() {
        let active = ActiveCell::Data(CellRef::new(0, 0));
        let next = move_active(active, GridKey::PageDown, 2, 100, 10, true);
        assert_eq!(next, ActiveCell::Data(CellRef::new(10, 0)));
        let up = move_active(
            ActiveCell::Data(CellRef::new(30, 0)),
            GridKey::PageUp,
            2,
            100,
            10,
            true,
        );
        assert_eq!(up, ActiveCell::Data(CellRef::new(20, 0)));
        // At the last row the move clamps.
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(99, 0)),
                GridKey::PageDown,
                2,
                100,
                10,
                true,
            ),
            ActiveCell::Data(CellRef::new(99, 0))
        );
    }

    /// A move inside the window asks for no reload; a move outside does.
    #[test]
    fn a_move_outside_the_window_requests_it() {
        let window = Window::new(0, 10);
        assert_eq!(
            requested_window(
                GridKey::ArrowDown,
                ActiveCell::Data(CellRef::new(5, 0)),
                window,
                100,
                10
            ),
            None
        );
        assert_eq!(
            requested_window(
                GridKey::ArrowDown,
                ActiveCell::Data(CellRef::new(10, 0)),
                window,
                100,
                10
            ),
            Some(4)
        );
        assert_eq!(
            requested_window(
                GridKey::CtrlEnd,
                ActiveCell::Data(CellRef::new(99, 1)),
                window,
                100,
                10
            ),
            Some(90)
        );
        assert_eq!(
            requested_window(
                GridKey::CtrlHome,
                ActiveCell::Header { col: 0 },
                Window::new(40, 10),
                100,
                10
            ),
            Some(0)
        );
    }

    /// The visible row is derived from the scroll offset and the row height.
    #[test]
    fn scroll_offset_maps_to_the_visible_row() {
        assert_eq!(visible_start(0, DEFAULT_ROW_HEIGHT), 0);
        assert_eq!(visible_start(DEFAULT_ROW_HEIGHT - 1, DEFAULT_ROW_HEIGHT), 0);
        assert_eq!(visible_start(DEFAULT_ROW_HEIGHT, DEFAULT_ROW_HEIGHT), 1);
        assert_eq!(
            visible_start(1_000_000, DEFAULT_ROW_HEIGHT),
            1_000_000 / DEFAULT_ROW_HEIGHT
        );
        // A configured row height scales the same math.
        assert_eq!(visible_start(95, 48), 1);
        assert_eq!(visible_start(96, 48), 2);
        assert_eq!(visible_start(480, 48), 10);
    }

    /// The window keeps the overscan above the visible row and clamps at the end.
    #[test]
    fn window_offset_clamps_to_the_result() {
        assert_eq!(window_offset(0, 100, 10), 0);
        assert_eq!(window_offset(50, 100, 10), 44);
        assert_eq!(window_offset(99, 100, 10), 90);
        assert_eq!(window_offset_for_row(0, 100, 10), 0);
        assert_eq!(window_offset_for_row(99, 100, 10), 90);
        assert_eq!(window_rows(90, 100, 10), (90..100).collect::<Vec<_>>());
        // A pool shorter than the overscan still contains its row.
        assert_eq!(window_offset_for_row(2, 5, 2), 1);
    }

    /// A fresh assignment fills the free slots in order.
    #[test]
    fn assignment_fills_free_slots() {
        let slots = assign_pool(&[None; 3], None, &[0, 1, 2], 3);
        assert_eq!(slots, [Some(0), Some(1), Some(2)]);
    }

    /// Existing assignments are kept when the row is still in the window.
    #[test]
    fn assignment_keeps_existing_rows() {
        let old = [Some(0), Some(1), Some(2)];
        let slots = assign_pool(&old, None, &[0, 1, 2, 3], 3);
        assert_eq!(slots, [Some(0), Some(1), Some(2)]);
    }

    /// **No frame ever patches the shadow root itself.**
    ///
    /// [`NodeId::ROOT`] is a legal node, so a `SetText` aimed at it replaces
    /// every child the skeleton built — the whole grid disappears, in silence,
    /// and every DOM test times out waiting for a row. That is what an
    /// `Option`-shaped node was used as a sentinel for, once. This test is the
    /// wall that keeps it from happening again: the portable patch tests cannot
    /// see the consequence, because destroying a subtree is a DOM semantic and
    /// they hold no DOM.
    #[test]
    fn no_frame_writes_to_the_shadow_root() {
        for selection in [false, true] {
            let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
            let mut nodes = NodeAllocator::new();
            let mut buffer = PatchBuffer::new();
            let view = build_grid(
                &mut buffer,
                &mut nodes,
                &GridSkeleton {
                    label: None,
                    schema: &schema,
                    pool: 2,
                    texts: &GridTexts::default(),
                    declared: &[],
                    presentation: &Default::default(),
                    selection,
                    column_menu: false,
                    toolbar: false,
                    facets: false,
                    search: false,
                },
            );

            let mut state = GridState::new(schema);
            state.set_window(Window::new(0, 2));
            let mut frame = PatchBuffer::new();
            patch_grid(
                &mut frame,
                &view,
                &state,
                &[Some(0), Some(1)],
                ActiveCell::Header { col: 0 },
                &[],
                None,
                DEFAULT_ROW_HEIGHT,
                &GridTexts::default(),
                &crate::formats::Plain,
                Paging::whole(state.total_count()),
                None,
            );

            for patch in frame.patches() {
                let node = match patch {
                    Patch::SetAttribute { node, .. }
                    | Patch::RemoveAttribute { node, .. }
                    | Patch::SetText { node, .. } => Some(*node),
                    _ => None,
                };
                assert_ne!(
                    node,
                    Some(NodeId::ROOT),
                    "selection={selection}: a frame patched the shadow root ({patch:?})"
                );
            }
        }
    }

    /// Without the attribute nothing of the selection column exists — no cell,
    /// no header, and no place on the keyboard axis. It is opt-in for the same
    /// reason the filter row and the column menu are: a grid that is read
    /// rather than worked with should not carry a control that does nothing
    /// for it, and a reader should not be told there is a column there.
    #[test]
    fn the_selection_column_is_opt_in() {
        let schema = initial_schema(&["customer".to_owned(), "qty".to_owned()]);
        let build = |selection: bool| {
            let mut nodes = NodeAllocator::new();
            let mut buffer = PatchBuffer::new();
            build_grid(
                &mut buffer,
                &mut nodes,
                &GridSkeleton {
                    label: None,
                    schema: &schema,
                    pool: 1,
                    texts: &GridTexts::default(),
                    declared: &[],
                    presentation: &Default::default(),
                    selection,
                    column_menu: false,
                    toolbar: false,
                    facets: false,
                    search: false,
                },
            );
            buffer
        };

        let value_of = |buffer: &PatchBuffer, wanted: &str| -> Vec<String> {
            buffer
                .patches()
                .iter()
                .filter_map(|patch| match patch {
                    Patch::SetAttribute { name, value, .. } if name == wanted => {
                        Some(value.clone())
                    }
                    _ => None,
                })
                .collect()
        };

        let without = build(false);
        assert_eq!(value_of(&without, "aria-colcount"), ["2"]);
        assert!(value_of(&without, "data-select").is_empty());
        assert!(!value_of(&without, "role").iter().any(|r| r == "checkbox"));

        let with = build(true);
        assert_eq!(value_of(&with, "aria-colcount"), ["3"]);
        assert_eq!(value_of(&with, "data-select"), ["all", "row"]);
        assert!(value_of(&with, "role").iter().any(|r| r == "checkbox"));
    }

    /// The keyboard axis has the column only when the column is there.
    #[test]
    fn home_stops_at_the_first_schema_column_without_the_selection_column() {
        let at = ActiveCell::Data(CellRef::new(1, 1));
        assert_eq!(
            move_active(at, GridKey::Home, 2, 5, 3, false),
            ActiveCell::Data(CellRef::new(1, 0))
        );
        assert_eq!(
            move_active(at, GridKey::CtrlHome, 2, 5, 3, false),
            ActiveCell::Header { col: 0 }
        );
        // And left from the first column stays put rather than falling off.
        assert_eq!(
            move_active(
                ActiveCell::Data(CellRef::new(1, 0)),
                GridKey::ArrowLeft,
                2,
                5,
                3,
                false,
            ),
            ActiveCell::Data(CellRef::new(1, 0))
        );
    }

    /// The focused row is pinned even when it left the window.
    #[test]
    fn assignment_pins_the_focused_row() {
        let old = [Some(6), Some(20), Some(21), Some(22)];
        let slots = assign_pool(&old, Some(6), &[20, 21, 22, 23], 4);
        assert_eq!(slots[0], Some(6));
        assert!(!slots.contains(&Some(23)));
    }

    /// When nothing is pinned the window fills the pool, dropping the far edge.
    #[test]
    fn assignment_scrolls_without_focus() {
        let old = [Some(0), Some(1), Some(2), Some(3)];
        let slots = assign_pool(&old, None, &[10, 11, 12, 13], 4);
        assert_eq!(slots, [Some(10), Some(11), Some(12), Some(13)]);
    }
}
