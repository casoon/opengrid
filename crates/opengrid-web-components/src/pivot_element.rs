//! `<opengrid-pivot>` — the browser glue (plan point 32).
//!
//! The same lifecycle as the table: an open shadow root, a whole render per
//! answer, and the engine never named — only the provider's JSON-in/JSON-out
//! Promise. What differs is what travels: a pivot request, and the pivot wire
//! form of point 53 coming back.
//!
//! There is no interaction to preserve here, so a failure simply becomes the
//! status line's text. That is the point of Table Mode: nothing to lose.
//!
//! The answer the element shows is kept beside it, so that `get_pivot` exports
//! exactly that (issue #3).
//!
//! The view (issue #106) is the `rows`, `columns` and `values` attributes:
//! `set_view` writes them at once and runs one query, and every change of them
//! is reported as `opengrid-view-change`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::JsError;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Element, Event, HtmlElement};

use opengrid_web_core::element::{LABEL_ATTRIBUTE, attach_open_shadow_root, define};
use opengrid_web_core::host::{existing_id, id as host_id, on_release};
use opengrid_web_core::patch::{NodeAllocator, PatchBuffer};
use opengrid_web_core::provider::provider;

use crate::element::{
    answer_size, apply, clear_root, describe, dispatch, dispatch_query, is_latest, next_request,
    now,
};
use crate::grid_element_events::VIEW_EVENT;
use crate::pivot::{
    self, Axis, COLLAPSED_ATTRIBUTE, COLUMNS_ATTRIBUTE, DATASOURCE_ATTRIBUTE, FIELDS_ATTRIBUTE,
    FILTER_ATTRIBUTE, FieldAction, MEASURES_ATTRIBUTE, Offer, PIVOT_TAG, PivotLook, PivotModel,
    PivotView, PlainLook, ROWS_ATTRIBUTE, SORT_ATTRIBUTE, SortTarget, TOOLBAR_ATTRIBUTE,
    VALUES_ATTRIBUTE, VIEW_ATTRIBUTES,
};
use crate::texts;

/// Registers `<opengrid-pivot>`.
pub fn define_pivot() -> Result<(), JsValue> {
    define(
        PIVOT_TAG,
        pivot::OBSERVED,
        on_connected,
        |_host| {},
        on_attribute_changed,
    )
}

fn on_connected(host: HtmlElement) {
    if let Ok(root) = attach_open_shadow_root(&host)
        && root.child_element_count() == 0
    {
        crate::theme::adopt_table_look(&root);
        render(&host, None, "", "loading", &PlainLook);
        let callback = Closure::<dyn FnMut(Event)>::new(on_header_click).into_js_value();
        let _ = root.add_event_listener_with_callback("click", callback.unchecked_ref());
        let keys = Closure::<dyn FnMut(Event)>::new(on_menu_key).into_js_value();
        let _ = root.add_event_listener_with_callback("keydown", keys.unchecked_ref());
    }
    // The view it starts with is not a change.
    report_view(&host, false);
    run(&host);
}

fn on_attribute_changed(
    host: HtmlElement,
    name: String,
    _old: Option<String>,
    _new: Option<String>,
) {
    match name.as_str() {
        LABEL_ATTRIBUTE | DATASOURCE_ATTRIBUTE => run(&host),
        // What the toolbar shows and offers (issue #112).
        TOOLBAR_ATTRIBUTE | FIELDS_ATTRIBUTE | MEASURES_ATTRIBUTE => run(&host),
        // Display only: drawn again from what the pivot holds, nothing asked.
        COLLAPSED_ATTRIBUTE => {
            if APPLYING.get() {
                return;
            }
            redraw(&host);
            report_view(&host, true);
        }
        ROWS_ATTRIBUTE | COLUMNS_ATTRIBUTE | VALUES_ATTRIBUTE | SORT_ATTRIBUTE
        | FILTER_ATTRIBUTE => {
            // `write_view` sets all three and owns the one query after.
            if APPLYING.get() {
                return;
            }
            run(&host);
            report_view(&host, true);
        }
        _ => {}
    }
}

