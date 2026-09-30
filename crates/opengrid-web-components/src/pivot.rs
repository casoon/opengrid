//! The `<opengrid-pivot>` model: query building, result parsing and the native
//! `<table>` as pure patch data (plan point 32).
//!
//! # Why Table Mode
//!
//! 09-accessibility.md says it outright: not every table is forced into a
//! complex ARIA grid. A two-level column header made of `colspan` and
//! `scope="colgroup"` is solved HTML that every screen reader knows, while a
//! **virtual** pivot with spanning headers is, by 06-pivot.md's own reckoning,
//! the riskiest thing in the project (R5). V1 therefore renders the whole pivot
//! — no virtualization, no paging — and the limits of point 30 are what make
//! that safe: 256 columns, 2000 rows, and an error beyond them.
//!
//! Everything here is portable data, so the same functions run in unit tests on
//! the host and in the browser through the renderer.

use opengrid_json::{Json as Value, json};

use opengrid_web_core::element::{LABEL_ATTRIBUTE, mirror_label};
use opengrid_web_core::patch::{NodeAllocator, NodeId, Patch, PatchBuffer};

use crate::shared::element;

use crate::texts::GridTexts;

/// The custom element name (E1).
pub const PIVOT_TAG: &str = "opengrid-pivot";

/// The host attribute naming the source.
pub const DATASOURCE_ATTRIBUTE: &str = "datasource";

/// The host attribute listing the row dimensions, comma-separated.
pub const ROWS_ATTRIBUTE: &str = "rows";

/// The host attribute listing the column dimensions, comma-separated.
pub const COLUMNS_ATTRIBUTE: &str = "columns";

/// The host attribute carrying the measures, as the JSON of the wire form.
///
/// `values='[{"field":"qty","fn":"sum","as":"total"}]'`. Deliberately the
/// contract's own shape rather than an invented mini-syntax: one notation to
/// learn, and nothing to keep in step with the query model.
pub const VALUES_ATTRIBUTE: &str = "values";

/// The host attribute ordering the rows (rule P9, issue #108): the wire's own
/// JSON, `[{ "field", "by"?, "direction" }]`. The reader's clicks write it, so
/// the view stays "the attributes".
pub const SORT_ATTRIBUTE: &str = "sort";

/// The host attribute listing the folded groups (issue #110), as JSON: a list
/// of group paths, `[["DE"], ["FR", "Beta"]]`, NULL as `null`. Display only.
pub const COLLAPSED_ATTRIBUTE: &str = "collapsed";

/// The boolean host attribute that shows the field toolbar (issue #112).
pub const TOOLBAR_ATTRIBUTE: &str = "toolbar";

/// The fields a reader may pivot by, comma-separated (issue #112).
pub const FIELDS_ATTRIBUTE: &str = "fields";

/// The measures a reader may add, as the contract's JSON (issue #112).
pub const MEASURES_ATTRIBUTE: &str = "measures";

/// The marks of a group's fold button, beside its name and out of it.
const EXPANDED_MARK: &str = "▾";
const COLLAPSED_MARK: &str = "▸";

/// The host attributes the element reacts to.
pub const OBSERVED: &[&str] = &[
    LABEL_ATTRIBUTE,
    DATASOURCE_ATTRIBUTE,
    ROWS_ATTRIBUTE,
    COLUMNS_ATTRIBUTE,
    VALUES_ATTRIBUTE,
    SORT_ATTRIBUTE,
    COLLAPSED_ATTRIBUTE,
    TOOLBAR_ATTRIBUTE,
    FIELDS_ATTRIBUTE,
    MEASURES_ATTRIBUTE,
    // CSS alone: the look is `:host([theme=…])` rules (issue #32).
    crate::theme::THEME_ATTRIBUTE,
];

/// A comma-separated attribute as a list of names, empties dropped.
pub fn parse_dimensions(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Builds the pivot request JSON from the host attributes.
///
/// The measures are passed through as they were written: if they are not JSON,
/// or not an array, the element says so instead of sending something the server
/// would reject with a less helpful sentence.
pub fn pivot_json(
    source: &str,
    rows: &[String],
    columns: &[String],
    values: Option<&str>,
    sort: &[PivotOrder],
) -> Result<String, String> {
    let raw = values.unwrap_or_default().trim();
    if raw.is_empty() {
        return Err("the values attribute is empty: a pivot needs a measure".to_owned());
    }
    let values: Value = opengrid_json::from_str(raw)
        .map_err(|error| format!("the values attribute is not JSON: {error}"))?;
    if !values.is_array() {
        return Err("the values attribute must be a JSON array of measures".to_owned());
    }
    let mut request = json!({
        "source": source,
        "rows": rows,
        "columns": columns,
        "values": values,
    });
    // Only when there is one: a server from before rule P9 reads the rest.
    if !sort.is_empty()
        && let Value::Object(object) = &mut request
    {
        object.insert(
            "sort".to_owned(),
            Value::Array(sort.iter().map(PivotOrder::to_json).collect()),
        );
    }
    Ok(request.to_string())
}

/// One level's order (rule P9): by its own values, or `by` a measure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PivotOrder {
    /// The row dimension — the level.
    pub field: String,
    /// A measure alias; `None` orders the level by its own values.
    pub by: Option<String>,
    pub descending: bool,
}

impl PivotOrder {
    pub fn to_json(&self) -> Value {
        let direction = if self.descending { "desc" } else { "asc" };
        match &self.by {
            Some(by) => json!({ "field": self.field, "by": by, "direction": direction }),
            None => json!({ "field": self.field, "direction": direction }),
        }
    }

    /// `{ field, by?, direction? }`, or `None` when it is not that.
    fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let field = object.get("field")?.as_str()?.to_owned();
        let by = match object.get("by") {
            None => None,
            Some(by) => Some(by.as_str()?.to_owned()),
        };
        let descending = match object.get("direction").map(Value::as_str) {
            None => false,
            Some(Some("asc")) => false,
            Some(Some("desc")) => true,
            Some(_) => return None,
        };
        Some(Self {
            field,
            by,
            descending,
        })
    }
}

/// The `sort` attribute as orders. Absent or empty is none; anything that is
/// not a list of `{ field, by?, direction }` is an error the page can fix.
pub fn parse_sort(raw: Option<&str>) -> Result<Vec<PivotOrder>, String> {
    let raw = raw.unwrap_or_default().trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    opengrid_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .as_array()?
                .iter()
                .map(PivotOrder::from_json)
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| {
            "the sort attribute must be a JSON array of { field, by?, direction }".to_owned()
        })
}

/// The `collapsed` attribute as group paths. Absent or empty is none; anything
/// that is not a list of lists is an error the page can fix.
pub fn parse_collapsed(raw: Option<&str>) -> Result<Vec<Vec<Value>>, String> {
    let raw = raw.unwrap_or_default().trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    opengrid_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .as_array()?
                .iter()
                .map(|path| path.as_array().cloned())
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| "the collapsed attribute must be a JSON list of group paths".to_owned())
}

/// The `collapsed` attribute for `paths`; empty when nothing is folded.
pub fn collapsed_attribute(paths: &[Vec<Value>]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    Value::Array(paths.iter().cloned().map(Value::Array).collect()).to_string()
}

