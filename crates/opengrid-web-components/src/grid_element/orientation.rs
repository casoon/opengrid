//! Where am I (issue #525): the crosshair on the focused cell's row and column,
//! the position and a range's sums in the footer, the rows in sight while a
//! fling or a thumb drag runs, and a shadow at each edge with columns past it.
//!
//! All of it is drawn only. The cells say their place (`aria-rowindex`,
//! `aria-colindex`) and the grid announces every move and every range, so the
//! footer's line is not live and the overlays are `aria-hidden`.

use super::*;

use std::borrow::Cow;

use opengrid_grid::CellRange;
use opengrid_grid::range::{RangeSummary, summarize};

/// How long after the last scroll event the bubble goes, where the browser
/// sends no `scrollend` (Safari).
const BUBBLE_IDLE_MS: i32 = 400;
/// Scroll events further apart than this begin a new gesture.
const GESTURE_GAP_MS: f64 = 200.0;
/// How far a gesture has to move, in viewport heights, before the bubble shows:
/// a wheel step or `PageDown` moves less, a fling or a thumb drag more.
const BUBBLE_DISTANCE: f64 = 2.0;
/// Waits this long after a range changed before its sums are asked for, so
/// `Shift`+`↓` held down asks once.
const RANGE_ASK_MS: i32 = 250;
/// A range reaching past the loaded rows is summed when it has at most this
/// many rows; a larger one says its size only.
pub(super) const RANGE_ROWS_ASKED: u64 = 100_000;
/// The width of an edge's shadow, as the stylesheet draws it.
const EDGE_WIDTH: i32 = 10;

/// The runtime's share: a range's sums as the source answered them, and the
/// scroll gesture under way.
#[derive(Default)]
pub(super) struct Orientation {
    /// The sums of a range past the loaded rows, with the result (`epoch`)
    /// they were asked under.
    answered: Option<(CellRange, u64, RangeSummary)>,
    /// The range asked for, or waiting to be.
    asked: Option<(CellRange, u64)>,
    /// When the gesture's last scroll event came, and how far it moved.
    gesture_at: f64,
    gesture: f64,
    /// Bumped by every scroll event: an older idle timer finds it changed.
    scroll_round: u64,
}

/// A number as the page reads it: "1,234" or "1.234".
fn number(host: &HtmlElement, value: u64) -> String {
    let lang = host
        .closest("[lang]")
        .ok()
        .flatten()
        .and_then(|owner| owner.get_attribute("lang"))
        .unwrap_or_default();
    let lang = lang.trim();
    js_sys::Number::from(value as f64)
        .to_locale_string(if lang.is_empty() { "default" } else { lang })
        .into()
}

/// Brings the crosshair and the footer's line up to the focus and the range.
pub(super) fn show(host: &HtmlElement) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let active = runtime.borrow().active.data();
    crosshair(&root, active.map(|cell| cell.col));
    let text = position_text(host, &runtime).unwrap_or_default();
    if let Ok(Some(node)) = root.query_selector("[part=\"position\"]")
        && node.text_content().unwrap_or_default() != text
    {
        node.set_text_content(Some(&text));
    }
}

/// Marks the focused cell's column header with `data-crosshair`; the
/// stylesheet tints it, and the row by `:focus-within`, while the focus is
/// in the table.
fn crosshair(root: &ShadowRoot, col: Option<usize>) {
    let wanted = col.map(|col| col.to_string());
    if let Ok(marked) = root.query_selector_all("thead th[data-crosshair]") {
        for index in 0..marked.length() {
            if let Some(th) = marked
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
                && th.get_attribute("data-col") != wanted
            {
                let _ = th.remove_attribute("data-crosshair");
            }
        }
    }
    if let Some(col) = wanted
        && let Ok(Some(th)) = root.query_selector(&format!("thead th[data-col=\"{col}\"]"))
        && !th.has_attribute("data-crosshair")
    {
        let _ = th.set_attribute("data-crosshair", "");
    }
}

/// "Row 1,234 of 5,000 · Customer"; the column alone where a row has no
/// number of its own (grouped, a tree, the new row); a range's size and sums.
fn position_text(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) -> Option<String> {
    let texts = texts(host);
    let range = runtime.borrow().range.filter(|range| !range.is_single());
    if let Some(range) = range {
        return Some(range_text(host, runtime, range));
    }
    let borrowed = runtime.borrow();
    let cell = borrowed.active.data()?;
    let name = borrowed
        .state
        .schema()
        .fields()
        .get(cell.col)?
        .name
        .as_str();
    if is_grouped(host) || is_tree(host) || borrowed.state.row_kind(cell.row) == RowKind::Draft {
        return Some(texts.column(name).to_owned());
    }
    Some(texts.position_cell(
        &number(host, cell.row + 1),
        &number(host, borrowed.state.row_count()),
        name,
    ))
}

