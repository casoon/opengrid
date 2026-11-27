//! Presets (issue #532): the page's named views, a segmented control in the
//! toolbar — each a filter the page knows, one of them always chosen.
//!
//! The chosen preset is and-ed onto the page's `filter` and the reader's
//! filters in every query ([`effective_filter`]), it is in the view as
//! `preset`, and it is not a chip: "Remove all" takes back what the reader
//! added, not the view they are looking at. The control is a `radiogroup` —
//! one tab stop, the arrows move and choose — drawn into the toolbar by
//! [`draw_presets`], in place, so a scroll frame never takes the focus off it.

use super::*;

use opengrid_grid::presets as model;

/// Where `set_presets` keeps what the page gave while the grid is not
/// connected: it is checked when the grid is built.
const KEY: &str = "__opengridPresets";
const OPTIONS_KEY: &str = "__opengridPresetOptions";

/// What the grid knows of the counts: asked for, and what came back.
#[derive(Default)]
pub(super) struct Counts {
    /// `counts: true` of the last `set_presets`.
    pub(super) asked: bool,
    /// Rows per preset id, as the last round counted them.
    found: std::collections::BTreeMap<String, u64>,
    /// Queries that round took — what the counts cost (F4).
    pub(super) queries: usize,
    /// Bumped per round; an older round's answers are dropped.
    generation: u64,
}

/// What the page gave, as presets: nothing is none.
fn read(value: &JsValue) -> Result<Vec<model::Preset>, Vec<String>> {
    if value.is_undefined() || value.is_null() {
        return Ok(Vec::new());
    }
    let text = js_sys::JSON::stringify(value)
        .map(String::from)
        .map_err(|_| vec!["presets cannot be read as JSON".to_owned()])?;
    let json = opengrid_json::Json::parse(&text)
        .map_err(|error| vec![format!("presets is not a list: {error}")])?;
    model::parse(&json)
}

/// `counts: true` of the options; nothing else asks for them.
fn counts_asked(options: &JsValue) -> bool {
    js_sys::Reflect::get(options, &JsValue::from_str("counts"))
        .ok()
        .and_then(|counts| counts.as_bool())
        .unwrap_or(false)
}

/// [`crate::element::set_presets`] for a grid: takes the list and checks it,
/// like `set_columns` does — a list that is refused is applied not at all, and
/// said in the status line.
pub(crate) fn set_presets(host: &HtmlElement, value: &JsValue, options: &JsValue) {
    let Some(runtime) = runtime(host) else {
        // Not connected yet: kept as given, taken when the grid is built.
        let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(KEY), value);
        let _ = js_sys::Reflect::set(host.as_ref(), &JsValue::from_str(OPTIONS_KEY), options);
        return;
    };
    let Some(change) = install(host, &runtime, value, options) else {
        return;
    };
    if change.rows {
        rows_changed(host, &runtime);
    } else {
        render(host, false);
        refresh_presets(host);
    }
    if change.view {
        dispatch_view(host);
    }
}

/// Takes what `set_presets` was given before the grid was connected. The first
/// query follows, so nothing is asked here; a list that is refused is said
/// with its result.
pub(super) fn adopt(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let get = |key: &str| js_sys::Reflect::get(host.as_ref(), &JsValue::from_str(key)).ok();
    let Some(value) = get(KEY).filter(|value| !value.is_undefined()) else {
        return;
    };
    let options = get(OPTIONS_KEY).unwrap_or(JsValue::UNDEFINED);
    if let Err(problems) = read(&value) {
        runtime.borrow_mut().state.set_notice(problems.join(" "));
        return;
    }
    install(host, runtime, &value, &options);
}

/// What a new list did.
struct Change {
    /// Other rows are shown now: the chosen preset's filter is another.
    rows: bool,
    /// The reader's choice is another: the view changed.
    view: bool,
}