/// `paths` with `path` folded if it was open, opened if it was folded.
pub fn toggle_collapsed(paths: &[Vec<Value>], path: &[Value]) -> Vec<Vec<Value>> {
    if paths.iter().any(|folded| folded == path) {
        paths
            .iter()
            .filter(|folded| *folded != path)
            .cloned()
            .collect()
    } else {
        let mut next = paths.to_vec();
        next.push(path.to_vec());
        next
    }
}

/// One of the pivot's three axes, as the toolbar names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Rows,
    Columns,
    Values,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::Rows, Axis::Columns, Axis::Values];

    /// The token in `data-axis`.
    pub fn token(self) -> &'static str {
        match self {
            Axis::Rows => "rows",
            Axis::Columns => "columns",
            Axis::Values => "values",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        Axis::ALL.into_iter().find(|axis| axis.token() == token)
    }
}

/// What the page offers the reader (issue #112): fields to pivot by and
/// measures to add, whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Offer {
    pub fields: Vec<String>,
    pub measures: Vec<Value>,
}

impl Offer {
    /// The `fields` and `measures` attributes. `measures` that is not a list
    /// of measures is an error the page can fix.
    pub fn from_attributes(fields: Option<&str>, measures: Option<&str>) -> Result<Self, String> {
        let raw = measures.unwrap_or_default().trim();
        let measures = if raw.is_empty() {
            Vec::new()
        } else {
            opengrid_json::from_str::<Value>(raw)
                .ok()
                .and_then(|value| value.as_array().cloned())
                .filter(|list| list.iter().all(is_measure))
                .ok_or_else(|| {
                    "the measures attribute must be a JSON list of { field?, fn, as }".to_owned()
                })?
        };
        Ok(Self {
            fields: parse_dimensions(fields),
            measures,
        })
    }

    /// What the add button of `axis` would offer now: offered, and not in use.
    pub fn open(&self, view: &PivotView, axis: Axis) -> Vec<String> {
        match axis {
            Axis::Values => self
                .measures
                .iter()
                .filter_map(|measure| measure["as"].as_str())
                .filter(|alias| {
                    !view
                        .values
                        .iter()
                        .any(|value| value["as"].as_str() == Some(*alias))
                })
                .map(str::to_owned)
                .collect(),
            Axis::Rows | Axis::Columns => self
                .fields
                .iter()
                .filter(|field| !view.rows.contains(field) && !view.columns.contains(field))
                .cloned()
                .collect(),
        }
    }
}

/// What a reader does in the toolbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldAction {
    Add(Axis, String),
    Remove(Axis, String),
    /// Moves a field one place, later or earlier.
    Move(Axis, String, bool),
}

/// The names on `axis` of `view`: dimensions, or measure aliases.
pub fn names(view: &PivotView, axis: Axis) -> Vec<String> {
    match axis {
        Axis::Rows => view.rows.clone(),
        Axis::Columns => view.columns.clone(),
        Axis::Values => view
            .values
            .iter()
            .filter_map(|value| value["as"].as_str().map(str::to_owned))
            .collect(),
    }
}

/// The view after `action`, cleaned: a sort that names what is gone goes,
/// and the folded groups go when the rows change — their paths would name
/// other groups.
pub fn apply_field(view: &PivotView, offer: &Offer, action: &FieldAction) -> PivotView {
    let mut next = view.clone();
    let alias_of = |value: &Value| value["as"].as_str().map(str::to_owned);
    match action {
        FieldAction::Add(Axis::Rows, field) => next.rows.push(field.clone()),
        FieldAction::Add(Axis::Columns, field) => next.columns.push(field.clone()),
        FieldAction::Add(Axis::Values, alias) => {
            if let Some(measure) = offer
                .measures
                .iter()
                .find(|measure| alias_of(measure).as_deref() == Some(alias))
            {
                next.values.push(measure.clone());
            }
        }
        FieldAction::Remove(Axis::Rows, field) => next.rows.retain(|name| name != field),
        FieldAction::Remove(Axis::Columns, field) => next.columns.retain(|name| name != field),
        FieldAction::Remove(Axis::Values, alias) => next
            .values
            .retain(|value| alias_of(value).as_deref() != Some(alias)),
        FieldAction::Move(axis, name, later) => {
            let position = names(&next, *axis).iter().position(|other| other == name);
            if let Some(from) = position {
                let to = if *later {
                    from + 1
                } else {
                    from.wrapping_sub(1)
                };
                let length = names(&next, *axis).len();
                if to < length {
                    match axis {
                        Axis::Rows => next.rows.swap(from, to),
                        Axis::Columns => next.columns.swap(from, to),
                        Axis::Values => next.values.swap(from, to),
                    }
                }
            }
        }
    }
    let aliases = names(&next, Axis::Values);
    next.sort.retain(|order| {
        next.rows.contains(&order.field) && order.by.as_ref().is_none_or(|by| aliases.contains(by))
    });
    if next.rows != view.rows {
        next.collapsed.clear();
    }
    next
}

/// The field toolbar (issue #112), before the status line: per axis a
/// labelled group with its chips and its add button. The menus are built when
/// they open.
pub fn build_toolbar(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    parent: NodeId,
    view: &PivotView,
    offer: &Offer,
    texts: &GridTexts,
    look: &dyn PivotLook,
) {
    let toolbar = element(buffer, nodes, Some(parent), "div");
    attribute(buffer, toolbar, "part", "toolbar");
    attribute(buffer, toolbar, "role", "group");
    attribute(buffer, toolbar, "aria-label", &texts.pivot_toolbar);
    if !texts.lang.is_empty() {
        attribute(buffer, toolbar, "lang", &texts.lang);
    }
    for axis in Axis::ALL {
        let (label, add) = match axis {
            Axis::Rows => (&texts.pivot_rows, &texts.add_row),
            Axis::Columns => (&texts.pivot_columns, &texts.add_column),
            Axis::Values => (&texts.pivot_measures, &texts.add_measure),
        };
        let group = element(buffer, nodes, Some(toolbar), "div");
        attribute(buffer, group, "part", "field-group");
        attribute(buffer, group, "role", "group");
        attribute(buffer, group, "aria-label", label);
        attribute(buffer, group, "data-axis", axis.token());
        let caption = element(buffer, nodes, Some(group), "span");
        attribute(buffer, caption, "part", "menu-label");
        attribute(buffer, caption, "aria-hidden", "true");
        buffer.push(Patch::SetText {
            node: caption,
            text: label.clone(),
        });

        let chosen = names(view, axis);
        for (index, name) in chosen.iter().enumerate() {
            let title = look.title(name);
            let chip = element(buffer, nodes, Some(group), "span");
            attribute(buffer, chip, "part", "chip");
            attribute(buffer, chip, "data-field", name);
            let handle = format!("{}|{name}", axis.token());
            if index > 0 {
                chip_button(
                    buffer,
                    nodes,
                    chip,
                    "chip-move",
                    &texts.field_earlier(&title),
                    "◂",
                    ("data-move", &format!("{handle}|earlier")),
                );
            }
            let text = element(buffer, nodes, Some(chip), "span");
            buffer.push(Patch::SetText {
                node: text,
                text: title.clone(),
            });
            if index + 1 < chosen.len() {
                chip_button(
                    buffer,
                    nodes,
                    chip,
                    "chip-move",
                    &texts.field_later(&title),
                    "▸",
                    ("data-move", &format!("{handle}|later")),
                );
            }
            chip_button(
                buffer,
                nodes,
                chip,
                "chip-remove",
                &texts.field_remove(&title),
                "×",
                ("data-remove", &handle),
            );
        }

        let button = element(buffer, nodes, Some(group), "button");
        attribute(buffer, button, "part", "add-field");
        attribute(buffer, button, "type", "button");
        attribute(buffer, button, "data-add", axis.token());
        attribute(buffer, button, "aria-haspopup", "menu");
        attribute(buffer, button, "aria-expanded", "false");
        // V1 allows one column dimension (E20); and a menu with nothing in it
        // is not opened. Either way the button says why, in its name.
        let full = axis == Axis::Columns && !view.columns.is_empty();
        if full || offer.open(view, axis).is_empty() {
            attribute(buffer, button, "aria-disabled", "true");
            attribute(
                buffer,
                button,
                "aria-label",
                if full {
                    &texts.column_full
                } else {
                    &texts.nothing_to_add
                },
            );
        }
        buffer.push(Patch::SetText {
            node: button,
            text: add.clone(),
        });
    }
}