/// "3 by 12 cells · Count: 36 · Sum: 1,234.50 · Average: 34.29": the sums of
/// the loaded cells, or of the source's answer for a range past them — until
/// it comes, the size alone.
fn range_text(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, range: CellRange) -> String {
    use crate::presentation::Summary;
    use opengrid_query::AggregateFn;
    let texts = texts(host);
    let (rows, cols) = range.size();
    let size = texts.range_size(rows, cols);
    let Some(summary) = summary_of(host, runtime, range) else {
        return size;
    };
    let mut parts = vec![
        size,
        texts.aggregate_cell(
            Summary::Fn(AggregateFn::Count),
            &number(host, summary.count),
        ),
    ];
    let borrowed = runtime.borrow();
    let schema = borrowed.state.schema();
    let styles = presentation::styles(host);
    // In the column's format where every number is of one column — not one
    // that shows labels for its keys, whose sum is no key.
    let column = summary.column.filter(|col| {
        schema
            .fields()
            .get(*col)
            .is_some_and(|field| styles.lookup(field.name.as_str()).is_none())
    });
    // The sums are of what the reader reads (issue #537): a scaled column's
    // cells are summed in their scale already ([`summarize_shown`]), so the
    // format is the column's own, without the scale again.
    let format = Formatter::new(&formats(host), schema);
    let text = |value: &Value| match column {
        Some(col) => format.text(col, value),
        None => crate::formats::plain_text(value),
    };
    if let (Some(sum), Some(average)) = (&summary.sum, &summary.average) {
        parts.push(texts.aggregate_cell(Summary::Fn(AggregateFn::Sum), &text(sum)));
        parts.push(texts.aggregate_cell(Summary::Fn(AggregateFn::Avg), &text(average)));
    }
    parts.join(" · ")
}

/// The sums of a range: from the loaded rows when it lies within them, from
/// the source's answer otherwise — asked for here when there is none yet.
fn summary_of(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    range: CellRange,
) -> Option<RangeSummary> {
    let (first_row, last_row) = range.rows();
    let (first_col, last_col) = range.cols();
    let epoch = {
        let borrowed = runtime.borrow();
        let state = &borrowed.state;
        // The window is one run of rows: both ends loaded is all of it loaded.
        if state.cell(CellRef::new(first_row, first_col)).is_some()
            && state.cell(CellRef::new(last_row, first_col)).is_some()
        {
            let cells = (first_row..=last_row).flat_map(|row| {
                (first_col..=last_col).filter_map(move |col| {
                    state.cell(CellRef::new(row, col)).map(|value| (col, value))
                })
            });
            return Some(summarize_shown(&scales_of(host, state.schema()), cells));
        }
        if let Some((asked, at, summary)) = &borrowed.orientation.answered
            && *asked == range
            && *at == borrowed.epoch
        {
            return Some(summary.clone());
        }
        borrowed.epoch
    };
    let (rows, _) = range.size();
    if rows <= RANGE_ROWS_ASKED && runtime.borrow().orientation.asked != Some((range, epoch)) {
        runtime.borrow_mut().orientation.asked = Some((range, epoch));
        ask_later(host, range, epoch);
    }
    None
}

/// The places of each column that stores its value × 10^scale (issue #537).
fn scales_of(host: &HtmlElement, schema: &opengrid_types::Schema) -> Vec<Option<u8>> {
    let styles = presentation::styles(host);
    schema
        .fields()
        .iter()
        .map(|field| styles.scale(field.name.as_str()))
        .collect()
}

/// [`summarize`] of what the reader reads: a scaled column's cells in their
/// scale, so that the sum of a range is the sum of its numbers — exact, whole
/// cents added as integers — whichever columns it spans.
fn summarize_shown<'a>(
    scales: &[Option<u8>],
    cells: impl IntoIterator<Item = (usize, &'a Value)>,
) -> RangeSummary {
    let shown: Vec<(usize, Cow<Value>)> = cells
        .into_iter()
        .map(|(col, value)| match scales.get(col).copied().flatten() {
            Some(scale) => (col, Cow::Owned(opengrid_grid::scaled::shown(value, scale))),
            None => (col, Cow::Borrowed(value)),
        })
        .collect();
    summarize(shown.iter().map(|(col, value)| (*col, value.as_ref())))
}