/// Builds the request from the host attributes and runs it.
///
/// Missing attributes are "not ready yet", not errors — except the measures: an
/// empty or unreadable `values` is a mistake the page can fix, so it is shown.
pub(crate) fn run(host: &HtmlElement) {
    let Some(provider) = provider(host) else {
        return;
    };
    if host.shadow_root().is_none() {
        return;
    }
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let rows = pivot::parse_dimensions(host.get_attribute(ROWS_ATTRIBUTE).as_deref());
    let columns = pivot::parse_dimensions(host.get_attribute(COLUMNS_ATTRIBUTE).as_deref());

    let filter = pivot::parse_filter(host.get_attribute(FILTER_ATTRIBUTE).as_deref());
    let request_json = match offer(host)
        .and(pivot::parse_sort(
            host.get_attribute(SORT_ATTRIBUTE).as_deref(),
        ))
        .and_then(|sort| Ok((sort, filter?)))
        .and_then(|(sort, filter)| {
            pivot::pivot_json(
                &source,
                &rows,
                &columns,
                host.get_attribute(VALUES_ATTRIBUTE).as_deref(),
                &sort,
                filter.as_ref(),
            )
            .map(|json| (json, sort))
        }) {
        Ok(request) => request,
        Err(message) => {
            // A request that cannot be built supersedes an earlier one as
            // well: its error is what the element shows, not an answer that
            // arrives after it.
            next_request(host);
            let texts = texts::texts(host);
            render(host, None, &texts.error(&message), "error", &PlainLook);
            return;
        }
    };

    let (request_json, sort) = request_json;
    let request = next_request(host);
    let texts = texts::texts(host);
    render(host, None, &texts.loading, "loading", &PlainLook);

    let kind = provider.kind();
    let started = now();
    let promise = provider.pivot(&request_json, "");
    let host = host.clone();
    spawn_local(async move {
        let outcome = JsFuture::from(promise).await;
        let ms = now() - started;
        // A newer request was made since (an attribute changed again): its
        // answer is the one to draw, whenever it comes.
        if !is_latest(&host, request) {
            return;
        }
        let texts = texts::texts(&host);
        match outcome {
            Ok(value) => match pivot_json(&value) {
                Ok(json) => match pivot::parse_result(&json) {
                    Ok(mut model) => {
                        model.column_dimensions = columns;
                        model.sort = sort;
                        // Measured like the grid's queries (issue #70); a
                        // pivot's rows are all it answers, subtotals included.
                        let (bytes, form) = answer_size(&value);
                        let rows = model.rows.len() as u64;
                        dispatch_query(&host, kind, ms, rows, rows, bytes, form);
                        // The grand total is always a row, so "no matches" means
                        // nothing but the total came back.
                        let (status, state) = if model.rows.len() <= 1 {
                            (texts.empty.clone(), "empty")
                        } else {
                            (texts.matches(model.rows.len() as u64), "ready")
                        };
                        match HostLook::new(&host) {
                            Ok(look) => {
                                render(&host, Some((&model, &json)), &status, state, &look);
                                refocus(&host);
                            }
                            Err(message) => {
                                render(&host, None, &texts.error(&message), "error", &PlainLook)
                            }
                        }
                    }
                    Err(message) => {
                        render(&host, None, &texts.error(&message), "error", &PlainLook)
                    }
                },
                Err(message) => render(&host, None, &texts.error(&message), "error", &PlainLook),
            },
            Err(value) => render(
                &host,
                None,
                &texts.error(&describe(&value)),
                "error",
                &PlainLook,
            ),
        }
        // An answer that is an error draws the toolbar too: the reader who
        // pressed something there gets the focus back all the same.
        refocus(&host);
    });
}

/// A provider's pivot answer as the pivot wire form, whichever form it came in.
///
/// The binary form (E35) is read strictly and written back as JSON: the pivot
/// renders and exports from that one form, so the headings read the same
/// whichever way the answer travelled. A pivot's size is bounded by its column
/// limit, so the detour is small.
fn pivot_json(value: &JsValue) -> Result<String, String> {
    match crate::element::answer(value) {
        Some(crate::element::Answer::Json(json)) => Ok(json),
        Some(crate::element::Answer::Binary(bytes)) => {
            let (result, rows) =
                opengrid_pivot::pivot_from_bytes(&bytes).map_err(|error| error.to_string())?;
            Ok(opengrid_pivot::pivot_to_json(&result, &rows))
        }
        None => Err(crate::element::NOT_AN_ANSWER.to_owned()),
    }
}

/// Clears the root and renders the whole pivot as one patch list: the model,
/// and the answer it was read from, which is what `get_pivot` exports.
fn render(
    host: &HtmlElement,
    shown: Option<(&PivotModel, &str)>,
    status: &str,
    state: &str,
    look: &dyn PivotLook,
) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    clear_root(&root);
    // Kept and dropped with what is drawn, never apart from it: while a new
    // answer loads, or after an error, the table is empty and so is the export.
    remember(
        host,
        shown.map(|(model, answer)| (model, answer, status, state)),
    );
    // The folded groups are the attribute's, whenever it is drawn (#110).
    let folded = shown.map(|(model, _)| PivotModel {
        collapsed: pivot::parse_collapsed(host.get_attribute(COLLAPSED_ATTRIBUTE).as_deref())
            .unwrap_or_default(),
        ..model.clone()
    });
    let model = folded.as_ref();

    let label = host.get_attribute(LABEL_ATTRIBUTE);
    let texts = texts::texts(host);
    let mut nodes = NodeAllocator::new();
    let mut buffer = PatchBuffer::new();
    // The toolbar is drawn in every state — loading, empty, an error — so a
    // reader can always take back the field that caused it.
    let toolbar = host
        .has_attribute(TOOLBAR_ATTRIBUTE)
        .then(|| (view_of(host), offer(host).unwrap_or_default()));
    pivot::build_pivot(
        &mut buffer,
        &mut nodes,
        label.as_deref(),
        model,
        status,
        state,
        &texts,
        look,
        toolbar.as_ref().map(|(view, offer)| (view, offer)),
    );
    apply(&root, document, &buffer);
}