/// A chip's small button: its name in `aria-label`, its sign for the eye.
fn chip_button(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    chip: NodeId,
    part: &str,
    name: &str,
    sign: &str,
    (key, value): (&str, &str),
) {
    let button = element(buffer, nodes, Some(chip), "button");
    attribute(buffer, button, "part", part);
    attribute(buffer, button, "type", "button");
    attribute(buffer, button, "aria-label", name);
    attribute(buffer, button, key, value);
    buffer.push(Patch::SetText {
        node: button,
        text: sign.to_owned(),
    });
}

/// What a header sorts by when the reader presses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortTarget<'a> {
    /// A row dimension's own values, at its level.
    Dimension(&'a str),
    /// A measure, at every level.
    Measure(&'a str),
}

/// The order after the reader pressed `target` (issue #108).
///
/// A dimension toggles its level between ascending — the default, so its
/// entry goes — and descending; a level ordered by a measure starts again
/// ascending by its values. A measure orders every level by itself: ascending
/// first, descending when it already orders every level ascending.
pub fn next_sort(current: &[PivotOrder], rows: &[String], target: SortTarget) -> Vec<PivotOrder> {
    match target {
        SortTarget::Dimension(field) => {
            let descending = !matches!(
                current.iter().find(|order| order.field == field),
                Some(PivotOrder {
                    by: None,
                    descending: true,
                    ..
                })
            ) && !matches!(
                current.iter().find(|order| order.field == field),
                Some(PivotOrder { by: Some(_), .. })
            );
            let mut next: Vec<PivotOrder> = current
                .iter()
                .filter(|order| order.field != field)
                .cloned()
                .collect();
            if descending {
                next.push(PivotOrder {
                    field: field.to_owned(),
                    by: None,
                    descending: true,
                });
            }
            next
        }
        SortTarget::Measure(measure) => {
            let descending = measure_order(current, rows, measure) == Some(false);
            rows.iter()
                .map(|field| PivotOrder {
                    field: field.clone(),
                    by: Some(measure.to_owned()),
                    descending,
                })
                .collect()
        }
    }
}

/// Whether `measure` orders every level, and which way (`Some(true)` is
/// descending).
fn measure_order(current: &[PivotOrder], rows: &[String], measure: &str) -> Option<bool> {
    let mut direction = None;
    for field in rows {
        let order = current.iter().find(|order| order.field == *field)?;
        if order.by.as_deref() != Some(measure) || direction.is_some_and(|d| d != order.descending)
        {
            return None;
        }
        direction = Some(order.descending);
    }
    direction
}

/// `aria-sort` of a row dimension's header: its level's order by its values
/// — ascending unless said otherwise — or `none` while a measure orders it.
fn dimension_aria_sort(sort: &[PivotOrder], field: &str) -> &'static str {
    match sort.iter().find(|order| order.field == field) {
        None => "ascending",
        Some(PivotOrder { by: Some(_), .. }) => "none",
        Some(PivotOrder {
            descending: true, ..
        }) => "descending",
        Some(_) => "ascending",
    }
}

/// The pivot's view (issue #106): what it pivots by and what it measures.
///
/// **The attributes are the view.** `rows`, `columns` and `values` say it on
/// the element, so the view is read from them and written to them — there is
/// no second copy to fall out of step. `sort`, `collapsed` and `filters` join
/// it later (plan points 103, 104, 106).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotView {
    pub rows: Vec<String>,
    pub columns: Vec<String>,
    /// The measures, as the contract's own JSON: `[{ field?, fn, as }]`.
    pub values: Vec<Value>,
    /// Rule P9's orders; empty is every level ascending by its values.
    pub sort: Vec<PivotOrder>,
    /// The folded groups, as their paths (issue #110).
    pub collapsed: Vec<Vec<Value>>,
}

impl PivotView {
    /// The view the attributes say. A `values` that does not read as a list
    /// is no measures — the status line already says why.
    pub fn from_attributes(
        rows: Option<&str>,
        columns: Option<&str>,
        values: Option<&str>,
        sort: Option<&str>,
        collapsed: Option<&str>,
    ) -> Self {
        Self {
            rows: parse_dimensions(rows),
            columns: parse_dimensions(columns),
            values: values
                .and_then(|raw| opengrid_json::from_str::<Value>(raw.trim()).ok())
                .and_then(|values| values.as_array().cloned())
                .unwrap_or_default(),
            sort: parse_sort(sort).unwrap_or_default(),
            collapsed: parse_collapsed(collapsed).unwrap_or_default(),
        }
    }

    /// The attributes, in the order `rows`, `columns`, `values`, `sort`,
    /// `collapsed`.
    pub fn attributes(&self) -> [String; 5] {
        [
            self.rows.join(","),
            self.columns.join(","),
            Value::Array(self.values.clone()).to_string(),
            if self.sort.is_empty() {
                String::new()
            } else {
                Value::Array(self.sort.iter().map(PivotOrder::to_json).collect()).to_string()
            },
            collapsed_attribute(&self.collapsed),
        ]
    }

    pub fn to_json(&self) -> Value {
        json!({
            "rows": self.rows,
            "columns": self.columns,
            "values": Value::Array(self.values.clone()),
            "sort": Value::Array(self.sort.iter().map(PivotOrder::to_json).collect()),
            "collapsed": Value::Array(self.collapsed.iter().cloned().map(Value::Array).collect()),
        })
    }