/// Asks the source for the range's values once the range has held still.
fn ask_later(host: &HtmlElement, range: CellRange, epoch: u64) {
    let host = host.clone();
    let callback = Closure::once_into_js(move || {
        let Some(runtime) = runtime(&host) else {
            return;
        };
        let current = {
            let borrowed = runtime.borrow();
            borrowed.range == Some(range) && borrowed.epoch == epoch
        };
        if current {
            ask_range(&host, &runtime, range, epoch);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.unchecked_ref(),
            RANGE_ASK_MS,
        );
    }
}

/// One query for the range's columns over its rows — the grid's filter and
/// order, offset to its first row — summed here as the loaded rows are. The
/// query model sums over groups, not over positions, so the values come.
fn ask_range(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>, range: CellRange, epoch: u64) {
    let (Some(provider), Some(source)) = (provider(host), host.get_attribute(DATASOURCE_ATTRIBUTE))
    else {
        return;
    };
    let (first_row, _) = range.rows();
    let (first_col, last_col) = range.cols();
    let (rows, _) = range.size();
    let (query, names) = {
        let borrowed = runtime.borrow();
        let Ok(filter) = effective_filter(host, &borrowed, None) else {
            return;
        };
        let names: Vec<String> = (first_col..=last_col)
            .filter_map(|col| borrowed.state.schema().fields().get(col))
            .map(|field| field.name.as_str().to_owned())
            .collect();
        let sorts = borrowed.state.sort_keys();
        // A sort names an output column (S6): the order's columns come along.
        let mut select = names.clone();
        for (field, _) in &sorts {
            if !select.contains(field) {
                select.push(field.clone());
            }
        }
        let query = grid::query_json(&source, &select, &sorts, filter.as_ref(), first_row, rows);
        (query, names)
    };
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let host = host.clone();
    let runtime = runtime.clone();
    spawn_local(async move {
        let Ok(result) = ask_uncounted(&host, &provider, &query, &mode).await else {
            return;
        };
        // Each answered column by its place in the grid, by name.
        let places: Vec<Option<usize>> = result
            .schema
            .fields()
            .iter()
            .map(|field| {
                names
                    .iter()
                    .position(|name| name == field.name.as_str())
                    .map(|index| first_col + index)
            })
            .collect();
        let scales = scales_of(&host, runtime.borrow().state.schema());
        let summary = summarize_shown(
            &scales,
            result
                .columns
                .iter()
                .zip(&places)
                .filter_map(|(values, col)| col.map(|col| (col, values)))
                .flat_map(|(col, values)| values.iter().map(move |value| (col, value))),
        );
        {
            let mut borrowed = runtime.borrow_mut();
            if borrowed.range != Some(range) || borrowed.epoch != epoch {
                return;
            }
            borrowed.orientation.answered = Some((range, epoch, summary));
        }
        show(&host);
    });
}

/// The viewport's inner box — inside its border and scroll bars — in the
/// coordinates of the box the overlays are placed in.
fn inner_box(viewport: &HtmlElement) -> (i32, i32, i32, i32) {
    (
        viewport.offset_left() + viewport.client_left(),
        viewport.offset_top() + viewport.client_top(),
        viewport.client_width(),
        viewport.client_height(),
    )
}

/// The overlay `part` beside the viewport, as an element to place.
fn overlay(root: &ShadowRoot, part: &str) -> Option<HtmlElement> {
    root.query_selector(&format!("[part=\"{part}\"]"))
        .ok()
        .flatten()
        .and_then(|node| node.dyn_into::<HtmlElement>().ok())
}

fn set_shown(node: &Element, shown: bool) {
    if node.has_attribute("data-shown") != shown {
        let _ = node.toggle_attribute("data-shown");
    }
}

/// The scroll padding at one side, in pixels: how wide the pinned columns
/// there are ([`crate::pins`]).
fn padding(viewport: &Element, side: &str) -> i32 {
    web_sys::window()
        .and_then(|window| window.get_computed_style(viewport).ok().flatten())
        .and_then(|style| {
            style
                .get_property_value(&format!("scroll-padding-{side}"))
                .ok()
        })
        .and_then(|value| value.trim_end_matches("px").parse::<f64>().ok())
        .map_or(0, |value| value.round() as i32)
}

/// Shows a shadow at each edge that has columns past it, inside the pinned
/// columns there. Called when the viewport scrolls and when it or the table
/// changes size.
pub(super) fn edges(root: &ShadowRoot) {
    let Some(viewport) = root
        .query_selector("[part~=\"viewport\"]")
        .ok()
        .flatten()
        .and_then(|node| node.dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    let (Some(start), Some(end)) = (overlay(root, "edge-start"), overlay(root, "edge-end")) else {
        return;
    };
    let rtl = viewport.matches(":dir(rtl)").unwrap_or(false);
    let (left, right) = if rtl { (&end, &start) } else { (&start, &end) };
    let reach = (viewport.scroll_width() - viewport.client_width()).max(0);
    // Right to left the offset runs from 0 down to `-reach`.
    let from_left = if rtl {
        reach + viewport.scroll_left()
    } else {
        viewport.scroll_left()
    };
    let (x, y, width, height) = inner_box(&viewport);
    let (pad_left, pad_right) = (padding(&viewport, "left"), padding(&viewport, "right"));
    for (node, x, shown) in [
        (left, x + pad_left, from_left > 0),
        (
            right,
            x + width - pad_right - EDGE_WIDTH,
            from_left < reach - 1,
        ),
    ] {
        let style = node.style();
        let _ = style.set_property("transform", &format!("translate({x}px, {y}px)"));
        let _ = style.set_property("height", &format!("{height}px"));
        set_shown(node, shown);
    }
}

/// A scroll of the viewport moved it by `moved` pixels: a gesture that went
/// far enough shows the rows in sight at the scroll bar, until it ends.
pub(super) fn on_scroll(host: &HtmlElement, viewport: &Element, moved: f64, first_row: u64) {
    let (Some(runtime), Some(root)) = (runtime(host), host.shadow_root()) else {
        return;
    };
    let Some(viewport) = viewport.dyn_ref::<HtmlElement>() else {
        return;
    };
    let Some(bubble) = overlay(&root, "scroll-position") else {
        return;
    };
    let now = now();
    let (far, round) = {
        let mut borrowed = runtime.borrow_mut();
        let orientation = &mut borrowed.orientation;
        if now - orientation.gesture_at > GESTURE_GAP_MS {
            orientation.gesture = 0.0;
        }
        orientation.gesture_at = now;
        orientation.gesture += moved;
        orientation.scroll_round += 1;
        (
            orientation.gesture > BUBBLE_DISTANCE * f64::from(viewport.client_height()),
            orientation.scroll_round,
        )
    };
    if !far && !bubble.has_attribute("data-shown") {
        return;
    }
    let (row_height, total) = {
        let borrowed = runtime.borrow();
        (borrowed.row_height.max(1), borrowed.state.row_count())
    };
    let header = root
        .query_selector("thead")
        .ok()
        .flatten()
        .and_then(|node| node.dyn_into::<HtmlElement>().ok())
        .map_or(0, |thead| thead.offset_height());
    let (x, y, width, height) = inner_box(viewport);
    let rows_in_sight = (u64::try_from(height - header).unwrap_or(0) / row_height).max(1);
    let first = first_row.min(total.saturating_sub(1));
    let last = (first + rows_in_sight).min(total);
    let text = format!("{}\u{2013}{}", number(host, first + 1), number(host, last));
    if bubble.text_content().unwrap_or_default() != text {
        bubble.set_text_content(Some(&text));
    }
    // Beside the scroll bar, where its thumb is.
    let reach = (viewport.scroll_height() - viewport.client_height()).max(1);
    let share = f64::from(viewport.scroll_top().clamp(0, reach)) / f64::from(reach);
    let room = (height - header - bubble.offset_height() - 16).max(0);
    let top = y + header + 8 + (share * f64::from(room)).round() as i32;
    let rtl = viewport.matches(":dir(rtl)").unwrap_or(false);
    let left = if rtl {
        x + 12
    } else {
        x + width - bubble.offset_width() - 12
    };
    let _ = bubble
        .style()
        .set_property("transform", &format!("translate({left}px, {top}px)"));
    set_shown(&bubble, true);
    // Where no `scrollend` comes, the gesture ends when the events stop.
    let host = host.clone();
    let callback = Closure::once_into_js(move || {
        let ended = runtime.borrow().orientation.scroll_round == round;
        if ended {
            hide_bubble(&host);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            callback.unchecked_ref(),
            BUBBLE_IDLE_MS,
        );
    }
}

fn hide_bubble(host: &HtmlElement) {
    if let Some(root) = host.shadow_root()
        && let Some(bubble) = overlay(&root, "scroll-position")
    {
        set_shown(&bubble, false);
    }
    if let Some(runtime) = runtime(host) {
        runtime.borrow_mut().orientation.gesture = 0.0;
    }
}

/// `scrollend` of the viewport: the gesture is over, and the bubble goes.
pub(super) fn on_scroll_end(event: Event) {
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return;
    };
    if !target
        .get_attribute("part")
        .is_some_and(|part| part.split_whitespace().any(|name| name == "viewport"))
    {
        return;
    }
    if let Some(host) = target
        .get_root_node()
        .dyn_into::<ShadowRoot>()
        .ok()
        .and_then(|root| root.host().dyn_into::<HtmlElement>().ok())
    {
        hide_bubble(&host);
    }
}