/// What the page offers the reader (issue #112); only read with a toolbar.
fn offer(host: &HtmlElement) -> Result<Offer, String> {
    if !host.has_attribute(TOOLBAR_ATTRIBUTE) {
        return Ok(Offer::default());
    }
    Offer::from_attributes(
        host.get_attribute(FIELDS_ATTRIBUTE).as_deref(),
        host.get_attribute(MEASURES_ATTRIBUTE).as_deref(),
    )
}

/// The page's look for a pivot (issue #104): its `set_formats`, by field or
/// measure name, and — with the grid's `set_columns` — its titles.
struct HostLook {
    formats: Rc<crate::formats::ColumnFormats>,
    #[cfg(feature = "grid")]
    titles: Vec<(String, String)>,
}

impl HostLook {
    /// A pivot takes a `title` and nothing else: an option that silently did
    /// nothing would hide the mistake, as it would at the table. A title for a
    /// name the pivot does not show now is fine — its dimensions change with
    /// its attributes.
    fn new(host: &HtmlElement) -> Result<Self, String> {
        #[cfg(feature = "grid")]
        let titles = {
            let raw = crate::presentation::raw(host);
            let mut problems = Vec::new();
            for (name, entry) in raw.iter() {
                for (given, option) in [
                    (entry.width.is_some(), "width"),
                    (entry.align.is_some(), "align"),
                    (entry.mono.is_some(), "mono"),
                    (entry.emphasis.is_some(), "emphasis"),
                    (entry.muted.is_some(), "muted"),
                    (entry.aggregate.is_some(), "aggregate"),
                    (entry.facet.is_some(), "facet"),
                ] {
                    if given {
                        problems.push(format!("{name}: a pivot takes a title, not {option}"));
                    }
                }
            }
            if !problems.is_empty() {
                return Err(problems.join(" "));
            }
            raw.iter()
                .filter_map(|(name, entry)| entry.title.clone().map(|title| (name.clone(), title)))
                .collect()
        };
        Ok(Self {
            formats: crate::formats::formats(host),
            #[cfg(feature = "grid")]
            titles,
        })
    }
}

impl PivotLook for HostLook {
    fn title(&self, name: &str) -> String {
        #[cfg(feature = "grid")]
        if let Some((_, title)) = self.titles.iter().find(|(field, _)| field == name) {
            return title.clone();
        }
        name.to_owned()
    }

    fn text(&self, name: &str, value: &opengrid_json::Json) -> String {
        let plain = pivot::plain(value).unwrap_or_default();
        let Some(function) = self.formats.function(name) else {
            // No format for this name: no boundary crossing (R1).
            return plain;
        };
        let json = js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL);
        function
            .call2(&JsValue::NULL, &JsValue::from_str(&plain), &json)
            .ok()
            .and_then(|text| text.as_string())
            // A formatter that throws or answers nothing must not blank the
            // cell: the value is still there.
            .unwrap_or(plain)
    }
}

// ---------------------------------------------------------------------------
// The view (issue #106)
// ---------------------------------------------------------------------------

thread_local! {
    /// True while `write_view` sets the attributes: it runs the one query.
    static APPLYING: Cell<bool> = const { Cell::new(false) };
    /// The view each pivot last reported, by host id, as JSON text.
    static REPORTED: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

fn view_of(host: &HtmlElement) -> PivotView {
    PivotView::from_attributes(
        host.get_attribute(ROWS_ATTRIBUTE).as_deref(),
        host.get_attribute(COLUMNS_ATTRIBUTE).as_deref(),
        host.get_attribute(VALUES_ATTRIBUTE).as_deref(),
        host.get_attribute(SORT_ATTRIBUTE).as_deref(),
        host.get_attribute(COLLAPSED_ATTRIBUTE).as_deref(),
        host.get_attribute(FILTER_ATTRIBUTE).as_deref(),
    )
}

/// Remembers the view, and fires `opengrid-view-change` when `announce` and it
/// differs from the one last reported — setting what it has says nothing.
fn report_view(host: &HtmlElement, announce: bool) {
    if host.shadow_root().is_none() {
        // Upgrading: the attributes arrive before the element is connected,
        // and the view they make is the first one, not a change.
        return;
    }
    let view = view_of(host).to_json();
    let text = view.to_string();
    on_release(|id| {
        REPORTED.with(|map| map.borrow_mut().remove(&id));
    });
    let id = host_id(host);
    let changed = REPORTED.with(|map| map.borrow_mut().insert(id, text.clone())) != Some(text);
    if announce && changed {
        let detail = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&detail, &JsValue::from_str("view"), &to_js(&view));
        dispatch(host, VIEW_EVENT, &detail);
    }
}