/// Checks and takes a list; `None`, and said, where it is refused.
fn install(
    host: &HtmlElement,
    runtime: &Rc<RefCell<GridRuntime>>,
    value: &JsValue,
    options: &JsValue,
) -> Option<Change> {
    let items = match read(value) {
        Ok(items) => items,
        Err(problems) => {
            runtime
                .borrow_mut()
                .state
                .set_status(GridStatus::Error(problems.join(" ")));
            render(host, false);
            return None;
        }
    };
    let mut borrowed = runtime.borrow_mut();
    let (filter, chosen) = (
        borrowed.presets.filter(),
        borrowed.presets.chosen().map(str::to_owned),
    );
    borrowed.presets.replace(items);
    borrowed.preset_counts = Counts {
        asked: counts_asked(options),
        generation: borrowed.preset_counts.generation,
        ..Default::default()
    };
    Some(Change {
        rows: filter != borrowed.presets.filter(),
        // A choice the page's list took away is a change; the first one it
        // sets is the page's own start, not the reader's choice.
        view: chosen.is_some() && chosen.as_deref() != borrowed.presets.chosen(),
    })
}

/// Other rows are shown: the first window, what is selected by position gone
/// (by key it stays), the facets' values listed anew — the rule of every filter
/// change (point 35).
fn rows_changed(host: &HtmlElement, runtime: &Rc<RefCell<GridRuntime>>) {
    let pool = pool_of(host);
    {
        let mut borrowed = runtime.borrow_mut();
        borrowed.page = 0;
        borrowed.state.set_window(Window::new(0, pool));
        borrowed.state.invalidate_selection();
        // What a facet lists is the rows of the page *and* the preset.
        borrowed.facet_domain.clear();
        borrowed.facet_counts.clear();
    }
    announce_if_cleared(host, runtime);
    run_query(host, QueryKind::Data, false);
}

/// The reader chose `id`: the rows change, the page hears the view change, and
/// the result is said once with the preset's name.
fn choose(host: &HtmlElement, id: &str) {
    let Some(runtime) = runtime(host) else {
        return;
    };
    let label = {
        let mut borrowed = runtime.borrow_mut();
        if borrowed.presets.chosen() == Some(id) || !borrowed.presets.choose(id) {
            return;
        }
        borrowed
            .presets
            .active()
            .map(|preset| preset.label.clone())
            .unwrap_or_default()
    };
    // With the result, not before: the "loading" frame would say it too.
    runtime
        .borrow_mut()
        .state
        .notice_with_result(texts(host).preset_chosen(&label));
    rows_changed(host, &runtime);
    dispatch_view(host);
}

/// The sentence of the empty state, where the chosen preset has a filter: the
/// rows are none because of it, among others.
pub(super) fn empty_text(host: &HtmlElement, runtime: &GridRuntime) -> Option<String> {
    let preset = runtime
        .presets
        .active()
        .filter(|preset| preset.filter.is_some())?;
    Some(texts(host).empty_preset(&preset.label))
}

/// The label of the chosen preset, for the print table's caption.
pub(super) fn chosen_label(runtime: &GridRuntime) -> Option<String> {
    runtime.presets.active().map(|preset| preset.label.clone())
}

/// Takes the choice of a view: the rows are the preset's, the facets list
/// theirs, and where it changed it is said with the result — unless the page
/// said why it set the view.
pub(super) fn restore(
    host: &HtmlElement,
    runtime: &mut GridRuntime,
    chosen: Option<String>,
    said: bool,
) {
    if runtime.presets.chosen() == chosen.as_deref() {
        return;
    }
    runtime.presets.restore(chosen);
    runtime.facet_domain.clear();
    runtime.facet_counts.clear();
    if !said && let Some(preset) = runtime.presets.active() {
        runtime
            .state
            .notice_with_result(texts(host).preset_chosen(&preset.label));
    }
}

/// A click on a segment.
pub(super) fn on_click(host: &HtmlElement, button: &Element) {
    if let Some(id) = button.get_attribute("data-preset") {
        choose(host, &id);
    }
    // A click on a button focuses it everywhere but Safari: the segment is
    // where the reader is, as it is after a key.
    if let Ok(button) = button.clone().dyn_into::<HtmlElement>() {
        let _ = button.focus();
    }
}