    /// Reads a view, refusing it whole when any part does not hold — a pivot
    /// that looks restored and is not is the worse failure. A part left out
    /// is empty; a key the pivot does not know yet is ignored.
    pub fn from_json(value: &Value) -> Result<Self, Vec<String>> {
        let Some(object) = value.as_object() else {
            return Err(vec!["the view is not an object".to_owned()]);
        };
        let mut problems = Vec::new();
        let mut names = |key: &str| -> Vec<String> {
            let Some(list) = object.get(key) else {
                return Vec::new();
            };
            let names: Option<Vec<String>> = list.as_array().and_then(|list| {
                list.iter()
                    .map(|name| name.as_str().filter(|name| !name.trim().is_empty()))
                    .map(|name| name.map(str::to_owned))
                    .collect()
            });
            names.unwrap_or_else(|| {
                problems.push(format!("{key} is not a list of field names"));
                Vec::new()
            })
        };
        let rows = names("rows");
        let columns = names("columns");
        let values = match object.get("values") {
            None => Vec::new(),
            Some(list) => match list.as_array() {
                Some(list) if list.iter().all(is_measure) => list.clone(),
                _ => {
                    problems.push("values is not a list of measures { field?, fn, as }".to_owned());
                    Vec::new()
                }
            },
        };
        let sort = match object.get("sort") {
            None => Vec::new(),
            Some(list) => match list
                .as_array()
                .and_then(|list| list.iter().map(PivotOrder::from_json).collect())
            {
                Some(sort) => sort,
                None => {
                    problems.push("sort is not a list of { field, by?, direction }".to_owned());
                    Vec::new()
                }
            },
        };
        let collapsed = match object.get("collapsed") {
            None => Vec::new(),
            Some(list) => match list
                .as_array()
                .and_then(|list| list.iter().map(|path| path.as_array().cloned()).collect())
            {
                Some(paths) => paths,
                None => {
                    problems.push("collapsed is not a list of group paths".to_owned());
                    Vec::new()
                }
            },
        };
        if problems.is_empty() {
            Ok(Self {
                rows,
                columns,
                values,
                sort,
                collapsed,
            })
        } else {
            Err(problems)
        }
    }
}

/// A measure as the contract writes it: `fn` and `as` are names, `field` is
/// one when it is there. Whether the source has it is the engine's to say.
fn is_measure(value: &Value) -> bool {
    let Some(measure) = value.as_object() else {
        return false;
    };
    let name = |key: &str| measure.get(key).and_then(Value::as_str).is_some();
    name("fn")
        && name("as")
        && measure
            .get("field")
            .is_none_or(|field| field.as_str().is_some())
}

/// How the page wants the pivot to read (issue #104): its titles for fields
/// and measures, and its formats for their values.
///
/// Display only — `get_pivot` exports names and raw values.
pub trait PivotLook {
    /// What the dimension or measure called `name` is called where a reader
    /// reads it.
    fn title(&self, name: &str) -> String;
    /// The text of `value`, a value of the dimension or measure `name`. Never
    /// asked for NULL: a header has its own word for it, a cell is empty.
    fn text(&self, name: &str, value: &Value) -> String;
}

/// Field names and the values' own notation: the look without a page.
pub struct PlainLook;

impl PivotLook for PlainLook {
    fn title(&self, name: &str) -> String {
        name.to_owned()
    }

    fn text(&self, _name: &str, value: &Value) -> String {
        plain(value).unwrap_or_default()
    }
}

/// One generated column: what it stands for, and the measure it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnHeader {
    /// The column-dimension values, as JSON. Empty when the pivot has no
    /// column dimension.
    pub path: Vec<Value>,
    /// The measure alias.
    pub measure: String,
}

/// A pivot answer, ready to render.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotModel {
    /// The row dimensions, outermost first.
    pub row_dimensions: Vec<String>,
    /// The column dimensions, as the request named them — the answer does not
    /// carry them, and a format of a column dimension needs its name.
    pub column_dimensions: Vec<String>,
    /// The orders the request asked for (rule P9): what the headers say.
    pub sort: Vec<PivotOrder>,
    /// The folded groups (issue #110): their rows below are not drawn.
    pub collapsed: Vec<Vec<Value>>,
    pub columns: Vec<ColumnHeader>,
    /// Per row: how many row dimensions are set. See
    /// [`opengrid_pivot::PivotResult::row_levels`] — a subtotal is marked by
    /// this number, never by a NULL.
    pub levels: Vec<u16>,
    /// Per row, one cell per row dimension and then per column, as JSON.
    /// NULL is a value a **header** must not render as nothing (see
    /// [`GridTexts::dimension`]).
    pub rows: Vec<Vec<Value>>,
}

impl PivotModel {
    /// The group path of row `index`: its dimension values down to its level.
    pub fn path(&self, index: usize) -> &[Value] {
        let level = usize::from(self.levels[index]).min(self.row_dimensions.len());
        &self.rows[index][..level]
    }

    /// Whether row `index` lies inside a folded group — below it, not its
    /// own subtotal, which is what stays.
    pub fn folded(&self, index: usize) -> bool {
        let path = self.path(index);
        self.collapsed
            .iter()
            .any(|folded| folded.len() < path.len() && path[..folded.len()] == folded[..])
    }

    /// How many rows are drawn inside the group at `path`, its subtotal aside.
    pub fn shown_below(&self, path: &[Value]) -> u64 {
        (0..self.rows.len())
            .filter(|&index| {
                let row = self.path(index);
                row.len() > path.len() && row[..path.len()] == *path && !self.folded(index)
            })
            .count() as u64
    }

    /// How many measures repeat under each column value.
    pub fn measures(&self) -> usize {
        if self.columns.is_empty() {
            return 0;
        }
        let first = &self.columns[0].path;
        self.columns
            .iter()
            .take_while(|column| &column.path == first)
            .count()
    }

    /// The distinct column-dimension values, in order.
    pub fn groups(&self) -> Vec<&[Value]> {
        let mut groups: Vec<&[Value]> = Vec::new();
        for column in &self.columns {
            if groups.last() != Some(&column.path.as_slice()) {
                groups.push(&column.path);
            }
        }
        groups
    }
}

/// Reads the pivot wire form of point 53.
pub fn parse_result(json: &str) -> Result<PivotModel, String> {
    let body: Value = opengrid_json::from_str(json).map_err(|error| error.to_string())?;

    let row_dimensions: Vec<String> = body["row_dimensions"]
        .as_array()
        .ok_or("result has no row_dimensions")?
        .iter()
        .map(|name| name.as_str().unwrap_or_default().to_owned())
        .collect();

    let columns: Vec<ColumnHeader> = body["columns"]
        .as_array()
        .ok_or("result has no columns")?
        .iter()
        .map(|column| ColumnHeader {
            path: column["path"].as_array().cloned().unwrap_or_default(),
            measure: column["measure"].as_str().unwrap_or_default().to_owned(),
        })
        .collect();

    let levels: Vec<u16> = body["levels"]
        .as_array()
        .ok_or("result has no levels")?
        .iter()
        .map(|level| level.as_u64().unwrap_or_default() as u16)
        .collect();

    // The cells are the ordinary result form (E17): column-oriented, so they
    // are transposed here and nowhere else.
    let cells = body["result"]["columns"]
        .as_array()
        .ok_or("result has no cells")?;
    let mut rows = Vec::with_capacity(levels.len());
    for index in 0..levels.len() {
        rows.push(
            cells
                .iter()
                .map(|column| column["values"].get(index).cloned().unwrap_or(Value::Null))
                .collect(),
        );
    }

    Ok(PivotModel {
        row_dimensions,
        column_dimensions: Vec::new(),
        sort: Vec::new(),
        collapsed: Vec::new(),
        columns,
        levels,
        rows,
    })
}