fn to_js(value: &opengrid_json::Json) -> JsValue {
    js_sys::JSON::parse(&value.to_string()).unwrap_or(JsValue::NULL)
}

/// [`crate::element::get_view`] for a pivot: `null` before it is connected.
pub(crate) fn read_view(host: &HtmlElement) -> JsValue {
    if host.shadow_root().is_none() {
        return JsValue::NULL;
    }
    to_js(&view_of(host).to_json())
}

/// [`crate::element::set_view`] for a pivot: all three attributes, one query.
///
/// A view that does not hold is said in the status line and applied not at
/// all; the pivot shown stays.
pub(crate) fn write_view(host: &HtmlElement, value: &JsValue) {
    if host.shadow_root().is_none() {
        return;
    }
    let text = js_sys::JSON::stringify(value)
        .ok()
        .and_then(|text| text.as_string())
        .unwrap_or_default();
    let view = opengrid_json::from_str(&text)
        .map_err(|_| vec!["the view is not an object".to_owned()])
        .and_then(|value| PivotView::from_json(&value));
    let view = match view {
        Ok(view) => view,
        Err(problems) => {
            say(host, &texts::texts(host).error(&problems.join(" ")));
            return;
        }
    };
    apply_view(host, &view);
}

/// Writes every attribute of `view` at once: one query, one report. Setting
/// what it has costs nothing and says nothing.
fn apply_view(host: &HtmlElement, view: &PivotView) {
    if *view == view_of(host) {
        return;
    }
    APPLYING.set(true);
    for ((name, optional), value) in VIEW_ATTRIBUTES.into_iter().zip(view.attributes()) {
        let _ = if optional && value.is_empty() {
            host.remove_attribute(name)
        } else {
            host.set_attribute(name, &value)
        };
    }
    APPLYING.set(false);
    run(host);
    report_view(host, true);
}

// ---------------------------------------------------------------------------
// Sorting (issue #108)
// ---------------------------------------------------------------------------

thread_local! {
    /// The sort or fold button each pivot's reader pressed, by host id, as its
    /// `(data attribute, value)`: it gets the focus back once the answer is
    /// drawn, since drawing replaces every node.
    static PRESSED: RefCell<HashMap<u32, Vec<(String, String)>>> = RefCell::new(HashMap::new());
}