/// The keys of the radiogroup: the arrows move to the next or the previous
/// segment and choose it (wrapping), `Home` and `End` to the first and the last.
/// `Tab` and everything else is the browser's — the group is one tab stop.
pub(super) fn on_key(host: &HtmlElement, event: &KeyboardEvent, target: &Element) {
    if event.ctrl_key() || event.alt_key() || event.meta_key() {
        return;
    }
    let Some(runtime) = runtime(host) else {
        return;
    };
    // Along the line the segments stand in: `ArrowRight` is the next one only
    // where the text runs to the right.
    let rtl = target.matches(":dir(rtl)").unwrap_or(false);
    let ids: Vec<String> = runtime
        .borrow()
        .presets
        .items()
        .iter()
        .map(|preset| preset.id.clone())
        .collect();
    let Some(here) = target
        .closest("[part=\"preset\"]")
        .ok()
        .flatten()
        .and_then(|button| button.get_attribute("data-preset"))
        .and_then(|id| ids.iter().position(|other| *other == id))
    else {
        return;
    };
    let last = ids.len() - 1;
    let next = match event.key().as_str() {
        "ArrowDown" => (here + 1) % ids.len(),
        "ArrowUp" => (here + last) % ids.len(),
        "ArrowRight" if rtl => (here + last) % ids.len(),
        "ArrowRight" => (here + 1) % ids.len(),
        "ArrowLeft" if rtl => (here + 1) % ids.len(),
        "ArrowLeft" => (here + last) % ids.len(),
        "Home" => 0,
        "End" => last,
        _ => return,
    };
    event.prevent_default();
    choose(host, &ids[next]);
    if let Some(root) = host.shadow_root()
        && let Ok(Some(button)) = root.query_selector(&format!(
            "[part=\"preset\"][data-preset=\"{}\"]",
            ids[next].replace('\\', "\\\\").replace('"', "\\\"")
        ))
        && let Ok(button) = button.dyn_into::<HtmlElement>()
    {
        let _ = button.focus();
    }
}

/// Draws the control into the toolbar — the radiogroup, one segment per preset,
/// and the cost line where the counts are asked for — and keeps it in line with
/// the state. Runs after every render, so the segments are rebuilt only when the
/// list changed, and the rest is written in place: a rebuild would take the
/// focus off the segment the reader just chose.
pub(super) fn draw_presets(host: &HtmlElement, root: &ShadowRoot) {
    let Ok(Some(toolbar)) = root.query_selector("[part=\"toolbar\"]") else {
        return;
    };
    let Some(runtime) = runtime(host) else {
        return;
    };
    let (items, chosen, counts, found, queries) = {
        let borrowed = runtime.borrow();
        (
            borrowed
                .presets
                .items()
                .iter()
                .map(|preset| (preset.id.clone(), preset.label.clone()))
                .collect::<Vec<_>>(),
            borrowed.presets.chosen().map(str::to_owned),
            borrowed.preset_counts.asked,
            borrowed.preset_counts.found.clone(),
            borrowed.preset_counts.queries,
        )
    };
    let group = toolbar.query_selector("[part=\"presets\"]").ok().flatten();
    let cost = toolbar
        .query_selector("[part=\"presets-cost\"]")
        .ok()
        .flatten();
    if items.is_empty() {
        for node in group.into_iter().chain(cost) {
            node.remove();
        }
        return;
    }
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let texts = texts(host);
    let language = |element: &Element| {
        if !texts.lang.trim().is_empty() {
            let _ = element.set_attribute("lang", &texts.lang);
        }
    };

    let group = match group {
        Some(group) => group,
        None => {
            let Ok(group) = document.create_element("div") else {
                return;
            };
            let _ = group.set_attribute("part", "presets");
            let _ = group.set_attribute("role", "radiogroup");
            let _ = group.set_attribute("aria-label", &texts.presets_group);
            language(&group);
            // First: the views are what the reader meets before anything else.
            let _ = toolbar.insert_before(&group, toolbar.first_child().as_ref());
            group
        }
    };
    let signature = items
        .iter()
        .map(|(id, label)| format!("{id}={label}"))
        .collect::<Vec<_>>()
        .join("\u{1F}");
    if group.get_attribute("data-drawn").as_deref() != Some(signature.as_str()) {
        let _ = group.set_attribute("data-drawn", &signature);
        group.set_inner_html("");
        for (id, label) in &items {
            let Ok(button) = document.create_element("button") else {
                continue;
            };
            for (name, value) in [
                ("type", "button"),
                ("part", "preset"),
                ("role", "radio"),
                ("data-preset", id.as_str()),
            ] {
                let _ = button.set_attribute(name, value);
            }
            button.set_text_content(Some(label));
            language(&button);
            let _ = group.append_child(&button);
        }
    }
    // The state, in place: which segment is on, the one tab stop, the counts.
    // Nothing chosen (an id the page's list has not) leaves the first the stop.
    let known = items.iter().any(|(id, _)| Some(id) == chosen.as_ref());
    if let Ok(buttons) = group.query_selector_all("[part=\"preset\"]") {
        for index in 0..buttons.length() {
            let Some(button) = buttons
                .item(index)
                .and_then(|node| node.dyn_into::<Element>().ok())
            else {
                continue;
            };
            let Some((id, label)) = items.get(index as usize) else {
                continue;
            };
            let on = Some(id) == chosen.as_ref();
            let _ = button.set_attribute("aria-checked", &on.to_string());
            let stop = on || (!known && index == 0);
            let _ = button.set_attribute("tabindex", if stop { "0" } else { "-1" });
            let text = match (counts, found.get(id)) {
                (true, Some(count)) => texts.preset_count(label, *count),
                _ => label.to_string(),
            };
            if button.text_content().as_deref() != Some(text.as_str()) {
                button.set_text_content(Some(&text));
            }
        }
    }

    // What the counts cost, said as the facets say it (F4).
    if counts && queries > 0 {
        let cost = match cost {
            Some(cost) => cost,
            None => {
                let Ok(cost) = document.create_element("span") else {
                    return;
                };
                let _ = cost.set_attribute("part", "presets-cost");
                language(&cost);
                let _ = toolbar.insert_before(&cost, group.next_sibling().as_ref());
                cost
            }
        };
        let text = texts.facet_queries(queries);
        if cost.text_content().as_deref() != Some(text.as_str()) {
            cost.set_text_content(Some(&text));
        }
    } else if let Some(cost) = cost {
        cost.remove();
    }
}