/// A JSON scalar in its own notation, keeping NULL apart from the empty string.
///
/// A data cell renders both as nothing; a **header** must not (S14 —
/// `""` is not NULL, and an empty header cell is announced as silence).
pub fn plain(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// A dimension value as a header: NULL and the empty string keep their words,
/// so a format never blanks a header; every other value is the page's.
fn header(look: &dyn PivotLook, texts: &GridTexts, name: &str, value: &Value) -> String {
    match plain(value).as_deref() {
        None | Some("") => texts.dimension(plain(value).as_deref()),
        Some(_) => look.text(name, value),
    }
}

/// The name of the group at `path`, as its header says it: formatted, and
/// NULL and the empty string in their words.
pub fn group_label(
    model: &PivotModel,
    look: &dyn PivotLook,
    texts: &GridTexts,
    path: &[Value],
) -> String {
    match (path.len().checked_sub(1), path.last()) {
        (Some(level), Some(value)) => header(look, texts, &model.row_dimensions[level], value),
        _ => texts.total.clone(),
    }
}

/// A data cell as text. NULL renders as nothing — there is a value or there
/// is not, and the column header says which column.
fn cell(look: &dyn PivotLook, measure: &str, value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        value => look.text(measure, value),
    }
}

/// Builds the whole pivot as one patch list.
///
/// Two header rows when there is a column dimension — values with `colspan` and
/// `scope="colgroup"`, measures under them with `scope="col"` — and one when
/// there is not. Every data row starts with `<th scope="row">`, so each cell is
/// announced with the row it is in and the column it is under.
#[allow(clippy::too_many_arguments)]
pub fn build_pivot(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    label: Option<&str>,
    model: Option<&PivotModel>,
    status: &str,
    state: &str,
    texts: &GridTexts,
    look: &dyn PivotLook,
    toolbar: Option<(&PivotView, &Offer)>,
) {
    let layout = element(buffer, nodes, Some(NodeId::ROOT), "div");
    attribute(buffer, layout, "part", "layout");
    if let Some((view, offer)) = toolbar {
        build_toolbar(buffer, nodes, layout, view, offer, texts, look);
    }

    let status_node = element(buffer, nodes, Some(layout), "p");
    attribute(buffer, status_node, "part", "status");
    attribute(buffer, status_node, "role", "status");
    attribute(buffer, status_node, "aria-live", "polite");
    attribute(buffer, status_node, "data-state", state);
    if !texts.lang.is_empty() {
        attribute(buffer, status_node, "lang", &texts.lang);
    }
    buffer.push(Patch::SetText {
        node: status_node,
        text: status.to_owned(),
    });

    // Parts (issue #29) with the grid's names where there is one, so a page
    // styles the pivot from outside the shadow root: `table`, `caption`,
    // `header` (a column header cell), `row-header`, `row`, `total-row`, `cell`.
    let table = element(buffer, nodes, Some(layout), "table");
    attribute(buffer, table, "part", "table");
    if let Some((name, value)) = mirror_label(label) {
        attribute(buffer, table, name, value);
    }
    let caption = element(buffer, nodes, Some(table), "caption");
    attribute(buffer, caption, "part", "caption");
    buffer.push(Patch::SetText {
        node: caption,
        text: label.unwrap_or_default().to_owned(),
    });

    if let Some(model) = model {
        let measures = model.measures();
        let groups = model.groups();
        let nested = groups.first().is_some_and(|path| !path.is_empty());

        let thead = element(buffer, nodes, Some(table), "thead");
        let first_row = element(buffer, nodes, Some(thead), "tr");
        for name in &model.row_dimensions {
            let th = element(buffer, nodes, Some(first_row), "th");
            attribute(buffer, th, "part", "header");
            attribute(buffer, th, "scope", "col");
            if nested {
                // The dimension name spans both header rows, so the row below
                // holds only the measures.
                attribute(buffer, th, "rowspan", "2");
            }
            let aria_sort = dimension_aria_sort(&model.sort, name);
            sort_button(
                buffer,
                nodes,
                th,
                &look.title(name),
                aria_sort,
                ("data-sort-field", name),
            );
        }

        if nested {
            for group in &groups {
                let th = element(buffer, nodes, Some(first_row), "th");
                attribute(buffer, th, "part", "header");
                attribute(buffer, th, "scope", "colgroup");
                attribute(buffer, th, "colspan", &measures.to_string());
                // The separator is the one `get_pivot`'s CSV header composes
                // with (`opengrid_export::PATH_SEPARATOR`), so a column is named
                // in the same words in the table and in its export.
                buffer.push(Patch::SetText {
                    node: th,
                    text: group
                        .iter()
                        .enumerate()
                        .map(|(depth, value)| {
                            let name = model.column_dimensions.get(depth);
                            header(look, texts, name.map_or("", String::as_str), value)
                        })
                        .collect::<Vec<_>>()
                        .join(" · "),
                });
            }
            // Not sort buttons: a cell here is one column value's, and a
            // measure orders by the whole row (P9) — the order would not be
            // the one these cells show.
            let second_row = element(buffer, nodes, Some(thead), "tr");
            for column in &model.columns {
                let th = element(buffer, nodes, Some(second_row), "th");
                attribute(buffer, th, "part", "header");
                attribute(buffer, th, "scope", "col");
                buffer.push(Patch::SetText {
                    node: th,
                    text: look.title(&column.measure),
                });
            }
        } else {
            // Without a column dimension a measure's cells are the whole row,
            // so its header sorts by exactly what it shows.
            for column in &model.columns {
                let th = element(buffer, nodes, Some(first_row), "th");
                attribute(buffer, th, "part", "header");
                attribute(buffer, th, "scope", "col");
                let aria_sort =
                    match measure_order(&model.sort, &model.row_dimensions, &column.measure) {
                        Some(true) => "descending",
                        Some(false) => "ascending",
                        None => "none",
                    };
                sort_button(
                    buffer,
                    nodes,
                    th,
                    &look.title(&column.measure),
                    aria_sort,
                    ("data-sort-by", &column.measure),
                );
            }
        }

        let tbody = element(buffer, nodes, Some(table), "tbody");
        let dimensions = model.row_dimensions.len();
        for (index, cells) in model.rows.iter().enumerate() {
            if model.folded(index) {
                continue;
            }
            let level = usize::from(*model.levels.get(index).unwrap_or(&0));
            let tr = element(buffer, nodes, Some(tbody), "tr");
            attribute(buffer, tr, "data-level", &level.to_string());
            // Table Mode ships no stylesheet (the same choice as
            // `<opengrid-table>`), so the page needs a hook to shade a total
            // row. The text says it as well — shading alone is not information
            // (WCAG 1.4.1).
            attribute(
                buffer,
                tr,
                "part",
                if level < dimensions {
                    "row total-row"
                } else {
                    "row"
                },
            );

            if level < dimensions {
                // A subtotal: one header spanning the dimension columns, and it
                // **says** that it is a total (WCAG 1.4.1 — not colour alone).
                attribute(buffer, tr, "data-total", "true");
                let th = element(buffer, nodes, Some(tr), "th");
                attribute(buffer, th, "part", "row-header");
                attribute(buffer, th, "scope", "row");
                if dimensions > 1 {
                    attribute(buffer, th, "colspan", &dimensions.to_string());
                }
                if level == 0 {
                    buffer.push(Patch::SetText {
                        node: th,
                        text: texts.total.clone(),
                    });
                } else {
                    // The group's own row stays when it is folded, so its
                    // header holds the fold button (issue #110), named by the
                    // header's own words; the mark is beside the name.
                    let name = &model.row_dimensions[level - 1];
                    let text = texts.subtotal(&header(look, texts, name, &cells[level - 1]));
                    let path = &cells[..level];
                    let open = !model.collapsed.iter().any(|folded| folded == path);
                    let button = element(buffer, nodes, Some(th), "button");
                    attribute(buffer, button, "part", "group-toggle");
                    attribute(buffer, button, "type", "button");
                    attribute(buffer, button, "aria-expanded", &open.to_string());
                    attribute(
                        buffer,
                        button,
                        "data-path",
                        &Value::Array(path.to_vec()).to_string(),
                    );
                    let mark = crate::shared::marker(buffer, nodes, button, "group-mark");
                    buffer.push(Patch::SetText {
                        node: mark,
                        text: format!(
                            "{}\u{a0}",
                            if open { EXPANDED_MARK } else { COLLAPSED_MARK }
                        ),
                    });
                    let label = element(buffer, nodes, Some(button), "span");
                    buffer.push(Patch::SetText { node: label, text });
                }
            } else {
                for (value, name) in cells.iter().zip(&model.row_dimensions) {
                    let th = element(buffer, nodes, Some(tr), "th");
                    attribute(buffer, th, "part", "row-header");
                    attribute(buffer, th, "scope", "row");
                    buffer.push(Patch::SetText {
                        node: th,
                        text: header(look, texts, name, value),
                    });
                }
            }

            for (value, column) in cells.iter().skip(dimensions).zip(&model.columns) {
                let td = element(buffer, nodes, Some(tr), "td");
                attribute(buffer, td, "part", "cell");
                buffer.push(Patch::SetText {
                    node: td,
                    text: cell(look, &column.measure, value),
                });
            }
        }
    }
}