/// A sort button: its target goes into the `sort` attribute, and the
/// attribute's change runs the query and reports the view.
fn on_header_click(event: Event) {
    let Some(host) = event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::ShadowRoot>().ok())
        .and_then(|root| root.host().dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    let target = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok());
    if let Some(target) = &target
        && (on_filter_click(&host, target) || on_toolbar_click(&host, target))
    {
        return;
    }
    if let Some(toggle) = target.as_ref().and_then(|target| {
        target
            .closest(r#"button[part="group-toggle"]"#)
            .ok()
            .flatten()
    }) {
        on_group_toggle(&host, &toggle);
        return;
    }
    let Some(button) = target.and_then(|target| {
        target
            .closest(r#"button[part="sort-button"]"#)
            .ok()
            .flatten()
    }) else {
        return;
    };
    let (key, value) = if let Some(field) = button.get_attribute("data-sort-field") {
        ("data-sort-field", field)
    } else if let Some(measure) = button.get_attribute("data-sort-by") {
        ("data-sort-by", measure)
    } else {
        return;
    };
    let target = if key == "data-sort-field" {
        SortTarget::Dimension(&value)
    } else {
        SortTarget::Measure(&value)
    };
    let view = view_of(&host);
    let sort = pivot::next_sort(&view.sort, &view.rows, target);
    let next = PivotView { sort, ..view };
    on_release(|id| {
        PRESSED.with(|map| map.borrow_mut().remove(&id));
    });
    let id = host_id(&host);
    PRESSED.with(|map| {
        map.borrow_mut()
            .insert(id, vec![(key.to_owned(), value.clone())])
    });
    let sort = view_attribute(&next, SORT_ATTRIBUTE);
    let _ = if sort.is_empty() {
        host.remove_attribute(SORT_ATTRIBUTE)
    } else {
        host.set_attribute(SORT_ATTRIBUTE, &sort)
    };
}

// ---------------------------------------------------------------------------
// Folding (issue #110)
// ---------------------------------------------------------------------------

thread_local! {
    /// What the status line says after a fold, by host id — the sentence of
    /// the press that caused it, said once when the pivot is drawn again.
    static SAY: RefCell<HashMap<u32, String>> = RefCell::new(HashMap::new());
}

fn shown(host: &HtmlElement) -> Option<Shown> {
    existing_id(host).and_then(|id| SHOWN.with(|map| map.borrow().get(&id).cloned()))
}

/// Draws what the pivot holds again, with the folded groups as they are now.
fn redraw(host: &HtmlElement) {
    let Some(shown) = shown(host) else {
        return;
    };
    let said = existing_id(host).and_then(|id| SAY.with(|map| map.borrow_mut().remove(&id)));
    let status = said.unwrap_or_else(|| shown.status.clone());
    match HostLook::new(host) {
        Ok(look) => render(
            host,
            Some((&shown.model, &shown.answer)),
            &status,
            &shown.state,
            &look,
        ),
        Err(_) => render(
            host,
            Some((&shown.model, &shown.answer)),
            &status,
            &shown.state,
            &PlainLook,
        ),
    }
    // The status line the next redraw keeps is the answer's, not the press's.
    if let Some(id) = existing_id(host) {
        SHOWN.with(|map| {
            if let Some(entry) = map.borrow_mut().get_mut(&id) {
                entry.status = shown.status.clone();
            }
        });
    }
    refocus(host);
}

/// A fold button: the group's path goes in or out of `collapsed`, and the
/// status line says what happened, in the grid's words.
fn on_group_toggle(host: &HtmlElement, button: &Element) {
    let Some(raw) = button.get_attribute("data-path") else {
        return;
    };
    let Some(path) = opengrid_json::from_str::<opengrid_json::Json>(&raw)
        .ok()
        .and_then(|path| path.as_array().cloned())
    else {
        return;
    };
    let Some(shown) = shown(host) else {
        return;
    };
    let folded = pivot::parse_collapsed(host.get_attribute(COLLAPSED_ATTRIBUTE).as_deref())
        .unwrap_or_default();
    let next = pivot::toggle_collapsed(&folded, &path);
    let open = !next.contains(&path);
    let model = PivotModel {
        collapsed: next.clone(),
        ..(*shown.model).clone()
    };
    let texts = texts::texts(host);
    let name = match HostLook::new(host) {
        Ok(look) => pivot::group_label(&model, &look, &texts, &path),
        Err(_) => pivot::group_label(&model, &PlainLook, &texts, &path),
    };
    let sentence = texts.group_toggled(&name, model.shown_below(&path), open);
    on_release(|id| {
        SAY.with(|map| map.borrow_mut().remove(&id));
        PRESSED.with(|map| map.borrow_mut().remove(&id));
    });
    let id = host_id(host);
    SAY.with(|map| map.borrow_mut().insert(id, sentence));
    PRESSED.with(|map| {
        map.borrow_mut()
            .insert(id, vec![("data-path".to_owned(), raw)])
    });
    let value = pivot::collapsed_attribute(&next);
    let _ = if value.is_empty() {
        host.remove_attribute(COLLAPSED_ATTRIBUTE)
    } else {
        host.set_attribute(COLLAPSED_ATTRIBUTE, &value)
    };
}

// ---------------------------------------------------------------------------
// The field toolbar (issue #112)
// ---------------------------------------------------------------------------

/// Routes a click inside the toolbar or its menu; `true` when it was one.
fn on_toolbar_click(host: &HtmlElement, target: &Element) -> bool {
    let closest = |selector: &str| target.closest(selector).ok().flatten();
    if let Some(item) = closest(r#"[part="field-menu"] [role="menuitem"]"#) {
        pick_field(host, &item);
    } else if let Some(button) = closest("button[data-add]") {
        if button.get_attribute("aria-disabled").as_deref() != Some("true") {
            open_field_menu(host, &button);
        }
    } else if let Some(button) = closest("button[data-remove]") {
        let raw = button.get_attribute("data-remove").unwrap_or_default();
        if let Some((axis, name)) = handle(&raw) {
            act(
                host,
                FieldAction::Remove(axis, name),
                vec![add_button(axis)],
            );
        }
    } else if let Some(button) = closest("button[data-move]") {
        let raw = button.get_attribute("data-move").unwrap_or_default();
        let Some((rest, direction)) = raw.rsplit_once('|') else {
            return true;
        };
        if let Some((axis, name)) = handle(rest) {
            let later = direction == "later";
            let opposite = if later { "earlier" } else { "later" };
            act(
                host,
                FieldAction::Move(axis, name, later),
                vec![
                    ("data-move".to_owned(), raw.clone()),
                    ("data-move".to_owned(), format!("{rest}|{opposite}")),
                ],
            );
        }
    } else {
        return false;
    }
    true
}

/// `axis|name` as written on a chip's buttons.
fn handle(raw: &str) -> Option<(Axis, String)> {
    let (axis, name) = raw.split_once('|')?;
    Some((Axis::from_token(axis)?, name.to_owned()))
}

fn add_button(axis: Axis) -> (String, String) {
    ("data-add".to_owned(), axis.token().to_owned())
}

/// Applies a toolbar action as one view, and says where the focus goes back.
fn act(host: &HtmlElement, action: FieldAction, focus: Vec<(String, String)>) {
    let Ok(offer) = offer(host) else {
        return;
    };
    let next = pivot::apply_field(&view_of(host), &offer, &action);
    on_release(|id| {
        PRESSED.with(|map| map.borrow_mut().remove(&id));
    });
    let id = host_id(host);
    PRESSED.with(|map| map.borrow_mut().insert(id, focus));
    apply_view(host, &next);
}

/// Opens the add menu under `button`: what is offered and not in use.
fn open_field_menu(host: &HtmlElement, button: &Element) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    close_field_menu(host, false);
    let Some(axis) = button
        .get_attribute("data-add")
        .and_then(|token| Axis::from_token(&token))
    else {
        return;
    };
    let Ok(offer) = offer(host) else {
        return;
    };
    let Some(document) = host.owner_document() else {
        return;
    };
    let Ok(menu) = document.create_element("div") else {
        return;
    };
    let texts = texts::texts(host);
    let look: Box<dyn PivotLook> = match HostLook::new(host) {
        Ok(look) => Box::new(look),
        Err(_) => Box::new(PlainLook),
    };
    let _ = menu.set_attribute("part", "field-menu");
    let _ = menu.set_attribute("role", "menu");
    let _ = menu.set_attribute("popover", "auto");
    let _ = menu.set_attribute("data-axis", axis.token());
    let _ = menu.set_attribute(
        "aria-label",
        button
            .text_content()
            .unwrap_or_default()
            .trim_start_matches('+')
            .trim(),
    );
    if !texts.lang.is_empty() {
        let _ = menu.set_attribute("lang", &texts.lang);
    }
    for name in offer.open(&view_of(host), axis) {
        if let Ok(item) = document.create_element("div") {
            let _ = item.set_attribute("role", "menuitem");
            let _ = item.set_attribute("tabindex", "-1");
            let _ = item.set_attribute("data-field", &name);
            item.set_text_content(Some(&look.title(&name)));
            let _ = menu.append_child(&item);
        }
    }
    let Ok(menu) = menu.dyn_into::<HtmlElement>() else {
        return;
    };
    let _ = root.append_child(&menu);
    let _ = menu.show_popover();
    let _ = button.set_attribute("aria-expanded", "true");
    if let Ok(button) = button.clone().dyn_into::<HtmlElement>() {
        crate::element::place_under(&menu, &button);
    }
    // Light dismiss (a click outside) closes it; the button says so.
    let owner = host.clone();
    let toggle = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
        let open = js_sys::Reflect::get(&event, &JsValue::from_str("newState"))
            .ok()
            .and_then(|state| state.as_string());
        if open.as_deref() == Some("closed") {
            close_field_menu(&owner, false);
        }
    })
    .into_js_value();
    let _ = menu.add_event_listener_with_callback("toggle", toggle.unchecked_ref());
    if let Ok(Some(first)) = menu.query_selector(r#"[role="menuitem"]"#)
        && let Ok(first) = first.dyn_into::<HtmlElement>()
    {
        let _ = first.focus();
    }
}

