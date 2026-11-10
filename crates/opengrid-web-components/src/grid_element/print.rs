//! The print path (issue #334): every match as a plain table, for paper and
//! for the PDF a browser makes of it.
//!
//! The grid on the screen holds a window of rows inside a scroller, so it
//! cannot be printed by a stylesheet alone. `prepare_print` asks for every
//! match of the view — the query of [`current_query`], what an export sends —
//! and builds a native `<table>` beside the grid: a caption that says what is
//! printed, a `<thead>` the browser repeats on every page, one row per match,
//! the values through the page's formats. Only `@media print` shows it, and
//! hides the grid; the screen is not touched, so the window, the focus and
//! the scroll position are what they were. `afterprint` takes it away.
//!
//! The table stands in a shadow root of its own: the grid's sheet positions
//! every `tr` absolutely and cuts every cell to one line, and a print table
//! wants neither.

use super::*;

use web_sys::{ShadowRootInit, ShadowRootMode};

/// The most rows a print table holds — one request at a server's `max_limit`.
/// More matches are refused before anything is built: paper that silently
/// stops is worse than no paper.
const MAX_ROWS: u64 = 10_000;

/// Marks the holder of the print table in the grid's shadow root.
const HOLDER: &str = "data-print";

/// What the grid's shadow root gets while a print table is there: the table
/// only on paper, and on paper nothing else. The host gives up the height the
/// page set for the scroller — inner `!important` beats the page's rule — or
/// the table would run over what follows it. The grid is folded to no height
/// rather than taken out with `display: none`: an element that is not
/// rendered loses the focus, and the reader would come back from the print
/// dialog to a grid without one.
const GRID_CSS: &str = "\
[data-print] { display: none; }
@media print {
  :host { block-size: auto !important; max-block-size: none !important; overflow: visible !important; }
  [data-print] { display: block; }
  [data-print] ~ * { block-size: 0 !important; min-block-size: 0 !important; margin: 0 !important;
    border: 0 !important; padding: 0 !important; overflow: hidden !important; }
  :popover-open { display: none !important; }
}";