/// A header's sort button, as the table has it (issue #108): `aria-sort` on
/// the `<th>`, the name in its own span, the direction mark beside it and out
/// of the accessible name — the `<th>` already says the direction.
fn sort_button(
    buffer: &mut PatchBuffer,
    nodes: &mut NodeAllocator,
    th: NodeId,
    title: &str,
    aria_sort: &str,
    (key, value): (&str, &str),
) {
    attribute(buffer, th, "aria-sort", aria_sort);
    let button = element(buffer, nodes, Some(th), "button");
    attribute(buffer, button, "part", "sort-button");
    attribute(buffer, button, "type", "button");
    attribute(buffer, button, key, value);
    let name = element(buffer, nodes, Some(button), "span");
    buffer.push(Patch::SetText {
        node: name,
        text: title.to_owned(),
    });
    let mark = crate::shared::marker(buffer, nodes, button, "sort-direction");
    buffer.push(Patch::SetText {
        node: mark,
        text: crate::table::direction_mark(aria_sort),
    });
}

fn attribute(buffer: &mut PatchBuffer, node: NodeId, name: &str, value: &str) {
    buffer.push(Patch::SetAttribute {
        node,
        name: name.to_owned(),
        value: value.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{
        "row_dimensions": ["country"],
        "columns": [
            {"path": [2025], "measure": "total"},
            {"path": [2026], "measure": "total"}
        ],
        "levels": [1, 1, 1, 0],
        "result": {
            "total_count": 4,
            "row_count": 4,
            "columns": [
                {"name": "country", "type": "utf8", "nullable": true,
                 "values": ["DE", "FR", null, null]},
                {"name": "total_0", "type": "int64", "nullable": true,
                 "values": [1, null, null, 1]},
                {"name": "total_1", "type": "int64", "nullable": true,
                 "values": [258, 172, 269, 699]}
            ]
        }
    }"#;

    fn model() -> PivotModel {
        parse_result(ANSWER).expect("the pivot wire form parses")
    }

    fn markup(model: Option<&PivotModel>) -> String {
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_pivot(
            &mut buffer,
            &mut nodes,
            Some("Orders"),
            model,
            "3 matches",
            "ready",
            &GridTexts::default(),
            &PlainLook,
            None,
        );
        format!("{:?}", buffer.patches())
    }

    #[test]
    fn a_measure_list_must_be_json() {
        let rows = vec!["country".to_owned()];
        assert!(
            pivot_json(
                "orders",
                &rows,
                &[],
                Some(r#"[{"fn":"count","as":"n"}]"#),
                &[]
            )
            .is_ok()
        );
        assert!(pivot_json("orders", &rows, &[], None, &[]).is_err());
        assert!(pivot_json("orders", &rows, &[], Some("count(qty)"), &[]).is_err());
        assert!(
            pivot_json("orders", &rows, &[], Some(r#"{"fn":"count"}"#), &[]).is_err(),
            "an object is not a list of measures"
        );
    }

    #[test]
    fn dimensions_come_from_a_comma_separated_attribute() {
        assert_eq!(
            parse_dimensions(Some(" country , customer ,")),
            vec!["country".to_owned(), "customer".to_owned()]
        );
        assert!(parse_dimensions(None).is_empty());
    }

    /// The wire form of point 53 becomes rows, columns and levels.
    #[test]
    fn the_pivot_wire_form_becomes_a_model() {
        let model = model();
        assert_eq!(model.row_dimensions, vec!["country".to_owned()]);
        assert_eq!(model.measures(), 1, "one measure under each column value");
        assert_eq!(model.groups().len(), 2);
        assert_eq!(model.levels, vec![1, 1, 1, 0]);
        // Row 2 is the NULL **group** and row 3 the grand total. Both carry a
        // NULL in the dimension column; only the level tells them apart (P2).
        assert_eq!(model.levels[2], 1);
        assert_eq!(model.levels[3], 0);
        assert_eq!(model.rows[2][0], Value::Null);
        assert_eq!(model.rows[3][0], Value::Null);
    }

    /// The two-level column header: values with `colspan`, measures below.
    #[test]
    fn a_column_dimension_gives_a_two_level_header() {
        let markup = markup(Some(&model()));
        assert!(markup.contains("colgroup"), "{markup}");
        assert!(
            markup.contains("rowspan"),
            "the dimension name spans both rows"
        );
        assert!(markup.contains("\"2025\""), "{markup}");
    }

    /// A subtotal says that it is one. Colour alone is not information
    /// (WCAG 1.4.1), and a screen reader has nothing but the text.
    #[test]
    fn a_total_row_is_readable_as_a_total() {
        let markup = markup(Some(&model()));
        assert!(markup.contains("data-total"), "{markup}");
        assert!(
            markup.contains("Total"),
            "the grand total carries its name: {markup}"
        );
    }

    /// Every data row starts with a row header, so a cell is announced with the
    /// row it is in.
    #[test]
    fn every_row_has_a_header() {
        let markup = markup(Some(&model()));
        assert_eq!(
            markup.matches("name: \"scope\", value: \"row\"").count(),
            4,
            "{markup}"
        );
    }

    /// **No header cell is ever empty.** NULL and the empty string are two
    /// different groups (S10, S14) and both need a word: a blank header is
    /// silence to a screen reader and a shrug to everyone else.
    #[test]
    fn a_null_group_and_an_empty_string_group_are_both_named() {
        let texts = GridTexts::default();
        assert_eq!(texts.dimension(None), "(no value)");
        assert_eq!(texts.dimension(Some("")), "(empty)");
        assert_eq!(texts.dimension(Some("DE")), "DE");

        // The fixture's third row is the NULL country group.
        let markup = markup(Some(&model()));
        assert!(
            markup.contains("(no value)"),
            "the NULL group needs a name, and the total is not it"
        );
    }

    /// The page's look (issue #104): titles on the dimension and measure
    /// headers, formats on every value by its own name — and never on NULL.
    #[test]
    fn titles_and_formats_are_the_pages() {
        struct Page;
        impl PivotLook for Page {
            fn title(&self, name: &str) -> String {
                match name {
                    "country" => "Land".to_owned(),
                    "total" => "Umsatz".to_owned(),
                    other => other.to_owned(),
                }
            }
            fn text(&self, name: &str, value: &Value) -> String {
                format!("{name}={}", plain(value).unwrap_or_default())
            }
        }
        let mut model = model();
        model.column_dimensions = vec!["year".to_owned()];
        // The empty-string group, beside the NULL one.
        model.rows[1][0] = Value::String(String::new());
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_pivot(
            &mut buffer,
            &mut nodes,
            None,
            Some(&model),
            "",
            "ready",
            &GridTexts::default(),
            &Page,
            None,
        );
        let texts: Vec<String> = buffer
            .patches()
            .iter()
            .filter_map(|patch| match patch {
                Patch::SetText { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        for expected in ["Land", "Umsatz", "year=2025", "country=DE"] {
            assert!(
                texts.iter().any(|text| text == expected),
                "{expected}: {texts:?}"
            );
        }
        // The measure's alias names its format, not the result column's name.
        assert!(texts.iter().any(|text| text == "total=258"), "{texts:?}");
        assert!(
            !texts.iter().any(|text| text == "country"),
            "titled: {texts:?}"
        );
        // NULL and "" keep their words in a header; NULL stays empty in a cell.
        assert!(texts.iter().any(|text| text == "(no value)"), "{texts:?}");
        assert!(texts.iter().any(|text| text == "(empty)"), "{texts:?}");
        assert!(!texts.iter().any(|text| text.ends_with('=')), "{texts:?}");
    }

    /// The view is the attributes, and reads back as it was written.
    #[test]
    fn a_view_is_the_attributes_and_reads_back() {
        let view = PivotView::from_attributes(
            Some("country, customer"),
            Some("ordered_year"),
            Some(r#"[{"field":"qty","fn":"sum","as":"total"}]"#),
            Some(r#"[{"field":"country","by":"total","direction":"desc"}]"#),
            Some(r#"[["DE"],[null]]"#),
        );
        assert_eq!(view.rows, vec!["country".to_owned(), "customer".to_owned()]);
        assert_eq!(view.sort.len(), 1);
        assert_eq!(view.collapsed, vec![vec![json!("DE")], vec![Value::Null]]);
        let [rows, columns, values, sort, collapsed] = view.attributes();
        assert_eq!(rows, "country,customer");
        assert_eq!(columns, "ordered_year");
        assert_eq!(
            PivotView::from_attributes(
                Some(&rows),
                Some(&columns),
                Some(&values),
                Some(&sort),
                Some(&collapsed)
            ),
            view
        );
        assert_eq!(PivotView::from_json(&view.to_json()), Ok(view));
        // Unreadable measures are none — the status line says why.
        assert!(
            PivotView::from_attributes(None, None, Some("sum(qty)"), None, None)
                .values
                .is_empty()
        );
    }

    /// The reader's presses (issue #108): a dimension toggles its level, and
    /// ascending — the default — is no entry; a measure orders every level.
    #[test]
    fn a_press_toggles_the_order() {
        let rows = vec!["country".to_owned(), "customer".to_owned()];
        let desc = |field: &str| PivotOrder {
            field: field.to_owned(),
            by: None,
            descending: true,
        };
        let country = SortTarget::Dimension("country");
        assert_eq!(next_sort(&[], &rows, country), vec![desc("country")]);
        assert_eq!(next_sort(&[desc("country")], &rows, country), vec![]);
        // The other level keeps its order.
        assert_eq!(
            next_sort(&[desc("customer")], &rows, country),
            vec![desc("customer"), desc("country")]
        );

        let by_n = next_sort(&[], &rows, SortTarget::Measure("n"));
        assert_eq!(by_n.len(), 2);
        assert!(
            by_n.iter()
                .all(|order| order.by.as_deref() == Some("n") && !order.descending)
        );
        let again = next_sort(&by_n, &rows, SortTarget::Measure("n"));
        assert!(
            again.iter().all(|order| order.descending),
            "then descending"
        );
        assert!(
            next_sort(&again, &rows, SortTarget::Measure("n"))
                .iter()
                .all(|order| !order.descending),
            "and back"
        );
        // A level a measure orders starts again ascending by its values.
        assert_eq!(next_sort(&by_n, &rows, country).len(), 1);
    }

    /// The attribute is the wire's JSON; anything else is said.
    #[test]
    fn the_sort_attribute_is_the_wires_json() {
        assert_eq!(parse_sort(None), Ok(vec![]));
        assert_eq!(parse_sort(Some(" ")), Ok(vec![]));
        let sort = parse_sort(Some(r#"[{"field":"country","by":"n","direction":"desc"}]"#))
            .expect("the wire form");
        assert_eq!(sort[0].by.as_deref(), Some("n"));
        assert!(parse_sort(Some(r#"[{"field":"country","direction":"down"}]"#)).is_err());
        assert!(parse_sort(Some("country")).is_err());
        let request: Value = opengrid_json::from_str(
            &pivot_json(
                "orders",
                &["country".to_owned()],
                &[],
                Some(r#"[{"fn":"count","as":"n"}]"#),
                &sort,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(request["sort"][0]["direction"].as_str(), Some("desc"));
        let unsorted = pivot_json(
            "orders",
            &["country".to_owned()],
            &[],
            Some(r#"[{"fn":"count","as":"n"}]"#),
            &[],
        )
        .unwrap();
        assert!(!unsorted.contains("sort"), "no key without an order");
    }

    /// The headers say the order: a dimension ascending by default, a
    /// measure only when it orders every level — and without a column
    /// dimension alone is a measure header a button.
    #[test]
    fn the_headers_say_the_order() {
        let answer = r#"{"row_dimensions":["country"],"columns":[{"path":[],"measure":"n"}],
            "levels":[1,0],"result":{"total_count":2,"row_count":2,"columns":[
            {"name":"country","type":"utf8","nullable":true,"values":["DE",null]},
            {"name":"n_0","type":"int64","nullable":true,"values":[1,1]}]}}"#;
        let mut flat = parse_result(answer).unwrap();
        let sorts = |model: &PivotModel| {
            let markup = markup(Some(model));
            (
                markup.matches("value: \"sort-button\"").count(),
                markup
                    .match_indices("name: \"aria-sort\", value: \"")
                    .map(|(at, found)| {
                        let rest = &markup[at + found.len()..];
                        rest[..rest.find('"').unwrap()].to_owned()
                    })
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(sorts(&flat), (2, vec!["ascending".into(), "none".into()]));
        flat.sort = vec![PivotOrder {
            field: "country".to_owned(),
            by: Some("n".to_owned()),
            descending: true,
        }];
        assert_eq!(sorts(&flat), (2, vec!["none".into(), "descending".into()]));
        // With a column dimension only the dimension header sorts.
        assert_eq!(sorts(&model()).0, 1);
    }

    /// A folded group keeps its own row, now closed, and draws nothing below
    /// it (issue #110). The grand total and the innermost level fold nothing.
    #[test]
    fn a_folded_group_keeps_its_subtotal_alone() {
        let answer = r#"{"row_dimensions":["country","customer"],
            "columns":[{"path":[],"measure":"n"}],
            "levels":[2,2,1,2,1,0],"result":{"total_count":6,"row_count":6,"columns":[
            {"name":"country","type":"utf8","nullable":true,
             "values":["DE","DE","DE","FR","FR",null]},
            {"name":"customer","type":"utf8","nullable":true,
             "values":["Alpha","Beta",null,"Gamma",null,null]},
            {"name":"n_0","type":"int64","nullable":true,"values":[1,2,3,4,4,7]}]}}"#;
        let mut model = parse_result(answer).unwrap();
        let rows = |model: &PivotModel| markup(Some(model)).matches("data-level").count();
        let toggles = |model: &PivotModel| {
            let markup = markup(Some(model));
            (
                markup.matches("value: \"group-toggle\"").count(),
                markup
                    .matches("name: \"aria-expanded\", value: \"false\"")
                    .count(),
            )
        };
        assert_eq!(rows(&model), 6);
        assert_eq!(toggles(&model), (2, 0), "one per group, none for the total");
        assert_eq!(model.shown_below(&[json!("DE")]), 2);

        model.collapsed = toggle_collapsed(&[], &[json!("DE")]);
        assert_eq!(rows(&model), 4, "DE's two customers are not drawn");
        assert_eq!(toggles(&model), (2, 1));
        assert!(
            markup(Some(&model)).contains("Total DE"),
            "its subtotal stays"
        );
        assert_eq!(model.shown_below(&[json!("DE")]), 0);

        model.collapsed = toggle_collapsed(&model.collapsed, &[json!("DE")]);
        assert!(model.collapsed.is_empty(), "pressed again, open again");
        // A path that matches no group folds nothing.
        model.collapsed = vec![vec![json!("XX")]];
        assert_eq!(rows(&model), 6);

        assert_eq!(
            parse_collapsed(Some(r#"[["DE"],[null]]"#)).unwrap().len(),
            2
        );
        assert!(parse_collapsed(Some(r#"["DE"]"#)).is_err());
        assert_eq!(collapsed_attribute(&[]), "");
    }

    /// The toolbar's actions (issue #112): an add offers what is offered and
    /// not in use; a change takes along the sort and the folds it breaks.
    #[test]
    fn a_field_action_is_one_clean_view() {
        let offer = Offer::from_attributes(
            Some("country,customer,ordered_year"),
            Some(r#"[{"fn":"count","as":"n"},{"field":"qty","fn":"sum","as":"total"}]"#),
        )
        .unwrap();
        assert!(Offer::from_attributes(None, Some(r#"[{"fn":"count"}]"#)).is_err());
        let view = PivotView::from_attributes(
            Some("country,customer"),
            None,
            Some(r#"[{"fn":"count","as":"n"}]"#),
            Some(r#"[{"field":"customer","by":"n","direction":"desc"}]"#),
            Some(r#"[["DE"]]"#),
        );
        assert_eq!(
            offer.open(&view, Axis::Rows),
            vec!["ordered_year".to_owned()]
        );
        assert_eq!(offer.open(&view, Axis::Values), vec!["total".to_owned()]);

        let added = apply_field(
            &view,
            &offer,
            &FieldAction::Add(Axis::Values, "total".into()),
        );
        assert_eq!(
            names(&added, Axis::Values),
            vec!["n".to_owned(), "total".to_owned()]
        );
        assert_eq!(
            added.values[1]["fn"].as_str(),
            Some("sum"),
            "the offered measure, whole"
        );
        assert_eq!(added.collapsed.len(), 1, "the rows did not change");

        let moved = apply_field(
            &view,
            &offer,
            &FieldAction::Move(Axis::Rows, "country".into(), true),
        );
        assert_eq!(
            moved.rows,
            vec!["customer".to_owned(), "country".to_owned()]
        );
        assert!(
            moved.collapsed.is_empty(),
            "the paths would name other groups"
        );
        let edge = apply_field(
            &view,
            &offer,
            &FieldAction::Move(Axis::Rows, "country".into(), false),
        );
        assert_eq!(edge.rows, view.rows, "nothing before the first");

        let removed = apply_field(
            &view,
            &offer,
            &FieldAction::Remove(Axis::Rows, "customer".into()),
        );
        assert!(removed.sort.is_empty(), "its sort went with it");
        let no_measure = apply_field(
            &view,
            &offer,
            &FieldAction::Remove(Axis::Values, "n".into()),
        );
        assert!(
            no_measure.sort.is_empty(),
            "a sort by a measure that went, went"
        );
    }

    /// The toolbar: a group per axis, a chip per field with its buttons, and
    /// an add button that says why when it cannot add.
    #[test]
    fn the_toolbar_says_what_it_can_do() {
        let view = PivotView::from_attributes(
            Some("country,customer"),
            Some("ordered_year"),
            Some(r#"[{"fn":"count","as":"n"}]"#),
            None,
            None,
        );
        let offer = Offer::from_attributes(Some("country,customer,ordered_year"), None).unwrap();
        let mut nodes = NodeAllocator::new();
        let mut buffer = PatchBuffer::new();
        build_toolbar(
            &mut buffer,
            &mut nodes,
            NodeId::ROOT,
            &view,
            &offer,
            &GridTexts::default(),
            &PlainLook,
        );
        let markup = format!("{:?}", buffer.patches());
        assert_eq!(markup.matches("value: \"field-group\"").count(), 3);
        assert!(markup.contains("Move country later") && markup.contains("Move customer earlier"));
        assert!(
            !markup.contains("Move country earlier"),
            "nothing before the first"
        );
        assert!(markup.contains("Remove ordered_year"));
        // A column is there (one in V1), and every field is in use.
        assert!(markup.contains("One column field already"));
        assert_eq!(markup.matches("Nothing left to add").count(), 2);
    }

    /// A view that does not hold is refused whole, with every reason.
    #[test]
    fn a_view_that_does_not_hold_is_refused_whole() {
        let read = |text: &str| PivotView::from_json(&opengrid_json::from_str(text).unwrap());
        assert!(read("[]").is_err());
        let problems = read(r#"{"rows":"country","values":[{"fn":"sum"}]}"#).unwrap_err();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(read(r#"{"rows":["country",""]}"#).is_err(), "an empty name");
        assert!(read(r#"{"columns":[1]}"#).is_err());
        assert!(read(r#"{"values":[{"field":1,"fn":"sum","as":"n"}]}"#).is_err());
        // Left out is empty; an unknown key is a later version's.
        let view = read(r#"{"rows":["country"],"collapsed":[]}"#).unwrap();
        assert!(view.columns.is_empty() && view.values.is_empty());
        assert!(read(r#"{"values":[{"fn":"count","as":"n"}]}"#).is_ok());
    }

    /// Without a model there is a status line and an empty table — the state
    /// before the first answer, not an error.
    #[test]
    fn the_skeleton_is_a_status_line_and_an_empty_table() {
        let markup = markup(None);
        assert!(markup.contains("status"));
        assert!(markup.contains("table"));
        assert!(!markup.contains("tbody"));
    }
}