/// Closes the open add menu, and gives its button the focus when asked.
fn close_field_menu(host: &HtmlElement, refocus: bool) {
    let Some(root) = host.shadow_root() else {
        return;
    };
    let Ok(Some(menu)) = root.query_selector(r#"[part="field-menu"]"#) else {
        return;
    };
    let axis = menu.get_attribute("data-axis").unwrap_or_default();
    if let Ok(menu) = menu.clone().dyn_into::<HtmlElement>() {
        let _ = menu.hide_popover();
    }
    menu.remove();
    if let Ok(Some(button)) = root.query_selector(&format!(r#"button[data-add="{axis}"]"#)) {
        let _ = button.set_attribute("aria-expanded", "false");
        if refocus && let Ok(button) = button.dyn_into::<HtmlElement>() {
            let _ = button.focus();
        }
    }
}

/// A menu item: its field goes on the menu's axis.
fn pick_field(host: &HtmlElement, item: &Element) {
    let Some(name) = item.get_attribute("data-field") else {
        return;
    };
    let Some(axis) = item
        .closest(r#"[part="field-menu"]"#)
        .ok()
        .flatten()
        .and_then(|menu| menu.get_attribute("data-axis"))
        .and_then(|token| Axis::from_token(&token))
    else {
        return;
    };
    close_field_menu(host, true);
    act(host, FieldAction::Add(axis, name), vec![add_button(axis)]);
}

/// The add menu's keys: arrows and Home/End move, Enter and Space pick,
/// Escape closes and returns to the button, Tab closes.
fn on_menu_key(event: Event) {
    let Ok(event) = event.dyn_into::<web_sys::KeyboardEvent>() else {
        return;
    };
    #[cfg(feature = "grid")]
    if event.key() == "Enter"
        && let Some(input) = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
            .filter(|input| input.has_attribute("data-filter-input"))
        && let Some(host) = event
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::ShadowRoot>().ok())
            .and_then(|root| root.host().dyn_into::<HtmlElement>().ok())
    {
        event.prevent_default();
        submit_filter(&host, input.value());
        return;
    }
    let Some(item) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .filter(|target| {
            target
                .closest(r#"[part="field-menu"]"#)
                .ok()
                .flatten()
                .is_some()
        })
    else {
        return;
    };
    let Some(host) = event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::ShadowRoot>().ok())
        .and_then(|root| root.host().dyn_into::<HtmlElement>().ok())
    else {
        return;
    };
    let Some(menu) = item.closest(r#"[part="field-menu"]"#).ok().flatten() else {
        return;
    };
    let items: Vec<HtmlElement> = menu
        .query_selector_all(r#"[role="menuitem"]"#)
        .map(|list| {
            (0..list.length())
                .filter_map(|index| list.item(index))
                .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
                .collect()
        })
        .unwrap_or_default();
    let at = items
        .iter()
        .position(|other| other.is_same_node(Some(&item)))
        .unwrap_or(0);
    let go = |index: usize| {
        if let Some(target) = items.get(index) {
            let _ = target.focus();
        }
    };
    match event.key().as_str() {
        "ArrowDown" => go((at + 1) % items.len().max(1)),
        "ArrowUp" => go((at + items.len().max(1) - 1) % items.len().max(1)),
        "Home" => go(0),
        "End" => go(items.len().saturating_sub(1)),
        "Enter" | " " => pick_field(&host, &item),
        "Escape" => close_field_menu(&host, true),
        "Tab" => {
            close_field_menu(&host, false);
            return;
        }
        _ => return,
    }
    event.prevent_default();
}

/// One attribute of `view`, as [`PivotView::attributes`] writes it.
fn view_attribute(view: &PivotView, name: &str) -> String {
    VIEW_ATTRIBUTES
        .iter()
        .zip(view.attributes())
        .find(|((attribute, _), _)| *attribute == name)
        .map(|(_, value)| value)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The filter (issue #114)
// ---------------------------------------------------------------------------

/// The expression field, as the focus goes back to it.
fn filter_input() -> (String, String) {
    ("data-filter-input".to_owned(), String::new())
}

/// A filter chip's remove button, or "Remove all": the filter without it.
fn on_filter_click(host: &HtmlElement, target: &Element) -> bool {
    let closest = |selector: &str| target.closest(selector).ok().flatten();
    let view = view_of(host);
    let parts = pivot::clauses(view.filter.as_ref());
    let (next, focus) = if let Some(button) = closest("button[data-filter-remove]") {
        let Some(index) = button
            .get_attribute("data-filter-remove")
            .and_then(|index| index.parse::<usize>().ok())
        else {
            return true;
        };
        let mut rest = parts;
        if index < rest.len() {
            rest.remove(index);
        }
        // The chip that takes its place, else the one before, else the field.
        let mut focus = Vec::new();
        for at in [Some(index), index.checked_sub(1)].into_iter().flatten() {
            if at < rest.len() {
                focus.push(("data-filter-remove".to_owned(), at.to_string()));
            }
        }
        focus.push(filter_input());
        (pivot::join_clauses(rest), focus)
    } else if closest("button[data-filter-clear]").is_some() {
        (None, vec![filter_input()])
    } else {
        return false;
    };
    set_filter(host, next, focus);
    true
}

/// Writes `filter` into the view — one query, one report — and says where
/// the focus goes back.
fn set_filter(
    host: &HtmlElement,
    filter: Option<opengrid_json::Json>,
    focus: Vec<(String, String)>,
) {
    let next = PivotView {
        filter,
        ..view_of(host)
    };
    on_release(|id| {
        PRESSED.with(|map| map.borrow_mut().remove(&id));
    });
    let id = host_id(host);
    PRESSED.with(|map| map.borrow_mut().insert(id, focus));
    apply_view(host, &next);
}

/// Enter in the expression field: the clauses join the filter.
///
/// The pivot has no schema of its own, and an expression needs the types of
/// the fields it names. They are asked of the provider the pivot has — every
/// one answers a pivot, not every one a query: a pivot over the fields the
/// pivot knows, as rows, under a filter nothing passes. Its answer is the
/// grand total alone, and its columns carry the types. What does not parse is
/// said, and nothing changes.
#[cfg(feature = "grid")]
fn submit_filter(host: &HtmlElement, text: String) {
    if text.trim().is_empty() {
        return;
    }
    let Some(provider) = provider(host) else {
        return;
    };
    let Some(source) = host.get_attribute(DATASOURCE_ATTRIBUTE) else {
        return;
    };
    let view = view_of(host);
    let offer = offer(host).unwrap_or_default();
    let mut fields: Vec<String> = Vec::new();
    let measured = view
        .values
        .iter()
        .chain(&offer.measures)
        .filter_map(|measure| measure["field"].as_str().map(str::to_owned));
    for field in offer
        .fields
        .iter()
        .chain(&view.rows)
        .chain(&view.columns)
        .cloned()
        .chain(measured)
    {
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    let Some(first) = fields.first().cloned() else {
        say(
            host,
            &texts::texts(host).query_problem(&crate::search::Problem::UnknownColumn(
                text.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
            )),
        );
        return;
    };
    let probe = opengrid_json::json!({
        "source": source,
        "rows": fields,
        "columns": [],
        "values": [{ "fn": "count", "as": "opengrid_probe" }],
        "filter": { "and": [
            { "field": first, "op": "is_null" },
            { "field": first, "op": "is_not_null" },
        ] },
    });
    let promise = provider.pivot(&probe.to_string(), "");
    let host = host.clone();
    spawn_local(async move {
        let texts = texts::texts(&host);
        let schema = match JsFuture::from(promise).await {
            Ok(value) => pivot_json(&value).and_then(|json| {
                let body: opengrid_json::Json =
                    opengrid_json::from_str(&json).map_err(|error| error.to_string())?;
                crate::table::parse_result(&body["result"].to_string()).map(|result| result.schema)
            }),
            Err(value) => Err(describe(&value)),
        };
        let schema = match schema {
            Ok(schema) => schema,
            Err(message) => {
                say(&host, &texts.error(&message));
                return;
            }
        };
        let entries = match crate::search::parse(&text, &texts.query_and, &schema) {
            Ok(entries) => entries,
            Err(problem) => {
                say(&host, &texts.query_problem(&problem));
                return;
            }
        };
        let added = match crate::grid::filter_expr(&entries, &schema) {
            Ok(Some(expr)) => opengrid_json::ToJson::to_json(&expr),
            Ok(None) => return,
            Err(problems) => {
                if let Some(problem) = problems.first() {
                    say(
                        &host,
                        &texts.filter_invalid(&problem.column, &problem.value),
                    );
                }
                return;
            }
        };
        let mut parts = pivot::clauses(view_of(&host).filter.as_ref());
        parts.extend(pivot::clauses(Some(&added)));
        set_filter(&host, pivot::join_clauses(parts), vec![filter_input()]);
    });
}

/// Gives the pressed sort button the focus back, once there is one again.
fn refocus(host: &HtmlElement) {
    let Some(id) = existing_id(host) else {
        return;
    };
    let Some(candidates) = PRESSED.with(|map| map.borrow_mut().remove(&id)) else {
        return;
    };
    let Some(root) = host.shadow_root() else {
        return;
    };
    // The first that is there: a move button at an edge is gone, its
    // opposite is not.
    for (key, value) in candidates {
        let selector = format!(r#"[{key}="{}"]"#, value.replace('"', "\\\""));
        if let Ok(Some(button)) = root.query_selector(&selector)
            && let Ok(button) = button.dyn_into::<HtmlElement>()
        {
            let _ = button.focus();
            return;
        }
    }
}

/// Puts `text` in the status line without touching what is shown.
fn say(host: &HtmlElement, text: &str) {
    if let Some(status) = host
        .shadow_root()
        .and_then(|root| root.query_selector(r#"[part="status"]"#).ok().flatten())
    {
        status.set_text_content(Some(text));
        let _ = status.set_attribute("data-state", "error");
    }
}

thread_local! {
    /// The answer each pivot shows, by host id: the pivot wire form, as the
    /// provider sent it.
    static SHOWN: RefCell<HashMap<u32, Shown>> = RefCell::new(HashMap::new());
}

/// What a pivot shows: the answer as it came, the model drawn from it, and
/// its status line — enough to draw it again without asking (issue #110).
#[derive(Clone)]
struct Shown {
    answer: Rc<str>,
    model: Rc<PivotModel>,
    status: String,
    state: String,
}

fn forget(id: u32) {
    SHOWN.with(|map| map.borrow_mut().remove(&id));
}

/// Keeps what `host` shows, or forgets what it showed.
fn remember(host: &HtmlElement, shown: Option<(&PivotModel, &str, &str, &str)>) {
    match shown {
        Some((model, answer, status, state)) => {
            on_release(forget);
            let id = host_id(host);
            let shown = Shown {
                answer: Rc::from(answer),
                model: Rc::new(model.clone()),
                status: status.to_owned(),
                state: state.to_owned(),
            };
            SHOWN.with(|map| map.borrow_mut().insert(id, shown));
        }
        None => {
            if let Some(id) = existing_id(host) {
                forget(id);
            }
        }
    }
}

/// [`crate::element::get_pivot`] — the shown pivot as CSV, or `null`.
///
/// The options are read first, so a wrong one is an error whether or not a
/// pivot is shown yet.
pub(crate) fn read_pivot(host: &HtmlElement, options: &JsValue) -> Result<JsValue, JsError> {
    let options = crate::export::pivot_options(options)?;
    let Some(answer) = existing_id(host)
        .and_then(|id| SHOWN.with(|map| map.borrow().get(&id).map(|shown| shown.answer.clone())))
    else {
        return Ok(JsValue::NULL);
    };
    let csv = crate::export::pivot_csv(&answer, &texts::texts(host), &options)?;
    Ok(JsValue::from_str(&csv))
}