/// The print table's own sheet: black on white whatever the look, lines
/// instead of grounds, no row split over two pages.
const TABLE_CSS: &str = "\
table { inline-size: 100%; border-collapse: collapse; color: #000; background: #fff; }
caption { padding-block-end: 0.5em; text-align: start; font-weight: 600; }
th, td { padding: 2px 6px; border: 1px solid #000; text-align: start; vertical-align: top; overflow-wrap: anywhere; }
tr { break-inside: avoid; }
[data-level=\"2\"] { padding-inline-start: 1.5em; }
[data-level=\"3\"] { padding-inline-start: 3em; }
[data-align=\"end\"] { text-align: end; font-variant-numeric: tabular-nums; }
[data-align=\"center\"] { text-align: center; }";

thread_local! {
    static SHEETS: Option<(web_sys::CssStyleSheet, web_sys::CssStyleSheet)> = {
        let sheet = |css: &str| {
            let sheet = web_sys::CssStyleSheet::new().ok()?;
            sheet.replace_sync(css).ok()?;
            Some(sheet)
        };
        sheet(GRID_CSS).zip(sheet(TABLE_CSS))
    };
}

/// [`crate::element::prepare_print`]: a promise of the number of rows in the
/// print table, or of `null` when the grid has no query to print.
pub(crate) fn prepare(host: &HtmlElement) -> js_sys::Promise {
    let host = host.clone();
    wasm_bindgen_futures::future_to_promise(async move {
        discard(&host);
        let (Some(provider), Some(query), Some(runtime)) =
            (provider(&host), current_query(&host), runtime(&host))
        else {
            return Ok(JsValue::NULL);
        };
        let Ok(mut query) = opengrid_json::Json::parse(&query) else {
            return Ok(JsValue::NULL);
        };
        query["offset"] = opengrid_json::json!(0);
        query["limit"] = opengrid_json::json!(MAX_ROWS);
        let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
        let result = ask_uncounted(&host, &provider, &query.to_string(), &mode)
            .await
            .map_err(|message| JsValue::from(js_sys::Error::new(&message)))?;
        let rows = result.row_count() as u64;
        if result.total_count > rows {
            let error = js_sys::Error::new(&format!(
                "prepare_print: {} rows, more than the {MAX_ROWS} a print table holds — narrow the view, or export it",
                result.total_count
            ));
            let _ = js_sys::Reflect::set(&error, &"code".into(), &"too_many_rows".into());
            return Err(error.into());
        }
        // The grouping as it is now: the rows came ordered by it.
        let groups = match &*runtime.borrow() {
            GridRuntime { tree: Some(_), .. } => Vec::new(),
            GridRuntime { grouping, .. } => grouping
                .as_ref()
                .map(|grouping| grouping.by().to_vec())
                .unwrap_or_default(),
        };
        build(&host, &result, &groups);
        Ok(JsValue::from_f64(rows as f64))
    })
}

/// Takes the print table away, and its sheet off the grid.
fn discard(host: &HtmlElement) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    if let Ok(Some(holder)) = root.query_selector(&format!("[{HOLDER}]")) {
        holder.remove();
        // The look, the widths and the pins, as `adopt_sheets` put them.
        root.set_adopted_style_sheets(&root.adopted_style_sheets().slice(0, 3));
    }
}

/// Builds the table and puts it first in the grid's shadow root, where the
/// sheet's `~` reaches everything after it.
fn build(host: &HtmlElement, result: &opengrid_datasource::QueryResult, groups: &[String]) {
    let (Some(root), Some(document)) = (host.shadow_root(), host.owner_document()) else {
        return;
    };
    let node = |parent: &Node, tag: &str, text: Option<&str>| {
        let element = document.create_element(tag).ok()?;
        element.set_text_content(text);
        parent.append_child(&element).ok()?;
        Some(element)
    };
    let texts = texts(host);
    let fields = result.schema.fields();
    let format = Formatter::new(&formats(host), &result.schema);
    let styles = presentation::styles(host);
    let aligns: Vec<Option<String>> = fields
        .iter()
        .map(|field| {
            styles
                .markers(field.name.as_str(), field.data_type)
                .into_iter()
                .find_map(|(name, value)| (name == "data-align").then_some(value))
        })
        .collect();
    let align = |cell: &Element, col: usize| {
        if let Some(Some(value)) = aligns.get(col) {
            let _ = cell.set_attribute("data-align", value);
        }
    };

    let Ok(holder) = document.create_element("div") else {
        return;
    };
    let _ = holder.set_attribute(HOLDER, "");
    let Ok(inner) = holder.attach_shadow(&ShadowRootInit::new(ShadowRootMode::Open)) else {
        return;
    };
    let Some(table) = node(&inner, "table", None) else {
        return;
    };

    // What the paper holds: the grid's name, how much, and under which view —
    // the chips' words, so the filter and the grouping read as on the screen.
    let tree = result.tree.as_ref();
    let count = tree.map_or(result.total_count, |tree| tree.matches);
    let mut caption: Vec<String> = host.get_attribute(LABEL_ATTRIBUTE).into_iter().collect();
    caption.push(if count == 0 {
        texts.empty.clone()
    } else {
        texts.matches(count)
    });
    caption.extend(chips_of(host).into_iter().map(|chip| chip.text));
    node(&table, "caption", Some(&caption.join(" \u{b7} ")));

    let header = node(&table, "thead", None).and_then(|thead| node(&thead, "tr", None));
    for (col, field) in fields.iter().enumerate() {
        let Some(th) = header
            .as_ref()
            .and_then(|row| node(row, "th", Some(texts.column(field.name.as_str()))))
        else {
            return;
        };
        let _ = th.set_attribute("scope", "col");
        align(&th, col);
    }

    // The columns the rows are grouped by, where the grid shows them.
    let levels: Vec<usize> = groups
        .iter()
        .filter_map(|name| fields.iter().position(|field| field.name.as_str() == name))
        .collect();
    let same = |a: usize, b: usize, depth: usize| {
        levels[..=depth]
            .iter()
            .all(|col| result.columns[*col][a] == result.columns[*col][b])
    };
    let flat = tree.and_then(|tree| tree.flat.as_ref());
    let rows = result.row_count();
    let mut body = node(&table, "tbody", None);
    for row in 0..rows {
        // A group opens a row group of its own, under a header that names it:
        // every group open, each with its count (the summaries are not printed).
        for depth in 0..levels.len() {
            if row > 0 && same(row, row - 1, depth) {
                continue;
            }
            let size = (row..rows)
                .take_while(|next| same(row, *next, depth))
                .count();
            let name = fields[levels[depth]].name.as_str();
            let value = grid::group_value_text(
                &result.columns[levels[depth]][row],
                name,
                fields,
                &texts,
                &format,
            );
            body = node(&table, "tbody", None);
            let th = body
                .as_ref()
                .and_then(|body| node(body, "tr", None))
                .and_then(|tr| node(&tr, "th", Some(&texts.group_row(name, &value, size as u64))));
            if let Some(th) = th {
                let _ = th.set_attribute("scope", "rowgroup");
                let _ = th.set_attribute("colspan", &fields.len().to_string());
                let _ = th.set_attribute("data-level", &(depth + 1).to_string());
            }
        }
        let Some(tr) = body.as_ref().and_then(|body| node(body, "tr", None)) else {
            return;
        };
        for (col, values) in result.columns.iter().enumerate() {
            let mut text = format.text(col, &values[row]);
            // A tree is printed whole and flat, every node open: the depth
            // indents the first column, and a node that is only the way to a
            // match says so in words — paper has no muted ink to rely on.
            if let (0, Some(flat)) = (col, flat) {
                let depth = flat.levels[row].saturating_sub(1) as usize;
                text.insert_str(0, &"\u{2003}".repeat(depth));
                if flat.filtered && tree.is_some_and(|tree| !tree.matched[row]) {
                    text = format!("{text} ({})", texts.tree_context);
                }
            }
            if let Some(td) = node(&tr, "td", Some(&text)) {
                align(&td, col);
            }
        }
    }

    SHEETS.with(|sheets| {
        let Some((grid_sheet, table_sheet)) = sheets else {
            return;
        };
        inner.set_adopted_style_sheets(&js_sys::Array::of1(table_sheet));
        let adopted = root.adopted_style_sheets();
        adopted.push(grid_sheet);
        root.set_adopted_style_sheets(&adopted);
    });
    let _ = root.insert_before(&holder, root.first_child().as_ref());

    // Gone after the print, and the listener with it.
    let Some(window) = web_sys::window() else {
        return;
    };
    let host = host.clone();
    let after = Closure::once_into_js(move || discard(&host));
    let options = web_sys::AddEventListenerOptions::new();
    options.set_once(true);
    let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
        "afterprint",
        after.unchecked_ref(),
        &options,
    );
}