/// Counts the rows of every preset, where the page asked for it
/// (`counts: true`): one query per preset, under the page's filter, the
/// reader's and the preset's own — the rows the segment shows when it is
/// chosen. Asked with every query for the rows, as the facets' counts are
/// (F4), and said in the same way.
pub(super) fn refresh_presets(host: &HtmlElement) {
    let Some(grid_runtime) = runtime(host) else {
        return;
    };
    let (Some(provider), Some(source)) = (provider(host), host.get_attribute(DATASOURCE_ATTRIBUTE))
    else {
        return;
    };
    let mode = host.get_attribute(MODE_ATTRIBUTE).unwrap_or_default();
    let (generation, plan) = {
        let mut runtime = grid_runtime.borrow_mut();
        if !runtime.preset_counts.asked || runtime.presets.items().is_empty() {
            runtime.preset_counts.found.clear();
            runtime.preset_counts.queries = 0;
            return;
        }
        runtime.preset_counts.generation += 1;
        let (Ok(page), Ok(reader)) = (page_filter(host), reader_filter(host, &runtime, None))
        else {
            return;
        };
        let plan: Vec<(String, Option<FilterExpr>)> = runtime
            .presets
            .items()
            .iter()
            .map(|preset| {
                (
                    preset.id.clone(),
                    model::all_of([page.clone(), preset.filter.clone(), reader.clone()]),
                )
            })
            .collect();
        (runtime.preset_counts.generation, plan)
    };

    let host = host.clone();
    spawn_local(async move {
        let mut queries = 0usize;
        for (id, filter) in plan {
            let query = grouping::total_query_json(&source, filter.as_ref(), &[]);
            queries += 1;
            let Ok(result) = ask_uncounted(&host, &provider, &query, &mode).await else {
                continue;
            };
            if grid_runtime.borrow().preset_counts.generation != generation {
                return;
            }
            let count = match result.columns.first().and_then(|column| column.first()) {
                Some(Value::Int64(count)) => (*count).max(0) as u64,
                _ => continue,
            };
            grid_runtime
                .borrow_mut()
                .preset_counts
                .found
                .insert(id, count);
            if let Some(root) = host.shadow_root() {
                draw_presets(&host, &root);
            }
        }
        if grid_runtime.borrow().preset_counts.generation != generation {
            return;
        }
        grid_runtime.borrow_mut().preset_counts.queries = queries;
        if let Some(root) = host.shadow_root() {
            draw_presets(&host, &root);
        }
        show_source(&host);
    });
}
